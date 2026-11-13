import type { WorkerDetail, WorkerEvent } from "@apas/protocol";
import { invalidateWorkers, synchronizeWorker, useWorkers, workerControlsReady, workerStorageScope } from "./runtime";
import { workersApi } from "./api";
import { readJobRecord, writeJobRecord } from "@/jobs/storage";

jest.mock("@/api/client", () => ({ ApiError: class extends Error { status: number; constructor(message: string, code: number) { super(message); this.status = code; } }, bootstrap: jest.fn() }));
jest.mock("@/jobs/runtime", () => ({ purgeJobAccess: jest.fn() }));
jest.mock("@/query/client", () => ({ queryClient: { cancelQueries: jest.fn(), removeQueries: jest.fn(), invalidateQueries: jest.fn() } }));
jest.mock("@/jobs/storage", () => ({ authorizeJobStorageProject: jest.fn(), jobCachedProjects: jest.fn(async () => []), readJobRecord: jest.fn(async () => null), writeJobRecord: jest.fn(async () => undefined) }));
jest.mock("./api", () => ({ abortWorkerRequests: jest.fn(), workersApi: { detail: jest.fn(), resolve: jest.fn(), events: jest.fn() } }));
const detail = (watermark: number): WorkerDetail => ({ worker: { id: "worker", project_id: "project", high_watermark: watermark, state: "pausing", desired_state: "paused" }, checkpoint: { run_id: "completed-current-run", summary: "Current outcome retained" } }) as WorkerDetail;
const event: WorkerEvent = { worker_id: "worker", project_id: "project", seq: 2, state_version: 3, kind: "checkpoint", summary: "Accepted current outcome while paused", run_id: "completed-current-run", created_at: "2026-01-01T00:00:00Z" };

beforeEach(() => {
  invalidateWorkers(); jest.clearAllMocks();
  jest.mocked(readJobRecord).mockResolvedValue(null);
  useWorkers.setState({ account: "account", online: true, foreground: true, authorized: true, supported: true, mutations: true, entries: {}, revoked: {} });
});

test("connected transport and a cached snapshot cannot grant mutation authority", () => {
  useWorkers.setState({ authorized: false, entries: { worker: { detail: detail(1), cursor: 1, events: [], ready: true } } });
  expect(workerControlsReady("worker")).toBe(false);
  useWorkers.setState({ authorized: true, foreground: false });
  expect(workerControlsReady("worker")).toBe(false);
});

test("accepted snapshot, bounded events and watermark persist together before controls enable", async () => {
  jest.mocked(readJobRecord).mockResolvedValue({ detail: detail(1), cursor: 1, events: [] });
  jest.mocked(workersApi.detail).mockResolvedValue(detail(2));
  jest.mocked(workersApi.events).mockResolvedValue({ events: [event], high_watermark: 2, has_more: false, resync_required: false });
  await synchronizeWorker("worker", "project");
  expect(writeJobRecord).toHaveBeenCalledWith(workerStorageScope({ account: "account", project: "project", worker: "worker" }), "snapshot", { detail: detail(2), cursor: 2, events: [event] });
  expect(useWorkers.getState().entries.worker.detail.checkpoint?.run_id).toBe("completed-current-run");
  expect(useWorkers.getState().entries.worker.detail.worker.desired_state).toBe("paused");
  expect(workerControlsReady("worker")).toBe(true);
});

test("late HTTP completion after the revocation generation cannot restore content", async () => {
  const pending = Promise.withResolvers<WorkerDetail>();
  jest.mocked(workersApi.detail).mockReturnValueOnce(pending.promise);
  const work = synchronizeWorker("worker", "project");
  invalidateWorkers();
  pending.resolve(detail(2));
  await work;
  expect(writeJobRecord).not.toHaveBeenCalled();
  expect(useWorkers.getState().entries.worker).toBeUndefined();
  expect(workerControlsReady("worker")).toBe(false);
});

test("a retained replay gap refreshes the baseline rather than guessing missing events", async () => {
  jest.mocked(readJobRecord).mockResolvedValue({ detail: detail(1), cursor: 1, events: [] });
  jest.mocked(workersApi.detail).mockResolvedValue(detail(9));
  jest.mocked(workersApi.events).mockResolvedValueOnce({ events: [], high_watermark: 9, has_more: false, resync_required: true }).mockResolvedValueOnce({ events: [], high_watermark: 9, has_more: false, resync_required: false });
  await synchronizeWorker("worker", "project");
  expect(useWorkers.getState().entries.worker.cursor).toBe(9);
  expect(useWorkers.getState().entries.worker.events).toEqual([]);
  expect(workersApi.events).toHaveBeenLastCalledWith("project", "worker", 9);
});
