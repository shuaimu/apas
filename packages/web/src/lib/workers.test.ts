import { waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import { jobFetch } from "./jobs";
import { onWorkerMessage, receiveWorkerMessage, resolveWorkerRequest, sendWorkerRequest, synchronizeWorker, workerFetch, workerPath } from "./workers";
import { persistWorkerRequest, purgeWorkerCache, readWorkerRecord, workerRequests, writeWorkerRecord } from "./workerCache";
import type { SavedWorker, SavedWorkerRequest } from "./workerCache";
import { workerAccount, workerDetail, workerEvent, workerId, workerPage, workerReceipt, workerRequest, workerScope } from "./workers.test-fixtures";

const session = vi.hoisted(() => ({ userId: "7c6d2400-7b4b-455d-b732-7f066dab7a24", token: "account-token", accountStatus: "active", isAuthenticated: true, negotiatedCapabilities: new Set(["autonomous_workers_v1"]), logout: vi.fn() }));
vi.mock("./store", () => ({ useStore: { getState: () => session } }));
const response = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json" } });

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(async () => {
  session.userId = workerAccount; session.token = "account-token"; session.accountStatus = "active"; session.isAuthenticated = true;
  session.negotiatedCapabilities = new Set(["autonomous_workers_v1"]);
  session.logout.mockClear();
  await purgeWorkerCache();
});
afterEach(() => { vi.restoreAllMocks(); });
afterAll(() => { vi.unstubAllGlobals(); });

describe("worker HTTP authorization and canonical contracts", () => {
  it("rejects unsupported successful payloads instead of accepting a truthy body", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response({ worker: { id: workerId } })));
    await expect(workerFetch(workerPath(workerScope[1], workerId), "detail")).rejects.toThrow("unsupported worker response");
  });

  it.each([workerFetch, jobFetch])("fences old-account 401 responses before logout side effects", async (fetchContract) => {
    const pending = Promise.withResolvers<Response>();
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(pending.promise));
    const loading = fetchContract("/projects/project-a/workers", "receipt");
    session.userId = "other-account"; session.token = "other-token";
    pending.resolve(response({ error: "Expired", code: "authentication_required" }, 401));
    await expect(loading).rejects.toThrow("access changed");
    expect(session.logout).not.toHaveBeenCalled();
  });

  it("fences decoding already in progress when the project is purged", async () => {
    const decoded = Promise.withResolvers<unknown>();
    const wire = response(workerDetail());
    vi.spyOn(wire, "json").mockReturnValue(decoded.promise);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(wire));
    const loading = workerFetch(workerPath(workerScope[1], workerId), "detail");
    await waitFor(() => expect(wire.json).toHaveBeenCalledOnce());
    await purgeWorkerCache(workerAccount, workerScope[1]);
    decoded.resolve(workerDetail());
    await expect(loading).rejects.toThrow("access changed");
  });

  it("purges project and global protected content on a current denial", async () => {
    await writeWorkerRecord(workerScope, "draft", { private: true });
    await writeWorkerRecord([workerAccount, "", ""], "list", { private: true });
    await writeWorkerRecord([workerAccount, "other-project", ""], "draft", { keep: true });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response({ error: "Revoked", code: "forbidden" }, 403)));
    await expect(workerFetch(workerPath(workerScope[1], workerId), "detail")).rejects.toThrow();
    expect(await readWorkerRecord(workerScope, "draft")).toBeUndefined();
    expect(await readWorkerRecord([workerAccount, "", ""], "list")).toBeUndefined();
    expect(await readWorkerRecord([workerAccount, "other-project", ""], "draft")).toEqual({ keep: true });
  });
});

describe("ordered HTTP catch-up", () => {
  it("serializes concurrent readers and fetches a final snapshot covering every committed event", async () => {
    let details = 0;
    const fetcher = vi.fn(async (url: string) => {
      if (url.includes("after_seq=0")) return response(workerPage([workerEvent(1)]));
      if (url.includes("after_seq=1")) return response(workerPage([workerEvent(2)]));
      details += 1;
      return response(workerDetail(details === 1 ? 1 : 2));
    });
    vi.stubGlobal("fetch", fetcher);
    const first = synchronizeWorker(workerScope);
    expect(synchronizeWorker(workerScope)).toBe(first);
    const saved = await first;
    expect(saved.cursor).toBe(2);
    expect(saved.detail.worker.high_watermark).toBe(2);
    expect(saved.events.map((event) => event.seq)).toEqual([1, 2]);
    expect(details).toBe(3);
    expect(await readWorkerRecord(workerScope, "snapshot")).toEqual(saved);
  });

  it("resynchronizes an expired cursor once and does not invent missing history", async () => {
    vi.stubGlobal("fetch", vi.fn(async (url: string) => {
      if (url.includes("after_seq=0")) return response({ ...workerPage([], 20), resync_required: true });
      if (url.includes("after_seq=20")) return response(workerPage([], 20));
      return response(workerDetail(20));
    }));
    const saved = await synchronizeWorker(workerScope);
    expect(saved).toMatchObject({ cursor: 20, events: [], historyResynced: true });
  });

  it("does not continue replay or persist late detail after a purge", async () => {
    const pending = Promise.withResolvers<Response>();
    const fetcher = vi.fn().mockReturnValue(pending.promise);
    vi.stubGlobal("fetch", fetcher);
    const loading = synchronizeWorker(workerScope);
    await purgeWorkerCache(workerAccount, workerScope[1]);
    pending.resolve(response(workerDetail()));
    await expect(loading).rejects.toThrow("access changed");
    expect(fetcher).toHaveBeenCalledOnce();
    expect(await readWorkerRecord<SavedWorker>(workerScope, "snapshot")).toBeUndefined();
  });

  it("bounds total catch-up work and retains committed progress without enabling controls", async () => {
    const fetcher = vi.fn(async (url: string) => {
      const cursor = new URL(url).searchParams.get("after_seq");
      if (cursor === null) return response(workerDetail(100));
      return response(workerPage([workerEvent(Number(cursor) + 1)], 100, true));
    });
    vi.stubGlobal("fetch", fetcher);
    await expect(synchronizeWorker(workerScope)).rejects.toThrow("bounded page limit");
    expect(fetcher).toHaveBeenCalledTimes(33);
    expect((await readWorkerRecord<SavedWorker>(workerScope, "snapshot"))?.cursor).toBe(32);
  });
});

describe("durable same-ID mutation recovery", () => {
  it("only resolves the original receipt and never posts during recovery", async () => {
    const request = workerRequest();
    await persistWorkerRequest(request);
    const fetcher = vi.fn().mockResolvedValue(response(workerReceipt()));
    vi.stubGlobal("fetch", fetcher);
    const saved = await resolveWorkerRequest(request);
    expect(saved.receipt?.worker_id).toBe(workerId);
    expect(fetcher).toHaveBeenCalledWith(expect.stringContaining(`/projects/worker-requests/${request.id}`), expect.objectContaining({ method: "GET" }));
    expect((await workerRequests(workerAccount))[0].receipt).toEqual(workerReceipt());
  });

  it("persists before POST and explicitly retries an unknown receipt using unchanged identity and payload", async () => {
    const request = workerRequest();
    await persistWorkerRequest(request);
    const methods: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (_url: string, init: RequestInit) => {
      methods.push(init.method ?? "GET");
      expect((await readWorkerRecord<SavedWorkerRequest>(workerScope, "request", request.id))?.body).toEqual(request.body);
      if (init.method === "GET") return response({ error: "Unknown receipt", code: "not_found" }, 404);
      expect(JSON.parse(String(init.body))).toEqual(request.body);
      return response(workerReceipt());
    }));
    expect((await sendWorkerRequest(request)).receipt?.status).toBe("accepted");
    expect(methods).toEqual(["GET", "POST"]);
    await sendWorkerRequest(request);
    expect(methods).toEqual(["GET", "POST"]);
  });

  it("does not replay a still-pending request or accept a foreign receipt", async () => {
    const request = workerRequest();
    await persistWorkerRequest(request);
    const fetcher = vi.fn().mockResolvedValue(response({ ...workerReceipt(), status: "pending" }));
    vi.stubGlobal("fetch", fetcher);
    expect((await sendWorkerRequest(request)).receipt?.status).toBe("pending");
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(fetcher.mock.calls[0][1].method).toBe("GET");
    fetcher.mockResolvedValue(response({ ...workerReceipt(), project_id: "foreign-project" }));
    await expect(resolveWorkerRequest(request)).rejects.toThrow("does not match");
  });
});

describe("authenticated worker socket dispatch", () => {
  it("requires the negotiated capability and stops fanout at a purge barrier", async () => {
    const first = vi.fn();
    const stop = onWorkerMessage(first);
    session.negotiatedCapabilities.clear();
    receiveWorkerMessage({ type: "worker_inbox_changed" });
    expect(first).not.toHaveBeenCalled();
    session.negotiatedCapabilities.add("autonomous_workers_v1");
    receiveWorkerMessage({ type: "worker_inbox_changed" });
    expect(first).toHaveBeenCalledOnce();
    stop();
    let purge: Promise<void> | undefined;
    const removeFirst = onWorkerMessage(() => { purge = purgeWorkerCache(workerAccount); });
    const afterPurge = vi.fn();
    const removeSecond = onWorkerMessage(afterPurge);
    try {
      receiveWorkerMessage({ type: "worker_inbox_changed" });
      await purge;
      expect(afterPurge).not.toHaveBeenCalled();
    } finally { removeFirst(); removeSecond(); }
  });
});
