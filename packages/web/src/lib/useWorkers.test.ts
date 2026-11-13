import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type * as WorkerApi from "./workers";
import type * as JobApi from "./jobs";
import { useWorker, useWorkerDraft, useWorkerMutation } from "./useWorkers";
import { useJobDraft, useJobMutation } from "./useJobs";
import { resolveWorkerRequest, sendWorkerRequest, synchronizeWorker, WorkerHttpError } from "./workers";
import { resolveJobRequest } from "./jobs";
import * as workerCache from "./workerCache";
import * as jobCache from "./jobCache";
import type { SavedWorker, SavedWorkerRequest } from "./workerCache";
import type { SavedJobRequest } from "./jobCache";
import { workerAccount, workerDetail, workerEvent, workerId, workerReceipt, workerRequest, workerScope } from "./workers.test-fixtures";

const session = vi.hoisted(() => ({ userId: "7c6d2400-7b4b-455d-b732-7f066dab7a24", token: "account-token", accountStatus: "active", connected: true, isAuthenticated: true, negotiatedCapabilities: new Set(["autonomous_workers_v1", "autonomous_jobs_v1"]), ws: { readyState: 1, send: vi.fn() }, showToast: vi.fn() }));
vi.mock("./store", () => ({ useStore: Object.assign((selector: (state: typeof session) => unknown) => selector(session), { getState: () => session }) }));
vi.mock("./workers", async (original) => ({ ...await original<typeof WorkerApi>(), synchronizeWorker: vi.fn(), resolveWorkerRequest: vi.fn(), sendWorkerRequest: vi.fn() }));
vi.mock("./jobs", async (original) => ({ ...await original<typeof JobApi>(), resolveJobRequest: vi.fn() }));
const snapshot: SavedWorker = { detail: workerDetail(), events: [workerEvent(1)], cursor: 1 };

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(async () => {
  session.userId = workerAccount; session.token = "account-token"; session.connected = true; session.isAuthenticated = true;
  session.ws = { readyState: 1, send: vi.fn() };
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  vi.spyOn(navigator, "onLine", "get").mockReturnValue(true);
  await jobCache.purgeJobCache();
  vi.mocked(synchronizeWorker).mockResolvedValue(snapshot);
  vi.mocked(resolveWorkerRequest).mockImplementation(async (request) => ({ ...request, receipt: workerReceipt() }));
  vi.mocked(sendWorkerRequest).mockImplementation(async (request) => ({ ...request, receipt: workerReceipt() }));
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.mocked(synchronizeWorker).mockReset(); vi.mocked(resolveWorkerRequest).mockReset(); vi.mocked(sendWorkerRequest).mockReset(); vi.mocked(resolveJobRequest).mockReset(); });
afterAll(() => { vi.unstubAllGlobals(); });

describe("protected worker and execution drafts", () => {
  it("never restores a late worker draft after project revocation", async () => {
    const delayed = Promise.withResolvers<{ text: string } | undefined>();
    vi.spyOn(workerCache, "readWorkerRecord").mockImplementationOnce(<T,>() => delayed.promise as Promise<T | undefined>);
    const { result } = renderHook(() => useWorkerDraft(workerScope, "item:exact-item", { text: "" }));
    await act(async () => { await jobCache.purgeJobCache(workerAccount, workerScope[1]); });
    await act(async () => { delayed.resolve({ text: "Private old item" }); });
    expect(result.current.value).toEqual({ text: "" });
    expect(result.current.loaded).toBe(false);
    await expect(result.current.flush()).rejects.toThrow("access changed");
  });

  it("fences reused execution answer draft reads through the same barrier", async () => {
    const delayed = Promise.withResolvers<string | undefined>();
    vi.spyOn(jobCache, "readJobRecord").mockImplementationOnce(<T,>() => delayed.promise as Promise<T | undefined>);
    const { result } = renderHook(() => useJobDraft(workerScope, "run:exact-run:question:exact-question", ""));
    await act(async () => { await workerCache.purgeWorkerCache(workerAccount, workerScope[1]); });
    await act(async () => { delayed.resolve("Private old answer"); });
    expect(result.current.value).toBe("");
    expect(result.current.loaded).toBe(false);
  });

  it("flushes the final worker edit even when multiple keystrokes share one render", async () => {
    const { result } = renderHook(() => useWorkerDraft(workerScope, "mandate", ""));
    await waitFor(() => expect(result.current.loaded).toBe(true));
    await act(async () => {
      result.current.update("first edit");
      result.current.update("final edit");
      await result.current.flush();
    });
    expect(await workerCache.readWorkerRecord(workerScope, "draft", "mandate")).toBe("final edit");
  });

  it("hides exact-question drafts before effects restore a different scope", async () => {
    const { result, rerender } = renderHook(({ question }) => useWorkerDraft(workerScope, `run:run-a:question:${question}`, ""), { initialProps: { question: "question-a" } });
    await waitFor(() => expect(result.current.loaded).toBe(true));
    act(() => result.current.update("Answer for A"));
    rerender({ question: "question-b" });
    expect(result.current.value).toBe("");
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(await workerCache.readWorkerRecord(workerScope, "draft", "run:run-a:question:question-b")).toBeUndefined();
  });
});

describe("worker detail catch-up gate", () => {
  it("shows cached content read-only until replay and subscribes at the committed cursor", async () => {
    await workerCache.writeWorkerRecord(workerScope, "snapshot", snapshot);
    const delayed = Promise.withResolvers<SavedWorker>();
    vi.mocked(synchronizeWorker).mockReturnValueOnce(delayed.promise);
    const { result } = renderHook(() => useWorker(workerId, workerScope[1]));
    await waitFor(() => expect(result.current.saved?.cursor).toBe(1));
    expect(result.current.ready).toBe(false);
    expect(session.ws.send).not.toHaveBeenCalled();
    await act(async () => { delayed.resolve(snapshot); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(session.ws.send).toHaveBeenCalledWith(JSON.stringify({ type: "subscribe_worker", project_id: workerScope[1], worker_id: workerId, after_seq: 1 }));
  });

  it("cannot resurrect a purged detail from an outstanding catch-up", async () => {
    const delayed = Promise.withResolvers<SavedWorker>();
    vi.mocked(synchronizeWorker).mockReturnValueOnce(delayed.promise);
    const { result } = renderHook(() => useWorker(workerId, workerScope[1]));
    await waitFor(() => expect(synchronizeWorker).toHaveBeenCalledOnce());
    await act(async () => { await jobCache.purgeJobCache(workerAccount, workerScope[1]); });
    await act(async () => { delayed.resolve(snapshot); });
    expect(result.current.saved).toBeUndefined();
    expect(result.current.ready).toBe(false);
    expect(session.ws.send).not.toHaveBeenCalled();
    expect(synchronizeWorker).toHaveBeenCalledOnce();
  });
});

describe("explicit mutation receipt recovery", () => {
  it("only GET-recovers on mount and discards a receipt arriving after purge", async () => {
    const request = workerRequest();
    await workerCache.persistWorkerRequest(request);
    const delayed = Promise.withResolvers<SavedWorkerRequest>();
    vi.mocked(resolveWorkerRequest).mockReturnValueOnce(delayed.promise);
    const { result } = renderHook(() => useWorkerMutation(workerScope, request.path));
    await waitFor(() => expect(resolveWorkerRequest).toHaveBeenCalledOnce());
    expect(sendWorkerRequest).not.toHaveBeenCalled();
    await act(async () => { await jobCache.purgeJobCache(workerAccount, workerScope[1]); });
    await act(async () => { delayed.resolve({ ...request, receipt: workerReceipt() }); });
    expect(result.current.request).toBeUndefined();
    expect(sendWorkerRequest).not.toHaveBeenCalled();
  });

  it("requires receipt resolution before same-ID explicit retry, never reconnect replay", async () => {
    const request = workerRequest();
    await workerCache.persistWorkerRequest(request);
    vi.mocked(resolveWorkerRequest).mockRejectedValue(new WorkerHttpError(404, "not_found", "Unknown receipt"));
    const { result, rerender } = renderHook(() => useWorkerMutation(workerScope, request.path));
    await waitFor(() => expect(result.current.retryAllowed).toBe(true));
    expect(sendWorkerRequest).not.toHaveBeenCalled();
    session.connected = false;
    rerender();
    session.connected = true;
    rerender();
    await waitFor(() => expect(result.current.retryAllowed).toBe(true));
    expect(sendWorkerRequest).not.toHaveBeenCalled();
    await act(async () => { await result.current.submit({ ...request, id: "new-id", body: { request_id: "new-id", expected_version: 1 } }); });
    expect(sendWorkerRequest).not.toHaveBeenCalled();
    await act(async () => { await result.current.submit(request); });
    expect(sendWorkerRequest).toHaveBeenCalledWith(expect.objectContaining({ id: request.id, body: request.body }));
  });

  it("suppresses duplicate clicks and returns no navigation receipt after access changes", async () => {
    const request = workerRequest();
    const delayed = Promise.withResolvers<SavedWorkerRequest>();
    vi.mocked(sendWorkerRequest).mockReturnValueOnce(delayed.promise);
    const { result } = renderHook(() => useWorkerMutation(workerScope, request.path));
    await waitFor(() => expect(result.current.unresolved).toBe(false));
    let submission: Promise<SavedWorkerRequest | undefined> | undefined;
    act(() => { submission = result.current.submit(request); void result.current.submit(request); });
    await waitFor(() => expect(sendWorkerRequest).toHaveBeenCalledOnce());
    await act(async () => { await jobCache.purgeJobCache(workerAccount, workerScope[1]); });
    await act(async () => { delayed.resolve({ ...request, receipt: workerReceipt() }); await submission; });
    expect(await submission).toBeUndefined();
    expect(result.current.request).toBeUndefined();
  });

  it("also fences retained job-question receipt resolution after worker purge", async () => {
    const request: SavedJobRequest = { id: workerRequest().id, scope: workerScope, path: `/projects/project-a/workers/${workerId}/runs/exact-run/retry-publication`, body: { request_id: workerRequest().id, expected_version: 1 }, createdAt: "2026-11-08T00:00:00Z" };
    await jobCache.writeJobRecord(workerScope, "request", request, request.id);
    const delayed = Promise.withResolvers<SavedJobRequest>();
    vi.mocked(resolveJobRequest).mockReturnValueOnce(delayed.promise);
    const { result } = renderHook(() => useJobMutation(workerScope, request.path));
    await waitFor(() => expect(resolveJobRequest).toHaveBeenCalledOnce());
    await act(async () => { await workerCache.purgeWorkerCache(workerAccount, workerScope[1]); });
    await act(async () => { delayed.resolve(request); });
    expect(result.current.request).toBeUndefined();
  });
});
