import { act, renderHook, waitFor } from "@testing-library/react-native";
import type { WorkerDetail, WorkerMutationReceipt } from "@apas/protocol";
import { ApiError } from "@/api/client";
import type { JobScope } from "@/jobs/storage";
import { useJobReceipt } from "@/jobs/hooks";
import { readJobRecord } from "@/jobs/storage";
import { useWorkerMutation } from "./hooks";
import { useWorkers } from "./runtime";
import { workersApi } from "./api";

const mockRecords = new Map<string, unknown>();
jest.mock("expo-router", () => ({ useFocusEffect: jest.fn() }));
jest.mock("expo-crypto", () => ({ randomUUID: () => "new-request" }));
jest.mock("@/api/client", () => ({ ApiError: class extends Error { status: number; constructor(message: string, code: number) { super(message); this.status = code; } }, bootstrap: jest.fn() }));
jest.mock("@/jobs/runtime", () => ({ useJobs: jest.requireActual("zustand").create(() => ({ account: "actor", online: true, foreground: true, authorized: true })), purgeJobAccess: jest.fn(), jobControlsReady: jest.fn() }));
jest.mock("@/jobs/api", () => ({ jobsApi: { receipt: jest.fn() } }));
jest.mock("@/jobs/storage", () => ({
  readJobRecord: jest.fn(async (_scope: JobScope, key: string) => mockRecords.get(key) ?? null),
  writeJobRecord: jest.fn(async (_scope: JobScope, key: string, value: unknown) => { mockRecords.set(key, value); }),
  authorizeJobStorageProject: jest.fn(), jobCachedProjects: jest.fn(async () => []),
}));
jest.mock("@/query/client", () => ({ queryClient: { invalidateQueries: jest.fn(async () => undefined), cancelQueries: jest.fn(), removeQueries: jest.fn() } }));
jest.mock("./api", () => ({ abortWorkerRequests: jest.fn(), workersApi: { receipt: jest.fn(), executionReceipt: jest.fn(), mutate: jest.fn() } }));
jest.mock("./runtime", () => ({ ...jest.requireActual("./runtime"), synchronizeWorker: jest.fn(async () => undefined) }));
const scope = { account: "actor", project: "project", worker: "worker" };
const mutation = { kind: "pause" as const, body: { request_id: "original", expected_version: 7 } };
const receipt: WorkerMutationReceipt = { request_id: "original", project_id: "project", worker_id: "worker", operation: "pause", status: "accepted", state_version: 8, error_code: null };

beforeEach(() => {
  mockRecords.clear(); jest.clearAllMocks();
  useWorkers.setState({ account: "actor", online: true, foreground: true, authorized: true, mutations: true, revoked: {}, entries: { worker: { ready: true, cursor: 7, events: [], detail: { worker: { id: "worker", project_id: "project" } } as WorkerDetail } } });
});

test("restoring an uncertain lifecycle intent reads its original receipt and never replays POST", async () => {
  mockRecords.set("control", { mutation, receipt: null });
  jest.mocked(workersApi.receipt).mockResolvedValue(receipt);
  const hook = renderHook(() => useWorkerMutation(scope, "control"));
  await waitFor(() => expect(hook.result.current.record?.receipt?.status).toBe("accepted"));
  expect(workersApi.receipt).toHaveBeenCalledWith("original");
  expect(workersApi.receipt).toHaveBeenCalledTimes(1);
  expect(workersApi.mutate).not.toHaveBeenCalled();
  expect(mockRecords.get("control")).toEqual({ mutation, receipt });
  hook.unmount();
});

test("lost reply retains the original payload and only an explicit same-ID retry sends again", async () => {
  jest.mocked(workersApi.receipt).mockRejectedValue(new ApiError("not found", 404));
  jest.mocked(workersApi.mutate).mockRejectedValueOnce(new Error("connection lost after commit"));
  const hook = renderHook(() => useWorkerMutation(scope, "control"));
  await waitFor(() => expect(hook.result.current.loaded).toBe(true));
  await act(async () => { await hook.result.current.submit(mutation); });
  await waitFor(() => expect(hook.result.current.canRetry).toBe(true));
  expect(workersApi.mutate).toHaveBeenCalledTimes(1);
  expect(mockRecords.get("control")).toEqual({ mutation, receipt: null });
  jest.mocked(workersApi.mutate).mockResolvedValueOnce(receipt);
  await act(async () => { await hook.result.current.retry(); });
  expect(workersApi.mutate).toHaveBeenCalledTimes(2);
  expect(workersApi.mutate).toHaveBeenLastCalledWith("project", "worker", "pause", mutation.body);
  expect(hook.result.current.record?.receipt?.request_id).toBe("original");
  hook.unmount();
});

test("earlier finite receipts never render under a newly selected scope while storage is loading", async () => {
  const original = { mutation: { kind: "retry", body: { request_id: "earlier", expected_version: 1 } }, receipt: null };
  jest.mocked(readJobRecord).mockResolvedValueOnce(original);
  const pending = Promise.withResolvers<null>();
  const hook = renderHook(({ project }: { project: string }) => useJobReceipt({ account: "actor", project, job: "earlier-job" }, "legacy"), { initialProps: { project: "first" } });
  await waitFor(() => expect(hook.result.current.record?.mutation.body.request_id).toBe("earlier"));
  jest.mocked(readJobRecord).mockImplementationOnce(() => pending.promise);
  hook.rerender({ project: "second" });
  expect(hook.result.current.record).toBeNull();
  expect(hook.result.current.message).toBeNull();
  await act(async () => { pending.resolve(null); });
  expect(hook.result.current.record).toBeNull();
  hook.unmount();
});
