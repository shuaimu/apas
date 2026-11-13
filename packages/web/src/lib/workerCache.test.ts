import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { SavedWorker, WorkerScope } from "./workerCache";
import * as cache from "./workerCache";
import { purgeJobCache, readJobRecord, writeJobRecord } from "./jobCache";
import { workerAccount, workerDetail, workerEvent, workerPage, workerReceipt, workerRequest, workerScope } from "./workers.test-fixtures";

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(() => purgeJobCache());
afterAll(() => { vi.unstubAllGlobals(); });

describe("atomic bounded worker replay", () => {
  it("aborts an entire gapped page without changing detail, event bytes or cursor", async () => {
    await cache.commitWorkerEvents(workerScope, workerDetail(), workerPage([workerEvent(1)]));
    await expect(cache.commitWorkerEvents(workerScope, workerDetail(4), workerPage([workerEvent(2), workerEvent(4)]))).rejects.toThrow("gap");
    const retained = await cache.readWorkerRecord<SavedWorker>(workerScope, "snapshot");
    expect(retained?.cursor).toBe(1);
    expect(retained?.detail.worker.state_version).toBe(1);
    expect(retained?.events.map((event) => event.seq)).toEqual([1]);
  });

  it("deduplicates overlap and keeps only a bounded ordered suffix", async () => {
    for (let start = 1; start <= 201; start += 100) {
      const end = Math.min(start + 99, 250);
      const events = Array.from({ length: end - start + 1 }, (_, index) => workerEvent(start + index));
      await cache.commitWorkerEvents(workerScope, workerDetail(end), workerPage(events, 250, end < 250));
    }
    await cache.commitWorkerEvents(workerScope, workerDetail(251), workerPage([workerEvent(250), workerEvent(251)]));
    const retained = await cache.readWorkerRecord<SavedWorker>(workerScope, "snapshot");
    expect(retained?.cursor).toBe(251);
    expect(retained?.events).toHaveLength(200);
    expect(retained?.events[0].seq).toBe(52);
  });

  it.each([
    ["foreign project", () => workerPage([{ ...workerEvent(2), project_id: "other-project" }])],
    ["foreign worker", () => workerPage([{ ...workerEvent(2), worker_id: "9780bba0-8f31-4800-990f-f02eec0f1111" }])],
    ["unordered page", () => workerPage([workerEvent(3), workerEvent(2)])],
    ["watermark without events", () => workerPage([], 3)],
    ["nonadvancing page", () => workerPage([], 3, true)],
  ] as const)("rejects %s without advancing durable state", async (_name, page) => {
    await cache.commitWorkerEvents(workerScope, workerDetail(), workerPage([workerEvent(1)]));
    await expect(cache.commitWorkerEvents(workerScope, workerDetail(3), page())).rejects.toBeInstanceOf(Error);
    expect((await cache.readWorkerRecord<SavedWorker>(workerScope, "snapshot"))?.cursor).toBe(1);
  });

  it("records explicit retention resets atomically and rejects regressing final snapshots", async () => {
    await cache.commitWorkerEvents(workerScope, workerDetail(), workerPage([workerEvent(1)]));
    const reset = await cache.commitWorkerSnapshot(workerScope, workerDetail(20), true);
    expect(reset).toMatchObject({ cursor: 20, events: [], historyResynced: true });
    await expect(cache.commitWorkerSnapshot(workerScope, workerDetail(19))).rejects.toThrow("regressed");
    await cache.commitWorkerEvents(workerScope, workerDetail(21), workerPage([workerEvent(21)]));
    expect((await cache.commitWorkerSnapshot(workerScope, workerDetail(21))).historyResynced).toBe(true);
  });
});

describe("shared worker and execution purge barrier", () => {
  it("purges every protected record and global page without deleting other projects/accounts", async () => {
    const scopes: WorkerScope[] = [workerScope, [workerAccount, "", ""], [workerAccount, "other-project", "other-worker"], ["other-account", workerScope[1], workerScope[2]]];
    for (const scope of scopes) {
      for (const kind of ["snapshot", "draft", "list", "preferences", "request", "evidence"]) await cache.writeWorkerRecord(scope, kind, { private: scope[1] });
    }
    await writeJobRecord(workerScope, "draft", { answer: "Protected execution answer" });
    await purgeJobCache(workerAccount, workerScope[1]);
    for (const scope of scopes.slice(0, 2)) {
      for (const kind of ["snapshot", "draft", "list", "preferences", "request", "evidence"]) expect(await cache.readWorkerRecord(scope, kind)).toBeUndefined();
    }
    expect(await readJobRecord(workerScope, "draft")).toBeUndefined();
    expect(await cache.readWorkerRecord(scopes[2], "draft")).toEqual({ private: "other-project" });
    expect(await cache.readWorkerRecord(scopes[3], "draft")).toEqual({ private: "project-a" });
  });

  it("cannot collide with historical jobs even when IDs and record kinds match", async () => {
    await writeJobRecord(workerScope, "snapshot", { old: "job" });
    await cache.writeWorkerRecord(workerScope, "snapshot", { current: "worker" });
    expect(await readJobRecord(workerScope, "snapshot")).toEqual({ old: "job" });
    expect(await cache.readWorkerRecord(workerScope, "snapshot")).toEqual({ current: "worker" });
  });

  it("blocks remount reads and rejects stale writes/commits after a job purge", async () => {
    const revision = cache.workerCacheRevision();
    await cache.writeWorkerRecord(workerScope, "draft", { instruction: "Private responsibility" });
    let restored: Promise<unknown> | undefined;
    const remove = cache.onWorkerCachePurge(() => { restored = cache.readWorkerRecord(workerScope, "draft"); });
    try {
      await purgeJobCache(workerAccount, workerScope[1]);
      expect(await restored).toBeUndefined();
      await expect(cache.writeWorkerRecord(workerScope, "draft", { instruction: "Late bytes" }, "", revision)).rejects.toThrow();
      await expect(cache.commitWorkerEvents(workerScope, workerDetail(), workerPage([workerEvent(1)]), revision)).rejects.toThrow();
      expect(await cache.readWorkerRecord(workerScope, "snapshot")).toBeUndefined();
    } finally { remove(); }
  });

  it("preserves request identity, terminal receipts and actor isolation", async () => {
    const request = workerRequest();
    await cache.persistWorkerRequest(request);
    await expect(cache.persistWorkerRequest({ ...request, body: { ...request.body, expected_version: 2 } })).rejects.toThrow("different worker operation");
    await cache.persistWorkerRequest({ ...request, receipt: workerReceipt() });
    expect((await cache.persistWorkerRequest(request)).receipt?.status).toBe("accepted");
    await cache.persistWorkerRequest({ ...request, scope: ["other-account", workerScope[1], workerScope[2]] });
    expect((await cache.workerRequests(workerAccount)).map((value) => value.scope[0])).toEqual([workerAccount]);
    await cache.purgeWorkerCache(workerAccount);
    expect(await cache.workerRequests(workerAccount)).toEqual([]);
    expect(await cache.workerRequests("other-account")).toHaveLength(1);
  });
});
