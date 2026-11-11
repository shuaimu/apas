import { validateJobContract } from "@apas/protocol";
import type { JobContractKind, JobTargetsResponse, JobListResponse, JobDetail, JobPreviewRequest, JobPreview, JobAdmissionRequest, JobMutationRequest, JobAnswerRequest, JobMutationReceipt, JobEventsResponse, JobReview, JobArtifactContent } from "@apas/protocol";
import { ApiError, validAccessToken } from "@/api/client";
import { endpoints } from "@/config/endpoints";

const segment = encodeURIComponent;
export const jobPath = (project: string, job: string) => `/projects/${segment(project)}/jobs/${segment(job)}`;
const requests = new Set<AbortController>();

export function abortJobRequests(): void {
  for (const controller of requests) controller.abort();
  requests.clear();
}

async function request<T>(path: string, kind: JobContractKind, body?: unknown, signal?: AbortSignal): Promise<T> {
  const controller = new AbortController();
  requests.add(controller);
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) controller.abort();
  const timeout = setTimeout(abort, 30_000);
  try {
    let token = await validAccessToken();
    const send = () => fetch(`${endpoints.apiUrl}${path}`, {
      method: body === undefined ? "GET" : "POST", signal: controller.signal,
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    let response = await send();
    // Never replay a mutation, including after authentication expiry.
    if (response.status === 401 && body === undefined) { token = await validAccessToken(true); response = await send(); }
    const value: unknown = await response.json();
    if (!response.ok) {
      const error = value as { message?: string; error?: string; code?: string };
      throw new ApiError(`${error.code ? `${error.code}: ` : ""}${error.message ?? error.error ?? "Job request unavailable"}`, response.status);
    }
    const validation = validateJobContract(kind, value);
    if (!validation.valid) throw new Error("Server returned an incompatible job response");
    return value as T;
  } finally {
    clearTimeout(timeout);
    signal?.removeEventListener("abort", abort);
    requests.delete(controller);
  }
}

export const jobsApi = {
  list: (cursor?: string | null, attention = false, signal?: AbortSignal) => request<JobListResponse>(`/projects/jobs?limit=40${cursor ? `&cursor=${segment(cursor)}` : ""}${attention ? "&status=attention" : ""}`, "list", undefined, signal),
  targets: (project: string, signal?: AbortSignal) => request<JobTargetsResponse>(`/projects/${segment(project)}/job-targets`, "targets", undefined, signal),
  preview: (project: string, body: JobPreviewRequest) => request<JobPreview>(`/projects/${segment(project)}/job-previews`, "preview", body),
  detail: (project: string, job: string, signal?: AbortSignal) => request<JobDetail>(jobPath(project, job), "detail", undefined, signal),
  resolve: (job: string, signal?: AbortSignal) => request<JobDetail>(`/projects/jobs/${segment(job)}`, "detail", undefined, signal),
  events: (project: string, job: string, after: number, signal?: AbortSignal) => request<JobEventsResponse>(`${jobPath(project, job)}/events?after_seq=${after}&limit=100`, "events", undefined, signal),
  receipt: (id: string, signal?: AbortSignal) => request<JobMutationReceipt>(`/projects/job-requests/${segment(id)}`, "receipt", undefined, signal),
  admit: (project: string, body: JobAdmissionRequest) => request<JobMutationReceipt>(`/projects/${segment(project)}/jobs`, "receipt", body),
  mutate: (project: string, job: string, action: "cancel" | "retry" | "retry-publication", body: JobMutationRequest) => request<JobMutationReceipt>(`${jobPath(project, job)}/${action}`, "receipt", body),
  answer: (project: string, job: string, question: string, body: JobAnswerRequest) => request<JobMutationReceipt>(`${jobPath(project, job)}/questions/${segment(question)}/answers`, "receipt", body),
  review: (project: string, job: string, cursor?: string | null, run?: string, signal?: AbortSignal) => request<JobReview>(`${jobPath(project, job)}/review?limit=40${cursor ? `&cursor=${segment(cursor)}` : ""}${run ? `&run_id=${segment(run)}` : ""}`, "review", undefined, signal),
  artifact: (project: string, job: string, artifact: string, cursor?: string | null, signal?: AbortSignal) => request<JobArtifactContent>(`${jobPath(project, job)}/artifacts/${segment(artifact)}?limit=32768${cursor ? `&cursor=${segment(cursor)}` : ""}`, "artifact", undefined, signal),
};
