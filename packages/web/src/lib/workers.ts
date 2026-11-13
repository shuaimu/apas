import { validateJobContract, validateServerMessage, validateWorkerContract } from "@apas/protocol";
import type { JobApiError, ServerToWeb, WorkerContractKind, WorkerDetail, WorkerEventsResponse, WorkerMutationReceipt } from "@apas/protocol";
import { useStore } from "./store";
import { commitWorkerEvents, commitWorkerSnapshot, persistWorkerRequest, purgeWorkerCache, readWorkerRecord, workerCacheRevision } from "./workerCache";
import type { SavedWorker, SavedWorkerRequest, WorkerScope } from "./workerCache";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
export class WorkerHttpError extends Error {
  constructor(public status: number, public code: string, message: string) { super(message); }
}
export const workerPath = (project: string, worker?: string) => `/projects/${encodeURIComponent(project)}/workers${worker ? `/${encodeURIComponent(worker)}` : ""}`;
export const workerHref = (project: string, worker: string, suffix = "") => `/code/workers/${encodeURIComponent(worker)}${suffix}?project=${encodeURIComponent(project)}`;

export async function workerFetch<T>(path: string, kind: WorkerContractKind, body?: unknown): Promise<T> {
  const { token, userId, accountStatus } = useStore.getState();
  const revision = workerCacheRevision();
  if (!token || !userId) throw new WorkerHttpError(401, "authentication_required", "Sign in to access workers.");
  if (accountStatus === "suspended") throw new WorkerHttpError(403, "account_suspended", "This account cannot access workers.");
  const current = () => useStore.getState().userId === userId && useStore.getState().token === token && workerCacheRevision() === revision;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 45_000);
  try {
    const response = await fetch(`${API_URL}${path}`, {
      method: body === undefined ? "GET" : "POST", cache: "no-store", signal: controller.signal,
      headers: { Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!current()) throw new Error("Worker access changed while loading.");
    const value: unknown = await response.json().catch(() => undefined);
    // A delayed denial from an old account must not log out the new account either.
    if (!current()) throw new Error("Worker access changed while loading.");
    if (!response.ok) {
      const error = validateJobContract("error", value).valid ? value as JobApiError : undefined;
      const project = path.match(/^\/projects\/([^/]+)\/(?:workers|worker-targets|worker-previews)(?:[/?]|$)/)?.[1];
      if (response.status === 401 || error?.code === "authentication_required") {
        try { await purgeWorkerCache(userId); }
        finally { if (useStore.getState().userId === userId && useStore.getState().token === token) useStore.getState().logout(); }
      } else if (response.status === 403 || (project || kind === "detail") && [404, 410].includes(response.status)) {
        await purgeWorkerCache(userId, project ? decodeURIComponent(project) : undefined);
      }
      throw new WorkerHttpError(response.status, error?.code ?? "request_failed", error?.error ?? `Worker request failed (${response.status}).`);
    }
    if (!validateWorkerContract(kind, value).valid) throw new Error("The server returned an unsupported worker response. Update the client before continuing.");
    return value as T;
  } finally { clearTimeout(timer); }
}

export async function resolveWorkerProject(id: string, hint?: string | null): Promise<string> {
  if (hint) return hint;
  const detail = await workerFetch<WorkerDetail>(`/projects/workers/${encodeURIComponent(id)}`, "detail");
  if (detail.worker.id !== id) throw new Error("The server returned a different worker.");
  return detail.worker.project_id;
}
const synchronizations = new Map<string, Promise<SavedWorker>>();
export function synchronizeWorker(scope: WorkerScope): Promise<SavedWorker> {
  const key = JSON.stringify([workerCacheRevision(), useStore.getState().token, ...scope]);
  const existing = synchronizations.get(key);
  if (existing) return existing;
  const operation = replayWorker(scope).finally(() => synchronizations.delete(key));
  synchronizations.set(key, operation);
  return operation;
}
async function replayWorker(scope: WorkerScope): Promise<SavedWorker> {
  const revision = workerCacheRevision();
  const token = useStore.getState().token;
  const assertCurrent = () => {
    if (scope[0] !== useStore.getState().userId || token !== useStore.getState().token || revision !== workerCacheRevision()) throw new Error("Worker access changed during synchronization.");
  };
  assertCurrent();
  let detail = await workerFetch<WorkerDetail>(workerPath(scope[1], scope[2]), "detail");
  assertCurrent();
  let saved = await readWorkerRecord<SavedWorker>(scope, "snapshot");
  let resynced = false;
  // Bound each catch-up operation, not just each response. A busy responsibility
  // remains read-only if it cannot reach a coherent snapshot in this pass.
  for (let count = 0; count < 32; count += 1) {
    assertCurrent();
    const page = await workerFetch<WorkerEventsResponse>(`${workerPath(scope[1], scope[2])}/events?after_seq=${saved?.cursor ?? 0}&limit=100`, "events");
    assertCurrent();
    if (page.resync_required) {
      if (resynced) throw new Error("Worker history changed during synchronization. Refresh again.");
      detail = await workerFetch<WorkerDetail>(workerPath(scope[1], scope[2]), "detail");
      assertCurrent();
      saved = await commitWorkerSnapshot(scope, detail, true, revision);
      resynced = true;
      continue;
    }
    saved = await commitWorkerEvents(scope, detail, page, revision);
    assertCurrent();
    if (page.has_more || saved.cursor < page.high_watermark) continue;
    detail = await workerFetch<WorkerDetail>(workerPath(scope[1], scope[2]), "detail");
    assertCurrent();
    if (detail.worker.high_watermark > saved.cursor) continue;
    return commitWorkerSnapshot(scope, detail, false, revision);
  }
  throw new Error("Worker catch-up reached its bounded page limit. Refresh to continue; controls remain read-only.");
}

function assertRequest(request: SavedWorkerRequest, revision: number, token: string | null) {
  if (request.scope[0] !== useStore.getState().userId || token !== useStore.getState().token || revision !== workerCacheRevision()) throw new Error("Worker request access changed. Nothing was submitted.");
  const base = workerPath(request.scope[1]);
  if (request.id !== request.body.request_id || !(request.path === base || request.path.startsWith(`${workerPath(request.scope[1], request.scope[2])}/`))) throw new Error("Worker request does not match its saved scope.");
}
function assertReceipt(request: SavedWorkerRequest, receipt: WorkerMutationReceipt) {
  if (receipt.request_id !== request.id || receipt.project_id !== request.scope[1] || (!("preview_id" in request.body) && receipt.worker_id != null && receipt.worker_id !== request.scope[2])) throw new Error("The worker receipt does not match the original request.");
}
export async function saveWorkerRequest(request: SavedWorkerRequest): Promise<void> {
  const revision = workerCacheRevision();
  const token = useStore.getState().token;
  assertRequest(request, revision, token);
  await persistWorkerRequest(request, revision);
  assertRequest(request, revision, token);
}
export async function resolveWorkerRequest(request: SavedWorkerRequest): Promise<SavedWorkerRequest> {
  const revision = workerCacheRevision();
  const token = useStore.getState().token;
  assertRequest(request, revision, token);
  const receipt = await workerFetch<WorkerMutationReceipt>(`/projects/worker-requests/${encodeURIComponent(request.id)}`, "receipt");
  assertRequest(request, revision, token);
  assertReceipt(request, receipt);
  const saved = await persistWorkerRequest({ ...request, receipt }, revision);
  assertRequest(request, revision, token);
  return saved;
}
/** Called only by an explicit user action. Recovery itself performs GETs, never POSTs. */
export async function sendWorkerRequest(request: SavedWorkerRequest): Promise<SavedWorkerRequest> {
  const revision = workerCacheRevision();
  const token = useStore.getState().token;
  assertRequest(request, revision, token);
  const previous = await readWorkerRecord<SavedWorkerRequest>(request.scope, "request", request.id);
  assertRequest(request, revision, token);
  // Persist before transport, and never overwrite a previously accepted identity.
  const pending = await persistWorkerRequest(request, revision);
  assertRequest(request, revision, token);
  if (pending.receipt && pending.receipt.status !== "pending") return pending;
  if (previous) {
    try { return await resolveWorkerRequest(pending); }
    catch (failure) { if (!(failure instanceof WorkerHttpError && failure.status === 404)) throw failure; }
  }
  assertRequest(request, revision, token);
  const receipt = await workerFetch<WorkerMutationReceipt>(request.path, "receipt", request.body);
  assertRequest(request, revision, token);
  assertReceipt(request, receipt);
  const saved = await persistWorkerRequest({ ...pending, receipt }, revision);
  assertRequest(request, revision, token);
  return saved;
}

export type WorkerWireMessage = Extract<ServerToWeb, { type: "worker_snapshot" | "worker_event" | "worker_inbox_changed" | "worker_subscription_rejected" }>;
const wireListeners = new Set<(message: WorkerWireMessage) => void>();
export function onWorkerMessage(listener: (message: WorkerWireMessage) => void) {
  wireListeners.add(listener);
  return () => { wireListeners.delete(listener); };
}
export function receiveWorkerMessage(value: unknown) {
  const { userId, token, isAuthenticated, accountStatus, negotiatedCapabilities } = useStore.getState();
  if (!userId || !token || !isAuthenticated || accountStatus === "suspended" || !negotiatedCapabilities.has("autonomous_workers_v1") || !validateServerMessage(value).valid) return;
  const message = value as WorkerWireMessage;
  if (!["worker_snapshot", "worker_event", "worker_inbox_changed", "worker_subscription_rejected"].includes(message.type)) return;
  const revision = workerCacheRevision();
  for (const listener of wireListeners) {
    if (workerCacheRevision() !== revision || useStore.getState().userId !== userId || useStore.getState().token !== token) break;
    listener(message);
  }
}
