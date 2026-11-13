import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkerSummary } from "@apas/protocol";
import { WorkerList } from "./WorkerList";

const mocks = vi.hoisted(() => ({ inventory: vi.fn(), requests: vi.fn(), resolve: vi.fn(), loadMore: vi.fn(), refresh: vi.fn() }));
vi.mock("@/lib/store", () => ({ useStore: (select: (state: { userId: string }) => unknown) => select({ userId: "account" }) }));
vi.mock("@/lib/useProjectWorkers", () => ({ useProjectWorkers: (...args: unknown[]) => mocks.inventory(...args) }));
vi.mock("@/lib/workerCache", () => ({ workerCacheRevision: () => 0, workerRequests: mocks.requests, onWorkerCachePurge: () => () => {} }));
vi.mock("@/lib/workers", () => ({ workerHref: (project: string, worker: string, suffix = "") => `/code/workers/${worker}${suffix}?project=${project}`, resolveWorkerRequest: mocks.resolve }));

const worker: WorkerSummary = {
  id: "responsibility", project_id: "project", title: "Maintain the parser", description: "The entire standing responsibility remains accessible without fetching each detail.",
  state: "waiting_review", reason: "Review the verified draft", desired_state: "active", mandate_revision: 2, backlog_revision: 3, state_version: 9, high_watermark: 10,
  created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-02T00:00:00Z",
  next_check_at: null, active_job_id: null, active_run_id: null, attention_run_id: "exact-older-run", attention_question_id: null, current_item_id: "selected-item", machine_id: "host", cluster_owner_user_id: "owner", hostname: "Build host", attention: true,
};

beforeEach(() => {
  vi.clearAllMocks(); mocks.requests.mockResolvedValue([]);
  mocks.inventory.mockReturnValue({ workers: [worker], ready: true, loading: false, error: "", hasMore: true, loadMore: mocks.loadMore, refresh: mocks.refresh });
});
describe("global worker oversight", () => {
  it("uses bounded attention inventory and retains exact review identity without mutations", async () => {
    render(<WorkerList attention />);
    expect(mocks.inventory).toHaveBeenCalledWith(undefined, "attention");
    expect(screen.getByRole("link", { name: "Open exact review" }).getAttribute("href")).toBe("/code/workers/responsibility/runs/exact-older-run/review?project=project");
    expect(screen.getByText(worker.description)).toBeTruthy();
    expect(screen.getByText(/loaded subset, not an authoritative total/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Load more workers" }));
    expect(mocks.loadMore).toHaveBeenCalledOnce();
    await waitFor(() => expect(mocks.requests).toHaveBeenCalledWith("account"));
    expect(mocks.resolve).not.toHaveBeenCalled();
  });
  it("opens the canonical exact attention dialog rather than a later active run", () => {
    mocks.inventory.mockReturnValue({ workers: [{ ...worker, state: "needs_input", active_run_id: "later-active-run", attention_run_id: "question-run", attention_question_id: "exact-dialog" }], ready: true, loading: false, error: "", hasMore: false, loadMore: mocks.loadMore, refresh: mocks.refresh });
    render(<WorkerList attention />);
    expect(screen.getByRole("link", { name: "Open exact question" }).getAttribute("href")).toBe("/code/workers/responsibility/runs/question-run/questions/exact-dialog?project=project");
    expect(screen.getByRole("link", { name: "Open current run and questions" }).getAttribute("href")).toBe("/code/workers/responsibility/runs/later-active-run?project=project");
    expect(mocks.resolve).not.toHaveBeenCalled();
  });
  it("keeps cached inventories visibly stale and disables recovery and further paging", async () => {
    mocks.inventory.mockReturnValue({ workers: [worker], ready: false, loading: false, error: "", hasMore: true, loadMore: mocks.loadMore, refresh: mocks.refresh });
    mocks.requests.mockResolvedValue([{ id: "same-request", scope: ["account", "project", "responsibility"], path: "/projects/project/workers/responsibility/pause", body: { request_id: "same-request", expected_version: 9 }, createdAt: "2026-01-02T00:00:00Z" }]);
    render(<WorkerList />);
    expect(screen.getByText(/Remote revocations cannot be discovered while offline/)).toBeTruthy();
    const recover = await screen.findByRole("button", { name: "Resolve original receipt" });
    expect((recover as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Load more workers" }) as HTMLButtonElement).disabled).toBe(true);
    expect(mocks.resolve).not.toHaveBeenCalled();
  });
});
