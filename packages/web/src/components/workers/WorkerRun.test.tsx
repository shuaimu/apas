import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { JobDetail, JobReview, WorkerDetail } from "@apas/protocol";
import { WorkerRun } from "./WorkerRun";
import { purgeWorkerCache } from "@/lib/workerCache";

const transport = vi.hoisted(() => ({ fetch: vi.fn(), submit: vi.fn() }));
vi.mock("@/lib/jobs", () => ({ jobFetch: (...args: unknown[]) => transport.fetch(...args), jobPath: (project: string, job: string) => `/projects/${project}/jobs/${job}`, jobHref: (project: string, job: string) => `/code/jobs/${job}?project=${project}` }));
vi.mock("@/lib/workers", () => ({ workerPath: (project: string, worker: string) => `/projects/${project}/workers/${worker}`, workerHref: (project: string, worker: string, suffix = "") => `/code/workers/${worker}${suffix}?project=${project}` }));
vi.mock("@/lib/useJobs", () => ({ useJobMutation: () => ({ busy: false, unresolved: false, error: "", retryAllowed: false, submit: transport.submit, resolve: vi.fn() }) }));

const usage = { elapsed_seconds: 1, input_tokens: 1, output_tokens: 1, spend_micro_usd: 1, estimated: false };
const detail = {
  job: { id: "job", project_id: "project", current_run_id: "new-run", state_version: 3, allowed_actions: { answer: true, retry_publication: true } },
  task: { instruction: "Original run instruction", acceptance_criteria: ["Original criteria"], repository: "example/repo", base_ref: "main", base_sha: "original-base", worker: { worker_id: "worker", purpose: "implementation", mandate_revision: 1 } },
  runs: ["old-run", "new-run"].map((id) => ({ id, job_id: "job", state: "failed", usage, attempt: 1, epoch: 1, started_at: null, ended_at: null, reason: null, stop_evidence: null, candidate_sha: null, worker_version: null })),
  questions: [], publication: null,
} as unknown as JobDetail;
const worker = { worker: { id: "worker", state_version: 4, active_run_id: "new-run" }, current_job: detail, allowed_actions: { answer: true, retry_publication: true } } as WorkerDetail;
const review: JobReview = { job_id: "job", run_id: "old-run", base_sha: "original-base", candidate_sha: "original-candidate", files: [{ path: "original.ts", additions: 1, deletions: 0, binary: false, artifact_id: "diff", unavailable_reason: null }], verification: null, publication: null, reason: null, next_cursor: null };
const props = { scope: ["account", "project", "worker"] as const, worker, runId: "old-run", ready: true, refresh: vi.fn() };
beforeEach(async () => {
  vi.stubGlobal("indexedDB", new IDBFactory()); await purgeWorkerCache();
  transport.fetch.mockReset(); transport.submit.mockReset();
  transport.fetch.mockImplementation(async (_: string, kind: string) => kind === "detail" ? detail : kind === "review" ? review : { artifact: { id: "diff", run_id: "old-run", kind: "diff", sha256: "digest", size_bytes: 100, content_type: "text/plain", expires_at: null, truncated: true }, text: "retained original diff", next_cursor: "32", expired: false });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("keeps an older requested run through review, artifact pagination and embedded navigation", async () => {
  const navigate = vi.fn();
  render(<WorkerRun {...props} review onNavigate={navigate} />);
  await screen.findByText("original-candidate");
  expect(transport.fetch).toHaveBeenCalledWith("/projects/project/workers/worker/runs/old-run", "detail");
  expect(transport.fetch).toHaveBeenCalledWith("/projects/project/workers/worker/runs/old-run/review?limit=40&run_id=old-run", "review");
  fireEvent.click(screen.getByRole("button", { name: "Read diff for original.ts" }));
  await screen.findByText("retained original diff");
  expect(screen.getByText(/truncated at its retention limit/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Next evidence page" }));
  await waitFor(() => expect(transport.fetch).toHaveBeenCalledWith("/projects/project/workers/worker/runs/old-run/artifacts/diff?limit=32768&offset=32", "artifact"));
  fireEvent.click(screen.getByRole("button", { name: "Run evidence" }));
  expect(navigate).toHaveBeenCalledWith({ runId: "old-run" });
  expect((screen.getByRole("button", { name: "Retry draft publication only" }) as HTMLButtonElement).disabled).toBe(true);
  expect(transport.submit).not.toHaveBeenCalled();
});

it("rejects a mismatched latest review rather than substituting it", async () => {
  transport.fetch.mockImplementation(async (_: string, kind: string) => kind === "detail" ? detail : { ...review, run_id: "new-run", candidate_sha: "wrong-candidate" });
  render(<WorkerRun {...props} review />);
  await screen.findByText(/requested run does not match this review/);
  expect(screen.queryByText("wrong-candidate")).toBeNull();
});

it("clears review and rejects an artifact response arriving after project purge", async () => {
  const pending = Promise.withResolvers<unknown>();
  transport.fetch.mockImplementation((_: string, kind: string) => kind === "detail" ? Promise.resolve(detail) : kind === "review" ? Promise.resolve(review) : pending.promise);
  render(<WorkerRun {...props} review />);
  fireEvent.click(await screen.findByRole("button", { name: "Read diff for original.ts" }));
  await waitFor(() => expect(transport.fetch).toHaveBeenCalledWith("/projects/project/workers/worker/runs/old-run/artifacts/diff?limit=32768", "artifact"));
  await act(async () => { await purgeWorkerCache("account", "project"); pending.resolve({ artifact: { id: "diff", run_id: "old-run", sha256: "secret", size_bytes: 1 }, text: "revoked source", next_cursor: null, expired: false }); });
  expect(screen.queryByText("original-candidate")).toBeNull();
  expect(screen.queryByText("revoked source")).toBeNull();
});
