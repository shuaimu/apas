import { create } from "zustand";
import type { JobListResponse, MobileBootstrapResponse } from "@apas/protocol";
import { ApiError, bootstrap } from "@/api/client";
import { queryClient } from "@/query/client";
import { loadCredentials } from "@/security/credentials";
import { abortJobRequests, jobsApi } from "./api";
import { authorizeJobStorageProject, setJobStorageAccount, commitJobSnapshot, drainJobStorage, jobCachedProjects, purgeJobStorage, readJobRecord, writeJobRecord } from "./storage";
import type { JobSnapshot } from "./storage";
import { acceptWorkersBootstrap, hydrateWorkers, invalidateWorkers, purgeWorkerAccess, setWorkerEnvironment, startWorkersRuntime, stopWorkersRuntime } from "@/workers/runtime";

interface JobState {
  account: string | null;
  online: boolean;
  foreground: boolean;
  authorized: boolean;
  mutations: boolean;
  unavailable: string[];
  inbox: JobListResponse;
  projects: { id: string; name: string }[];
  entries: Record<string, JobSnapshot & { ready: boolean; error: string | null }>;
}
const empty: JobState = { account: null, online: false, foreground: true, authorized: false, mutations: false, unavailable: [], inbox: { jobs: [], next_cursor: null }, entries: {}, projects: [] };
export const useJobs = create<JobState>(() => ({ ...empty }));
let epoch = 0;
let lifecycle = 0;
const synchronizing = new Map<string, Promise<void>>();
let timer: ReturnType<typeof setInterval> | null = null;
let active: { job: string; project?: string } | null = null;
let running: Promise<void> | null = null;
let accountQueue: Promise<void> = Promise.resolve();

export function invalidateJobs(): void {
  epoch++;
  invalidateWorkers();
  useJobs.setState((state) => ({ authorized: false, entries: Object.fromEntries(Object.entries(state.entries).map(([id, value]) => [id, { ...value, ready: false }])) }));
}

export async function hydrateJobs(): Promise<void> {
  const credentials = await loadCredentials();
  if (!credentials) return;
  const owner = await readJobRecord<{ account: string; device: string }>({ account: "", project: "", job: "" }, "owner");
  if (!owner || owner.device !== credentials.deviceSessionId) { await purgeJobStorage(); return; }
  setJobStorageAccount(owner.account);
  const inbox = await readJobRecord<JobListResponse>({ account: owner.account, project: "", job: "" }, "inbox");
  const projects = await readJobRecord<JobState["projects"]>({ account: owner.account, project: "", job: "" }, "projects");
  useJobs.setState({ account: owner.account, inbox: inbox ?? empty.inbox, projects: projects ?? [] });
  await hydrateWorkers(owner.account);
}

export function acceptJobsBootstrap(value: MobileBootstrapResponse): Promise<void> {
  const lifecycleAtStart = lifecycle;
  const work = async () => {
    if (lifecycleAtStart !== lifecycle) return;
    const previous = useJobs.getState().account;
    if (previous && previous !== value.user_id) {
      const { online, foreground } = useJobs.getState();
      invalidateJobs(); queryClient.clear(); await purgeJobStorage();
      if (lifecycleAtStart !== lifecycle) return;
      useJobs.setState({ ...empty, online, foreground });
    }
    const credentials = await loadCredentials();
    if (!credentials || lifecycleAtStart !== lifecycle) return;
    setJobStorageAccount(value.user_id);
    const info = value.autonomous_jobs;
    const inbox = { jobs: info?.jobs ?? [], next_cursor: info?.next_cursor ?? null };
    await writeJobRecord({ account: "", project: "", job: "" }, "owner", { account: value.user_id, device: credentials.deviceSessionId });
    await writeJobRecord({ account: value.user_id, project: "", job: "" }, "inbox", inbox);
    if (lifecycleAtStart !== lifecycle) return;
    const projects = Array.from(new Map(value.machines.flatMap((machine) => machine.projects.map((project) => [project.project_id, { id: project.project_id, name: project.name ?? project.project_id }] as const))).values());
    await writeJobRecord({ account: value.user_id, project: "", job: "" }, "projects", projects);
    if (lifecycleAtStart !== lifecycle) return;
    useJobs.setState({ account: value.user_id, mutations: Boolean(value.features.coding_mutations), unavailable: info?.unavailable_reasons ?? ["Autonomous jobs are not available on this server"], inbox, projects });
    // A paginated inbox is not an authorization inventory. Recheck every cached project.
    const authorizationEpoch = epoch;
    for (const project of await jobCachedProjects(value.user_id)) {
      try { await jobsApi.targets(project); authorizeJobStorageProject(value.user_id, project); }
      catch (error) {
        if (error instanceof ApiError && [403, 404, 410].includes(error.status)) await purgeJobAccess(project);
        else if (error instanceof ApiError && error.status === 401) throw error;
      }
    }
    if (lifecycleAtStart === lifecycle) useJobs.setState({ authorized: Boolean(info?.available) });
    if (lifecycleAtStart === lifecycle && authorizationEpoch === epoch) await acceptWorkersBootstrap(value);
  };
  accountQueue = accountQueue.then(work, work);
  return accountQueue;
}

export async function purgeJobAccess(project?: string): Promise<void> {
  const account = useJobs.getState().account;
  if (!project) lifecycle++;
  abortJobRequests();
  running = null;
  invalidateJobs();
  await queryClient.cancelQueries({ queryKey: ["jobs"] });
  queryClient.removeQueries({ queryKey: ["jobs"] });
  useJobs.setState((state) => ({
    entries: project ? Object.fromEntries(Object.entries(state.entries).filter(([, value]) => value.detail.job.project_id !== project)) : {},
    inbox: { ...state.inbox, jobs: project ? state.inbox.jobs.filter((job) => job.project_id !== project) : [] },
    ...(!project ? { account: null } : {}),
  }));
  await purgeJobStorage(account ?? undefined, project);
  await purgeWorkerAccess(project, true);
  if (account && project) {
    await writeJobRecord({ account, project: "", job: "" }, "inbox", useJobs.getState().inbox);
    const selection = await readJobRecord<{ project: string }>({ account, project: "", job: "" }, "composer-project");
    if (selection?.project === project) await writeJobRecord({ account, project: "", job: "" }, "composer-project", { project: "" });
    useJobs.setState((state) => ({ projects: state.projects.filter((item) => item.id !== project) }));
    await writeJobRecord({ account, project: "", job: "" }, "projects", useJobs.getState().projects);
  }
}

async function performJobSync(job: string, project?: string): Promise<void> {
  const before = useJobs.getState();
  if (!before.account || !before.authorized || !before.online || !before.foreground) return;
  const generation = epoch;
  const current = () => generation === epoch && useJobs.getState().account === before.account;
  useJobs.setState((state) => ({ entries: state.entries[job] ? { ...state.entries, [job]: { ...state.entries[job], ready: false } } : state.entries }));
  try {
    let detail = project ? await jobsApi.detail(project, job) : await jobsApi.resolve(job);
    if (!current()) return;
    authorizeJobStorageProject(before.account, detail.job.project_id);
    const scope = { account: before.account, project: detail.job.project_id, job };
    // A newly authorized snapshot is the baseline; catch up changes that raced it.
    let snapshot = await commitJobSnapshot(scope, detail);
    for (let pages = 0; pages < 100; pages++) {
      const page = await jobsApi.events(scope.project, job, snapshot.cursor);
      if (!current()) return;
      if (page.resync_required) {
        detail = await jobsApi.detail(scope.project, job);
        if (!current()) return;
        snapshot = await commitJobSnapshot(scope, detail, undefined, true);
        continue;
      }
      snapshot = await commitJobSnapshot(scope, detail, page);
      if (page.has_more || snapshot.cursor < page.high_watermark) continue;
      const latest = await jobsApi.detail(scope.project, job);
      if (!current()) return;
      if (latest.high_watermark > snapshot.cursor) { detail = latest; continue; }
      if (latest.high_watermark !== snapshot.cursor) throw new Error("Job snapshot moved backwards; resynchronize");
      snapshot = await commitJobSnapshot(scope, latest);
      if (current()) useJobs.setState((state) => ({ entries: { ...state.entries, [job]: { ...snapshot, ready: true, error: null } } }));
      return;
    }
    throw new Error("Job catch-up is still in progress; refresh to continue");
  } catch (error) {
    if (!current()) return;
    if (error instanceof ApiError && [401, 403, 404, 410].includes(error.status)) {
      const inaccessibleProject = project ?? before.entries[job]?.detail.job.project_id ?? before.inbox.jobs.find((item) => item.id === job)?.project_id;
      if (error.status === 401) await purgeJobAccess();
      else if (inaccessibleProject) await purgeJobAccess(inaccessibleProject);
      throw new Error("This job is no longer accessible. Return to Jobs for current access.");
    }
    useJobs.setState((state) => ({ entries: state.entries[job] ? { ...state.entries, [job]: { ...state.entries[job], ready: false, error: String(error) } } : state.entries }));
    throw error;
  }
}

export function synchronizeJob(job: string, project?: string): Promise<void> {
  const key = `${epoch}:${job}`;
  const existing = synchronizing.get(key);
  if (existing) return existing;
  const work = performJobSync(job, project).finally(() => { synchronizing.delete(key); });
  synchronizing.set(key, work);
  return work;
}

export async function selectJob(job: string, project?: string): Promise<void> {
  active = { job, project };
  const account = useJobs.getState().account;
  const generation = epoch;
  if (account && project) {
    const cached = await readJobRecord<JobSnapshot>({ account, project, job }, "snapshot");
    if (cached && generation === epoch && useJobs.getState().account === account) useJobs.setState((state) => ({ entries: { ...state.entries, [job]: { ...cached, ready: false, error: null } } }));
  }
  await synchronizeJob(job, project);
}
export function unselectJob(job: string): void { if (active?.job === job) active = null; }

async function refreshJobs(): Promise<void> {
  if (running) return running;
  if (!useJobs.getState().online || !useJobs.getState().foreground) return;
  const observedLifecycle = lifecycle;
  running = (async () => {
    try {
      if (!useJobs.getState().authorized) {
        const value = await bootstrap();
        if (observedLifecycle !== lifecycle) return;
        await acceptJobsBootstrap(value);
      }
      if (observedLifecycle !== lifecycle) return;
      if (active) await synchronizeJob(active.job, active.project);
      else if (useJobs.getState().authorized) {
        const generation = epoch;
        const inbox = await jobsApi.list();
        const account = useJobs.getState().account;
        if (generation !== epoch || !account) return;
        await writeJobRecord({ account, project: "", job: "" }, "inbox", inbox);
        if (generation === epoch) useJobs.setState({ inbox });
      }
    } catch (error) {
      if (observedLifecycle !== lifecycle) return;
      if (error instanceof ApiError && error.status === 401) await purgeJobAccess();
      else invalidateJobs();
    }
  })().finally(() => { if (observedLifecycle === lifecycle) running = null; });
  return running;
}

export function setJobEnvironment(change: { online?: boolean; foreground?: boolean }): void {
  const state = useJobs.getState();
  if ((change.online === undefined || change.online === state.online) && (change.foreground === undefined || change.foreground === state.foreground)) return;
  lifecycle++;
  abortJobRequests();
  running = null;
  invalidateJobs();
  useJobs.setState(change);
  setWorkerEnvironment(change);
  void refreshJobs();
}
export function startJobsRuntime(): void {
  startWorkersRuntime();
  timer ??= setInterval(() => { void refreshJobs(); }, 10_000);
  void refreshJobs();
}
export async function stopJobsRuntime(): Promise<void> {
  const workersStopped = stopWorkersRuntime();
  lifecycle++;
  abortJobRequests();
  running = null;
  setJobStorageAccount(null);
  clearInterval(timer ?? undefined);
  timer = null;
  active = null;
  invalidateJobs();
  useJobs.setState({ ...empty });
  await accountQueue.catch(() => undefined);
  await Promise.allSettled(synchronizing.values());
  await workersStopped;
  useJobs.setState({ ...empty });
  await drainJobStorage();
}

export function jobControlsReady(job: string): boolean {
  const state = useJobs.getState();
  return Boolean(state.account && state.online && state.foreground && state.authorized && state.mutations && state.entries[job]?.ready);
}
