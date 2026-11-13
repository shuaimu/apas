import { create } from "zustand";
import type { MobileBootstrapResponse, WorkerDetail, WorkerEvent, WorkerListResponse, ServerToWeb, WebToServer } from "@apas/protocol";
import { ApiError, bootstrap } from "@/api/client";
import { queryClient } from "@/query/client";
import { authorizeJobStorageProject, jobCachedProjects, readJobRecord, writeJobRecord } from "@/jobs/storage";
import { abortWorkerRequests, workersApi } from "./api";
import { acceptWorkerPage, workerSnapshot } from "./continuity";
import { purgeJobAccess } from "@/jobs/runtime";

export interface WorkerScope { account: string; project: string; worker: string }
export const workerStorageScope = (scope: WorkerScope) => ({ account: scope.account, project: scope.project, job: `worker:${scope.worker}` });
interface Snapshot { detail: WorkerDetail; events: WorkerEvent[]; cursor: number; ready: boolean }
interface State { account: string | null; online: boolean; foreground: boolean; authorized: boolean; mutations: boolean; supported: boolean; revoked: Record<string, true>; projects: { id: string; name: string }[]; inbox: WorkerListResponse; entries: Record<string, Snapshot> }
const empty: State = { account: null, online: false, foreground: true, authorized: false, mutations: false, supported: false, revoked: {}, projects: [], inbox: { workers: [], next_cursor: null }, entries: {} };
export const useWorkers = create<State>(() => ({ ...empty }));
let generation = 0;
let active: { worker: string; project?: string } | null = null;
let timer: ReturnType<typeof setInterval> | null = null;
const syncing = new Map<string, Promise<void>>();
let refreshing = false;
let workerSocket: ((message: WebToServer) => boolean) | null = null;
let subscribed: string | null = null;
export function bindWorkerSocket(send: ((message: WebToServer) => boolean) | null) {
  workerSocket = send; subscribed = null;
  if (active) {
    const entry = useWorkers.getState().entries[active.worker];
    if (entry?.ready && send?.({ type: "subscribe_worker", project_id: entry.detail.worker.project_id, worker_id: active.worker, after_seq: entry.cursor })) subscribed = active.worker;
  }
}
export function handleWorkerMessage(message: ServerToWeb): boolean {
  if (message.type === "worker_inbox_changed") {
    void queryClient.invalidateQueries({ queryKey: ["workers"] });
    return true;
  }
  if (message.type === "worker_event" || message.type === "worker_snapshot") {
    const worker = message.type === "worker_event" ? message.event.worker_id : message.detail.worker.id;
    if (active?.worker === worker) void synchronizeWorker(worker, active.project).catch(() => undefined);
    return true;
  }
  if (message.type === "worker_subscription_rejected") {
    // A socket hint cannot grant authority or advance the encrypted cursor.
    invalidateWorkers();
    return true;
  }
  return false;
}
export interface WorkerAuthority { generation: number; account: string | null }
export function workerAuthority(): WorkerAuthority { const state = useWorkers.getState(); return { generation, account: state.account }; }
export function authorityCurrent(stamp: WorkerAuthority) { return stamp.generation === generation && stamp.account === useWorkers.getState().account; }
export function invalidateWorkers() {
  generation++; abortWorkerRequests();
  useWorkers.setState((state) => ({ authorized: false, entries: Object.fromEntries(Object.entries(state.entries).map(([id, entry]) => [id, { ...entry, ready: false }])) }));
}
export function setWorkerEnvironment(change: { online?: boolean; foreground?: boolean }) {
  invalidateWorkers(); useWorkers.setState(change);
}
export async function hydrateWorkers(account: string) {
  const stamp = generation;
  const scope = workerStorageScope({ account, project: "", worker: "" });
  const inbox = await readJobRecord<WorkerListResponse>(scope, "inbox");
  if (stamp === generation) useWorkers.setState({ account, inbox: inbox ?? empty.inbox });
}
export async function acceptWorkersBootstrap(value: MobileBootstrapResponse) {
  if (useWorkers.getState().account && useWorkers.getState().account !== value.user_id) {
    const { online, foreground } = useWorkers.getState();
    invalidateWorkers(); useWorkers.setState({ ...empty, online, foreground });
  }
  const stamp = generation;
  const projects = Array.from(new Map(value.machines.flatMap((machine) => machine.projects.map((project) => [project.project_id, { id: project.project_id, name: project.name ?? project.project_id }] as const))).values());
  const info = value.autonomous_workers;
  useWorkers.setState({ account: value.user_id, projects, supported: Boolean(info?.enabled), mutations: Boolean(value.features.coding_mutations), authorized: false });
  // Every encrypted project, including later pages and evidence-only caches, is reauthorized.
  const inaccessible: string[] = [];
  const projectsToAuthorize = info?.enabled ? new Set([...await jobCachedProjects(value.user_id), ...(info.workers ?? []).map((worker) => worker.project_id)]) : new Set<string>();
  for (const project of projectsToAuthorize) {
    try {
      await workersApi.list(project);
      if (stamp !== generation) return;
      authorizeJobStorageProject(value.user_id, project);
      useWorkers.setState((state) => ({ revoked: Object.fromEntries(Object.entries(state.revoked).filter(([id]) => id !== project)) }));
    }
    catch (error) {
      if (stamp !== generation) return;
      if (error instanceof ApiError && [403, 404, 410].includes(error.status)) inaccessible.push(project);
      else if (error instanceof ApiError && error.status === 401) { await purgeWorkerAccess(); return; }
    }
  }
  if (inaccessible.length) {
    for (const project of inaccessible) await purgeWorkerAccess(project);
    return; // Never install the pre-revocation bootstrap after any purge barrier.
  }
  if (stamp !== generation) return;
  const inbox = { workers: info?.workers ?? [], next_cursor: info?.next_cursor ?? null };
  for (const project of new Set(inbox.workers.map((worker) => worker.project_id))) {
    await writeJobRecord(workerStorageScope({ account: value.user_id, project, worker: "" }), "inventory-project", true);
    if (stamp !== generation) return;
  }
  await writeJobRecord(workerStorageScope({ account: value.user_id, project: "", worker: "" }), "inbox", inbox);
  if (stamp === generation) useWorkers.setState({ inbox, authorized: Boolean(info?.enabled) });
}
export async function purgeWorkerAccess(project?: string, storageAlreadyPurged = false) {
  if (!storageAlreadyPurged) {
    // One shared account/project purge owns both finite history and worker data.
    await purgeJobAccess(project);
    return;
  }
  const account = useWorkers.getState().account;
  invalidateWorkers();
  useWorkers.setState((state) => ({ revoked: project ? { ...state.revoked, [project]: true } : {} }));
  await queryClient.cancelQueries({ queryKey: ["workers"] }); queryClient.removeQueries({ queryKey: ["workers"] });
  useWorkers.setState((state) => ({ entries: project ? Object.fromEntries(Object.entries(state.entries).filter(([, entry]) => entry.detail.worker.project_id !== project)) : {}, inbox: { workers: project ? state.inbox.workers.filter((worker) => worker.project_id !== project) : [], next_cursor: null }, projects: project ? state.projects.filter((value) => value.id !== project) : [], ...(!project ? { account: null } : {}) }));
  if (account && project) await writeJobRecord(workerStorageScope({ account, project: "", worker: "" }), "inbox", useWorkers.getState().inbox);
}
export async function handleWorkerError(error: unknown, project?: string) {
  if (error instanceof ApiError && [401, 403, 404, 410].includes(error.status)) await purgeWorkerAccess(error.status === 401 ? undefined : project);
}
export function workerControlsReady(worker: string) { const state = useWorkers.getState(); return Boolean(state.account && state.online && state.foreground && state.authorized && state.mutations && state.entries[worker]?.ready); }
export function synchronizeWorker(worker: string, project?: string): Promise<void> {
  const key = `${generation}:${worker}`;
  const pending = syncing.get(key); if (pending) return pending;
  const work = (async () => {
    const state = useWorkers.getState(); const stamp = workerAuthority();
    if (!state.account || !state.authorized || !state.online || !state.foreground) return;
    useWorkers.setState((current) => ({ entries: current.entries[worker] ? { ...current.entries, [worker]: { ...current.entries[worker], ready: false } } : current.entries }));
    try {
      let detail = project ? await workersApi.detail(project, worker) : await workersApi.resolve(worker);
      if (!authorityCurrent(stamp)) return;
      if (detail.worker.id !== worker || (project && detail.worker.project_id !== project)) throw new Error("Worker identity mismatch");
      const scope = workerStorageScope({ account: state.account, project: detail.worker.project_id, worker });
      authorizeJobStorageProject(state.account, scope.project);
      const prior = await readJobRecord<Snapshot>(scope, "snapshot");
      if (!authorityCurrent(stamp)) return;
      let cursor = prior?.cursor ?? detail.worker.high_watermark;
      let events: WorkerEvent[] = prior?.events ?? [];
      // Detail is an authorized atomic baseline; replay closes changes racing that read.
      for (let pageCount = 0; pageCount < 100; pageCount++) {
        const page = await workersApi.events(scope.project, worker, cursor);
        if (!authorityCurrent(stamp)) return;
        if (page.resync_required) { detail = await workersApi.detail(scope.project, worker); if (!authorityCurrent(stamp)) return; cursor = detail.worker.high_watermark; events = []; continue; }
        cursor = acceptWorkerPage(scope.project, worker, cursor, page); events = [...events, ...page.events].slice(-200);
        if (page.has_more) continue;
        detail = await workersApi.detail(scope.project, worker);
        if (!authorityCurrent(stamp)) return;
        if (detail.worker.high_watermark > cursor) continue;
        const snapshot = workerSnapshot(detail, events, cursor);
        await writeJobRecord(scope, "snapshot", snapshot);
        if (authorityCurrent(stamp)) useWorkers.setState((current) => ({ entries: { ...current.entries, [worker]: { ...snapshot, ready: true } } }));
        if (active?.worker === worker && subscribed !== worker && authorityCurrent(stamp) && workerSocket?.({ type: "subscribe_worker", project_id: scope.project, worker_id: worker, after_seq: cursor })) subscribed = worker;
        return;
      }
      throw new Error("Worker catch-up remains incomplete; refresh to continue");
    } catch (error) { if (authorityCurrent(stamp)) await handleWorkerError(error, project ?? state.entries[worker]?.detail.worker.project_id); throw error; }
  })().finally(() => syncing.delete(key));
  syncing.set(key, work); return work;
}
export async function selectWorker(worker: string, project?: string) {
  active = { worker, project }; const stamp = workerAuthority();
  const state = useWorkers.getState();
  const knownProject = project ?? state.inbox.workers.find((value) => value.id === worker)?.project_id;
  if (state.account && knownProject) {
    const cached = await readJobRecord<Snapshot>(workerStorageScope({ account: state.account, project: knownProject, worker }), "snapshot");
    if (cached && authorityCurrent(stamp)) useWorkers.setState((current) => ({ entries: { ...current.entries, [worker]: { ...cached, ready: false } } }));
  }
  await synchronizeWorker(worker, project);
}
export function unselectWorker(worker: string) {
  if (active?.worker === worker) {
    workerSocket?.({ type: "unsubscribe_worker", worker_id: worker });
    active = null; subscribed = null;
  }
}
export function startWorkersRuntime() {
  timer ??= setInterval(() => {
    const state = useWorkers.getState();
    if (refreshing || !state.account || !state.online || !state.foreground) return;
    refreshing = true;
    const stamp = workerAuthority();
    void (async () => {
      if (!state.authorized) {
        const value = await bootstrap();
        if (!authorityCurrent(stamp)) return;
        await acceptWorkersBootstrap(value);
      }
      if (active) await synchronizeWorker(active.worker, active.project);
      if (useWorkers.getState().authorized) {
        const current = workerAuthority();
        const inbox = await workersApi.list();
        if (!authorityCurrent(current) || !current.account) return;
        for (const project of new Set(inbox.workers.map((worker) => worker.project_id))) {
          await writeJobRecord(workerStorageScope({ account: current.account, project, worker: "" }), "inventory-project", true);
          if (!authorityCurrent(current)) return;
        }
        await writeJobRecord(workerStorageScope({ account: current.account, project: "", worker: "" }), "inbox", inbox);
        if (authorityCurrent(current)) useWorkers.setState({ inbox });
      }
    })().catch(async (error) => { if (authorityCurrent(stamp)) { await handleWorkerError(error); invalidateWorkers(); } }).finally(() => { refreshing = false; });
  }, 10_000);
}
export async function stopWorkersRuntime() {
  workerSocket = null; subscribed = null;
  invalidateWorkers(); clearInterval(timer ?? undefined); timer = null; active = null; useWorkers.setState({ ...empty });
  await Promise.allSettled(syncing.values());
}
