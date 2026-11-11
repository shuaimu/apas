import { validateJobContract } from "@apas/protocol";
import type { JobContractKind, JobApiError, JobDetail, JobEventsResponse, JobMutationReceipt, ServerToWeb } from "@apas/protocol";
import { useStore } from "./store";
import { commitJobEvents, jobCacheRevision, purgeJobCache, readJobRecord, writeJobRecord } from "./jobCache";
import type { JobScope, SavedJob, SavedJobRequest } from "./jobCache";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
export class JobHttpError extends Error {
  constructor(public status: number, public code: string, message: string) { super(message); }
}
export const jobPath = (project: string, job?: string) => `/projects/${encodeURIComponent(project)}/jobs${job ? `/${encodeURIComponent(job)}` : ""}`;
export const jobHref = (project: string, job: string, suffix = "") => `/code/jobs/${encodeURIComponent(job)}${suffix}?project=${encodeURIComponent(project)}`;
export async function jobFetch<T>(path: string, kind: JobContractKind, body?: unknown): Promise<T> {
  const { token, userId } = useStore.getState();
  const revision = jobCacheRevision();
  if (!token || !userId) throw new JobHttpError(401, "authentication_required", "Sign in to access jobs.");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 45_000);
  try {
    const response = await fetch(`${API_URL}${path}`, {
      method: body === undefined ? "GET" : "POST", cache: "no-store", signal: controller.signal,
      headers: { Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const value = await response.json();
    if (!response.ok) {
      const error = validateJobContract("error", value).valid ? value as JobApiError : undefined;
      if (response.status === 401 || error?.code === "authentication_required") {
        try { await purgeJobCache(userId); } finally { useStore.getState().logout(); }
      } else if (response.status === 403) {
        const match = path.match(/^\/projects\/([^/]+)\//);
        await purgeJobCache(userId, match && !["jobs", "job-requests"].includes(match[1]) ? decodeURIComponent(match[1]) : undefined);
      }
      throw new JobHttpError(response.status, error?.code ?? "request_failed", error?.error ?? `Job request failed (${response.status}).`);
    }
    if (useStore.getState().userId !== userId || jobCacheRevision() !== revision) throw new Error("Job access changed while loading.");
    if (!validateJobContract(kind, value).valid) throw new Error("The server returned an unsupported job response. Update the client before continuing.");
    return value as T;
  } finally { clearTimeout(timer); }
}
export async function resolveJobProject(id: string, hint?: string | null): Promise<string> {
  if (hint) return hint;
  const detail = await jobFetch<JobDetail>(`/projects/jobs/${encodeURIComponent(id)}`, "detail");
  return detail.job.project_id;
}
const synchronizations = new Map<string, Promise<SavedJob>>();
export function synchronizeJob(scope: JobScope): Promise<SavedJob> {
  const key = JSON.stringify([jobCacheRevision(), ...scope]);
  const existing = synchronizations.get(key);
  if (existing) return existing;
  const operation = replayJob(scope).finally(() => synchronizations.delete(key));
  synchronizations.set(key, operation);
  return operation;
}
async function replayJob(scope: JobScope): Promise<SavedJob> {
  const revision = jobCacheRevision();
  if (scope[0] !== useStore.getState().userId) throw new Error("The job belongs to a different account.");
  let detail = await jobFetch<JobDetail>(jobPath(scope[1], scope[2]), "detail");
  let saved = await readJobRecord<SavedJob>(scope, "snapshot");
  let resynced = false;
  for (;;) {
    const page = await jobFetch<JobEventsResponse>(`${jobPath(scope[1], scope[2])}/events?after_seq=${saved?.cursor ?? 0}&limit=100`, "events");
    if (page.resync_required) {
      if (resynced) throw new Error("Job history changed during synchronization. Refresh again.");
      detail = await jobFetch<JobDetail>(jobPath(scope[1], scope[2]), "detail");
      saved = { detail, events: [], cursor: detail.high_watermark, historyResynced: true };
      await writeJobRecord(scope, "snapshot", saved, "", revision);
      resynced = true;
      continue;
    }
    saved = await commitJobEvents(scope, detail, page, revision);
    if (page.has_more || saved.cursor < page.high_watermark) continue;
    // The action state must include every event just committed, not the older initial snapshot.
    detail = await jobFetch<JobDetail>(jobPath(scope[1], scope[2]), "detail");
    if (detail.high_watermark > saved.cursor) continue;
    saved = { ...saved, detail };
    await writeJobRecord(scope, "snapshot", saved, "", revision);
    return saved;
  }
}
export async function saveJobRequest(request: SavedJobRequest): Promise<void> {
  await writeJobRecord(request.scope, "request", request, request.id);
}
export async function resolveJobRequest(request: SavedJobRequest): Promise<SavedJobRequest> {
  const revision = jobCacheRevision();
  if (request.scope[0] !== useStore.getState().userId) throw new Error("The saved request belongs to a different account.");
  const receipt = await jobFetch<JobMutationReceipt>(`/projects/job-requests/${encodeURIComponent(request.id)}`, "receipt");
  const saved = { ...request, receipt };
  await writeJobRecord(request.scope, "request", saved, request.id, revision);
  return saved;
}
export async function sendJobRequest(request: SavedJobRequest): Promise<SavedJobRequest> {
  const revision = jobCacheRevision();
  if (request.scope[0] !== useStore.getState().userId) throw new Error("The saved request belongs to a different account.");
  await saveJobRequest(request);
  if (revision !== jobCacheRevision() || request.scope[0] !== useStore.getState().userId) throw new Error("Job access changed before submission. Nothing was sent.");
  const receipt = await jobFetch<JobMutationReceipt>(request.path, "receipt", request.body);
  const saved = { ...request, receipt };
  await writeJobRecord(request.scope, "request", saved, request.id, revision);
  return saved;
}
export type JobWireMessage = Extract<ServerToWeb, { type: "job_snapshot" | "job_event" | "job_changed" | "job_subscription_rejected" }>;
const wireListeners = new Set<(message: JobWireMessage) => void>();
export function onJobMessage(listener: (message: JobWireMessage) => void) { wireListeners.add(listener); return () => { wireListeners.delete(listener); }; }
export function receiveJobMessage(message: JobWireMessage) { wireListeners.forEach((listener) => listener(message)); }
