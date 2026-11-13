import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { WorkerListResponse } from "@apas/protocol";
import type * as WorkerApi from "./workers";
import { useProjectWorkers } from "./useProjectWorkers";
import { receiveWorkerMessage, workerFetch } from "./workers";
import { purgeJobCache } from "./jobCache";
import { readWorkerRecord, writeWorkerRecord } from "./workerCache";
import { workerAccount, workerSummary } from "./workers.test-fixtures";

const session = vi.hoisted(() => ({ userId: "7c6d2400-7b4b-455d-b732-7f066dab7a24", token: "account-token", ws: {} as object, online: true, isAuthenticated: true, accountStatus: "active", negotiatedCapabilities: new Set(["autonomous_workers_v1"]) }));
vi.mock("./store", () => ({ useStore: Object.assign((selector: (state: typeof session) => unknown) => selector(session), { getState: () => session }) }));
vi.mock("./useWorkers", () => ({ useWorkerOnline: () => session.online }));
vi.mock("./workers", async (original) => ({ ...await original<typeof WorkerApi>(), workerFetch: vi.fn() }));
interface PendingPage { path: string; resolve: (page: WorkerListResponse) => void; reject: (reason: unknown) => void }
const requests: PendingPage[] = [];
beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(async () => {
  session.userId = workerAccount; session.token = "account-token"; session.ws = {}; session.online = true;
  requests.length = 0;
  await purgeJobCache();
  vi.mocked(workerFetch).mockImplementation(<T,>(path: string) => {
    const { promise, resolve, reject } = Promise.withResolvers<WorkerListResponse>();
    requests.push({ path, resolve, reject });
    return promise as Promise<T>;
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.mocked(workerFetch).mockReset(); });
afterAll(() => { vi.unstubAllGlobals(); });

describe("worker inventory authority scopes", () => {
  it("uses project-authorized pagination, undefined global filters, and null to disable", async () => {
    const { result, rerender } = renderHook(({ project }: { project: string | null | undefined }) => useProjectWorkers(project, "attention"), { initialProps: { project: null as string | null | undefined } });
    expect(requests).toHaveLength(0);
    rerender({ project: undefined });
    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0].path).toBe("/projects/workers?status=attention");
    await act(async () => { requests[0].resolve({ workers: [{ ...workerSummary(), attention: true }], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    rerender({ project: "project-a" });
    expect(result.current.workers).toEqual([]);
    await waitFor(() => expect(requests).toHaveLength(2));
    expect(requests[1].path).toBe("/projects/project-a/workers?status=attention");
    rerender({ project: null });
    await act(async () => { requests[1].resolve({ workers: [{ ...workerSummary(), attention: true }], next_cursor: null }); });
    expect(result.current.workers).toEqual([]);
    expect(result.current.ready).toBe(false);
    expect(requests).toHaveLength(2);
  });

  it("hides old account/project rows synchronously and ignores their late pages", async () => {
    const { result, rerender } = renderHook(({ project }) => useProjectWorkers(project), { initialProps: { project: "project-a" } });
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].resolve({ workers: [workerSummary()], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    act(() => result.current.loadMore());
    expect(requests).toHaveLength(2);
    session.userId = "other-account"; session.token = "other-token";
    rerender({ project: "project-b" });
    expect(result.current.workers).toEqual([]);
    expect(result.current.ready).toBe(false);
    await waitFor(() => expect(requests).toHaveLength(3));
    await act(async () => { requests[1].resolve({ workers: [workerSummary(2)], next_cursor: null }); });
    expect(result.current.workers).toEqual([]);
    await act(async () => { requests[2].resolve({ workers: [], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(await readWorkerRecord(["other-account", "project-b", ""], "list", "all")).toEqual({ workers: [], next_cursor: null });
  });

  it("fences purge races and socket hints until explicit reauthorization", async () => {
    const { result } = renderHook(() => useProjectWorkers("project-a"));
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].resolve({ workers: [workerSummary()], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    act(() => result.current.loadMore());
    await act(async () => { await purgeJobCache(workerAccount, "project-a"); });
    expect(result.current.workers).toEqual([]);
    expect(result.current.ready).toBe(false);
    await act(async () => { requests[1].resolve({ workers: [workerSummary(3)], next_cursor: null }); });
    act(() => receiveWorkerMessage({ type: "worker_inbox_changed" }));
    expect(requests).toHaveLength(2);
    expect(await readWorkerRecord([workerAccount, "project-a", ""], "list", "all")).toBeUndefined();
    act(() => result.current.refresh());
    await waitFor(() => expect(requests).toHaveLength(3));
    await act(async () => { requests[2].resolve({ workers: [], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
  });
});

describe("bounded worker inventory refresh", () => {
  it("deduplicates overlap without regressing descriptions and refreshes only opened pages", async () => {
    const { result } = renderHook(() => useProjectWorkers("project-a"));
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].resolve({ workers: [workerSummary(3)], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    act(() => { result.current.loadMore(); result.current.loadMore(); });
    expect(requests).toHaveLength(2);
    expect(requests[1].path).toBe("/projects/project-a/workers?cursor=older");
    act(() => receiveWorkerMessage({ type: "worker_inbox_changed" }));
    expect(requests).toHaveLength(2);
    await act(async () => { requests[1].resolve({ workers: [workerSummary(1), workerSummary(2, "project-a", "second-worker")], next_cursor: "further" }); });
    await waitFor(() => expect(requests).toHaveLength(3));
    expect(result.current.ready).toBe(false);
    expect(result.current.workers.map((worker) => worker.state_version)).toEqual([3, 2]);
    await act(async () => { requests[2].resolve({ workers: [workerSummary(3)], next_cursor: "new-boundary" }); });
    await waitFor(() => expect(requests).toHaveLength(4));
    await act(async () => { requests[3].resolve({ workers: [workerSummary(3), workerSummary(3, "project-a", "second-worker")], next_cursor: "more-history" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.workers).toHaveLength(2);
    expect(result.current.workers[0].description).toBe(workerSummary().description);
    expect(result.current.hasMore).toBe(true);
    expect(requests).toHaveLength(4);
  });

  it("restores an offline first page read-only and catches up before reconnect controls", async () => {
    await writeWorkerRecord([workerAccount, "project-a", ""], "list", { workers: [workerSummary()], next_cursor: "older" }, "all");
    session.online = false;
    const { result, rerender } = renderHook(() => useProjectWorkers("project-a"));
    await waitFor(() => expect(result.current.workers).toHaveLength(1));
    expect(result.current.ready).toBe(false);
    act(() => result.current.loadMore());
    expect(requests).toHaveLength(0);
    session.online = true;
    rerender();
    expect(result.current.ready).toBe(false);
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].resolve({ workers: [workerSummary(2)], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.workers[0].state_version).toBe(2);
  });
});
