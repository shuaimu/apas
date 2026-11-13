import { validateWorkerContract, validateJobContract } from "@apas/protocol";
import type { WorkerContractKind, JobContractKind, WorkerListResponse, WorkerTargetsResponse, WorkerPreviewRequest, WorkerPreview, WorkerApprovalRequest, WorkerMutationReceipt, WorkerMutationRequest, WorkerDetail, WorkerEventsResponse, WorkerItemsResponse, WorkerItemRequest, WorkerRunsResponse, JobDetail, JobReview, JobArtifactContent, JobAnswerRequest, JobMutationRequest, JobMutationReceipt } from "@apas/protocol";
import { ApiError, validAccessToken } from "@/api/client";
import { endpoints } from "@/config/endpoints";

const segment = encodeURIComponent;
export const workerPath = (project: string, worker: string) => `/projects/${segment(project)}/workers/${segment(worker)}`;
const requests = new Set<AbortController>();
export function abortWorkerRequests() { for (const request of requests) request.abort(); requests.clear(); }
async function request<T>(path: string, kind: WorkerContractKind | JobContractKind, body?: unknown, signal?: AbortSignal, execution = false): Promise<T> {
  const controller = new AbortController(); requests.add(controller);
  const abort = () => controller.abort(); signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) abort();
  const timeout = setTimeout(abort, 30_000);
  try {
    let token = await validAccessToken();
    const send = () => fetch(`${endpoints.apiUrl}${path}`, { method: body === undefined ? "GET" : "POST", signal: controller.signal, headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
    let response = await send();
    if (response.status === 401 && body === undefined) { token = await validAccessToken(true); response = await send(); }
    const value: unknown = await response.json();
    if (!response.ok) { const error = value as { code?: string; message?: string; error?: string }; throw new ApiError(`${error.code ?? "Worker request unavailable"}: ${error.message ?? error.error ?? response.status}`, response.status); }
    const validation = execution ? validateJobContract(kind as JobContractKind, value) : validateWorkerContract(kind as WorkerContractKind, value);
    if (!validation.valid) throw new Error("Incompatible worker response; upgrade required");
    return value as T;
  } finally { clearTimeout(timeout); signal?.removeEventListener("abort", abort); requests.delete(controller); }
}
const page = (cursor?: string | null, status?: string) => `?limit=40${cursor ? `&cursor=${segment(cursor)}` : ""}${status ? `&status=${segment(status)}` : ""}`;
const runPath = (project: string, worker: string, run: string) => `${workerPath(project, worker)}/runs/${segment(run)}`;
export const workersApi = {
  list: (project?: string, cursor?: string | null, status?: string, signal?: AbortSignal) => request<WorkerListResponse>(`${project ? `/projects/${segment(project)}/workers` : "/projects/workers"}${page(cursor, status)}`, "list", undefined, signal),
  targets: (project: string, signal?: AbortSignal) => request<WorkerTargetsResponse>(`/projects/${segment(project)}/worker-targets`, "targets", undefined, signal),
  resolve: (worker: string, signal?: AbortSignal) => request<WorkerDetail>(`/projects/workers/${segment(worker)}`, "detail", undefined, signal),
  detail: (project: string, worker: string, signal?: AbortSignal) => request<WorkerDetail>(workerPath(project, worker), "detail", undefined, signal),
  preview: (project: string, body: WorkerPreviewRequest) => request<WorkerPreview>(`/projects/${segment(project)}/worker-previews`, "preview", body),
  approve: (project: string, worker: string, body: WorkerApprovalRequest) => request<WorkerMutationReceipt>(worker ? `${workerPath(project, worker)}/mandate` : `/projects/${segment(project)}/workers`, "receipt", body),
  mutate: (project: string, worker: string, action: "pause" | "stop" | "resume" | "wake", body: WorkerMutationRequest) => request<WorkerMutationReceipt>(`${workerPath(project, worker)}/${action}`, "receipt", body),
  receipt: (id: string) => request<WorkerMutationReceipt>(`/projects/worker-requests/${segment(id)}`, "receipt"),
  events: (project: string, worker: string, after: number) => request<WorkerEventsResponse>(`${workerPath(project, worker)}/events?after_seq=${after}&limit=100`, "events"),
  items: (project: string, worker: string, cursor?: string | null, signal?: AbortSignal) => request<WorkerItemsResponse>(`${workerPath(project, worker)}/items${page(cursor)}`, "items", undefined, signal),
  item: (project: string, worker: string, body: WorkerItemRequest) => request<WorkerMutationReceipt>(`${workerPath(project, worker)}/items`, "receipt", body),
  itemAction: (project: string, worker: string, item: string, action: "dismiss" | "retry", body: WorkerMutationRequest) => request<WorkerMutationReceipt>(`${workerPath(project, worker)}/items/${segment(item)}/${action}`, "receipt", body),
  runs: (project: string, worker: string, cursor?: string | null, signal?: AbortSignal) => request<WorkerRunsResponse>(`${workerPath(project, worker)}/runs${page(cursor)}`, "runs", undefined, signal),
  run: (project: string, worker: string, run: string, signal?: AbortSignal) => request<JobDetail>(runPath(project, worker, run), "detail", undefined, signal, true),
  review: (project: string, worker: string, run: string, cursor?: string | null, signal?: AbortSignal) => request<JobReview>(`${runPath(project, worker, run)}/review${page(cursor)}`, "review", undefined, signal, true),
  artifact: (project: string, worker: string, run: string, artifact: string, offset?: string | null, signal?: AbortSignal) => request<JobArtifactContent>(`${runPath(project, worker, run)}/artifacts/${segment(artifact)}?limit=32768${offset ? `&offset=${segment(offset)}` : ""}`, "artifact", undefined, signal, true),
  answer: (project: string, worker: string, run: string, question: string, body: JobAnswerRequest) => request<JobMutationReceipt>(`${runPath(project, worker, run)}/questions/${segment(question)}/answers`, "receipt", body, undefined, true),
  retryPublication: (project: string, worker: string, run: string, body: JobMutationRequest) => request<JobMutationReceipt>(`${runPath(project, worker, run)}/retry-publication`, "receipt", body, undefined, true),
  executionReceipt: (id: string) => request<JobMutationReceipt>(`/projects/job-requests/${segment(id)}`, "receipt", undefined, undefined, true),
};
