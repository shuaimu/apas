import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { WorkerDetail, WorkerItem, WorkerItemsResponse, WorkerRunsResponse } from "@apas/protocol";
import { WorkerDetailScreen } from "./WorkerDetail";
import { purgeWorkerCache } from "@/lib/workerCache";

const mocks = vi.hoisted(() => ({ detail: undefined as WorkerDetail | undefined, ready: true, fetch: vi.fn(), submit: vi.fn(), refresh: vi.fn() }));
vi.mock("@/lib/store", () => ({ useStore: (selector: (state: { userId: string }) => unknown) => selector({ userId: "account" }) }));
vi.mock("@/lib/workers", () => ({ workerFetch: (...args: unknown[]) => mocks.fetch(...args), workerPath: (project: string, worker: string) => `/projects/${project}/workers/${worker}`, workerHref: (project: string, worker: string, suffix = "") => `/code/workers/${worker}${suffix}?project=${project}` }));
vi.mock("@/lib/useWorkers", () => ({
  useWorker: () => ({ saved: mocks.detail ? { detail: mocks.detail, events: [], cursor: 1 } : undefined, ready: mocks.ready, project: "project", error: "", refresh: mocks.refresh }),
  useWorkerMutation: () => ({ busy: false, error: "", unresolved: false, retryAllowed: false, submit: mocks.submit, resolve: vi.fn() }),
}));
vi.mock("./WorkerComposer", () => ({ WorkerComposer: () => <p>Reviewed mandate editor</p> }));
vi.mock("./WorkerRun", () => ({ WorkerRun: () => <p>Exact run</p> }));

const usage = { elapsed_seconds: 2, input_tokens: 10, output_tokens: 20, spend_micro_usd: null, estimated: true };
const limits = { elapsed_seconds: 120, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000, memory_bytes: 1024, workspace_bytes: 2048, output_bytes: 4096, pids: 16, model_concurrency: 1, model_output_tokens: 1024 };
const detail: WorkerDetail = {
  worker: { id: "worker", project_id: "project", title: "Repository responsibility", description: "Full approved responsibility", state: "idle", desired_state: "active", reason: null, mandate_revision: 1, backlog_revision: 1, state_version: 7, high_watermark: 1, next_check_at: "2026-11-12T12:00:00Z", active_job_id: null, active_run_id: null, current_item_id: null, machine_id: "machine", cluster_owner_user_id: "owner", hostname: "host", attention: false, created_at: "2026-11-12T00:00:00Z", updated_at: "2026-11-12T01:00:00Z" },
  mandate: { revision: 1, repository: "example/project", worker_profile_digest: "worker", verification_profile_digest: "verifier", policy_digest: "policy", approved_by: "owner", approved_at: "2026-11-12T00:00:00Z", spec: { title: "Repository responsibility", description: "Full approved responsibility", checklist: ["Inspect maintainability"], machine_id: "machine", worker_profile: "worker", verification_profile: "verifier", model_profile: "model", base_ref: "main", interval_seconds: 900, limits: { per_run: limits, elapsed_seconds_24h: 1200, spend_micro_usd_24h: 10000, runs_24h: 10, attempts_per_item: 2 } } },
  checkpoint: null, current_job: null,
  budget: { usage, runs: 1, reserved_elapsed_seconds: 120, reserved_spend_micro_usd: 1000, reserved_runs: 1, accounting_uncertain: true, eligible_at: null, window_started_at: "2026-11-11T12:00:00Z" },
  allowed_actions: { edit_mandate: true, edit_backlog: true, pause: true, stop: false, resume: false, wake: true, retry: true, answer: false, retry_publication: false },
};
const item: WorkerItem = { id: "item", worker_id: "worker", revision: 2, item: { title: "Historical backlog item", instruction: "Inspect the retained finding", acceptance_criteria: ["Evidence retained"], scope_rationale: "Within the mandate", evidence: ["Discovery evidence"], priority: 1, dependencies: [] }, origin: "inspection", state: "blocked", attempts: 2, job_id: null, run_id: null, outcome: "Needs explicit disposition", created_at: "2026-11-12T00:00:00Z", updated_at: "2026-11-12T00:00:00Z" };
beforeEach(async () => {
  vi.stubGlobal("indexedDB", new IDBFactory());
  await purgeWorkerCache();
  mocks.detail = structuredClone(detail); mocks.ready = true; mocks.fetch.mockReset(); mocks.submit.mockReset(); mocks.refresh.mockReset();
  mocks.fetch.mockImplementation(async (path: string, kind: string): Promise<WorkerItemsResponse | WorkerRunsResponse> => kind === "items" ? { items: path.includes("cursor=") ? [item] : [], backlog_revision: 1, next_cursor: path.includes("cursor=") ? null : "items-next" } : { runs: [], next_cursor: "runs-next" });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("paginates backlog independently and preserves exhausted cumulative item attempts", async () => {
  render(<WorkerDetailScreen id="worker" projectHint="project" />);
  await waitFor(() => expect((screen.getByRole("button", { name: "Next backlog page" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Next backlog page" }));
  await screen.findByRole("heading", { name: "Historical backlog item" });
  expect(mocks.fetch).toHaveBeenCalledWith("/projects/project/workers/worker/items?cursor=items-next", "items");
  expect(mocks.fetch.mock.calls.filter((call) => call[1] === "runs")).toHaveLength(1);
  expect((screen.getByRole("button", { name: "Retry Historical backlog item" }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText(/Accounting is uncertain, not zero/)).toBeTruthy();
  expect(screen.getByText("Unknown — not zero")).toBeTruthy();
});

it("requires explicit lifecycle confirmation and submits the authoritative version", async () => {
  render(<WorkerDetailScreen id="worker" projectHint="project" />);
  expect((screen.getByRole("button", { name: "Stop now" }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Pause after this run" }));
  expect(mocks.submit).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Confirm Pause after this run" }));
  await waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce());
  expect(mocks.submit.mock.calls[0][0]).toMatchObject({ scope: ["account", "project", "worker"], path: "/projects/project/workers/worker/pause", body: { expected_version: 7 } });
  expect(screen.getByRole("heading", { name: "Status: idle" })).toBeTruthy();
});

it("does not render a late backlog response after a project purge", async () => {
  const pending = Promise.withResolvers<WorkerItemsResponse>();
  mocks.fetch.mockImplementation((_: string, kind: string) => kind === "items" ? pending.promise : Promise.resolve({ runs: [], next_cursor: null }));
  render(<WorkerDetailScreen id="worker" projectHint="project" />);
  await waitFor(() => expect(mocks.fetch).toHaveBeenCalledWith("/projects/project/workers/worker/items", "items"));
  await act(async () => { await purgeWorkerCache("account", "project"); pending.resolve({ items: [item], backlog_revision: 1, next_cursor: null }); });
  expect(screen.queryByRole("heading", { name: "Historical backlog item" })).toBeNull();
});
