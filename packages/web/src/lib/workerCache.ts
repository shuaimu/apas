import type { WorkerApprovalRequest, WorkerDetail, WorkerEvent, WorkerEventsResponse, WorkerItemRequest, WorkerMutationReceipt, WorkerMutationRequest } from "@apas/protocol";
import { jobCacheRevision, onJobCachePurge, purgeJobCache, readJobRecord, writeJobRecord } from "./jobCache";

export type WorkerScope = readonly [account: string, project: string, worker: string];
export interface SavedWorker { detail: WorkerDetail; events: WorkerEvent[]; cursor: number; historyResynced?: boolean }
export interface SavedWorkerRequest {
  id: string;
  scope: WorkerScope;
  path: string;
  body: WorkerApprovalRequest | WorkerItemRequest | WorkerMutationRequest;
  receipt?: WorkerMutationReceipt;
  createdAt: string;
}

// Worker and execution evidence share one deletion transaction and revocation generation.
// Prefix record kinds, not scope IDs: project revocation must include both products.
export const workerCacheRevision = jobCacheRevision;
export const onWorkerCachePurge = onJobCachePurge;
export const purgeWorkerCache = purgeJobCache;
const kindKey = (kind: string) => `worker:${kind}`;
const key = (scope: WorkerScope, kind: string, id = "") => [...scope, kindKey(kind), id];
export function readWorkerRecord<T>(scope: WorkerScope, kind: string, id = ""): Promise<T | undefined> {
  return readJobRecord<T>(scope, kindKey(kind), id);
}
export async function writeWorkerRecord<T>(scope: WorkerScope, kind: string, value: T, id = "", expectedRevision = workerCacheRevision()): Promise<void> {
  await writeJobRecord(scope, kindKey(kind), value, id, expectedRevision);
  assertRevision(expectedRevision);
}
function assertRevision(expected: number) {
  if (workerCacheRevision() !== expected) throw new Error("Worker access changed. Refresh before continuing.");
}
let database: Promise<IDBDatabase> | undefined;
async function transactionDatabase(scope: WorkerScope, expectedRevision: number) {
  // readJobRecord waits for the shared purge barrier and fails closed after a failed deletion.
  await readJobRecord(scope, kindKey("barrier"));
  assertRevision(expectedRevision);
  if (!database) database = new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open("apas-autonomous-jobs", 1);
    request.onupgradeneeded = () => { if (!request.result.objectStoreNames.contains("records")) request.result.createObjectStore("records"); };
    request.onsuccess = () => {
      request.result.onversionchange = () => { request.result.close(); database = undefined; };
      resolve(request.result);
    };
    request.onerror = () => { database = undefined; reject(request.error); };
    request.onblocked = () => { database = undefined; reject(new Error("Close other APAS tabs to unlock worker storage.")); };
  });
  const result = await database;
  assertRevision(expectedRevision);
  return result;
}
function assertDetail(scope: WorkerScope, detail: WorkerDetail) {
  if (detail.worker.id !== scope[2] || detail.worker.project_id !== scope[1]) throw new Error("Mismatched worker snapshot.");
}

/** The detail, bounded event suffix and acknowledged cursor always commit together. */
export async function commitWorkerEvents(scope: WorkerScope, detail: WorkerDetail, page: WorkerEventsResponse, expectedRevision = workerCacheRevision()): Promise<SavedWorker> {
  assertDetail(scope, detail);
  if (page.resync_required) throw new Error("Worker history requires a fresh authorized snapshot.");
  if (page.events.length > 100) throw new Error("Worker replay exceeded the requested page limit.");
  const database = await transactionDatabase(scope, expectedRevision);
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records", "readwrite");
    const store = tx.objectStore("records");
    const request = store.get(key(scope, "snapshot"));
    let result: SavedWorker;
    let failure: unknown;
    request.onsuccess = () => {
      try {
        assertRevision(expectedRevision);
        const previous = request.result as SavedWorker | undefined;
        let cursor = previous?.cursor ?? 0;
        const events = [...(previous?.events ?? [])];
        let lastSequence = -1;
        for (const event of page.events) {
          if (event.worker_id !== scope[2] || event.project_id !== scope[1]) throw new Error("Mismatched worker replay.");
          if (!Number.isSafeInteger(event.seq) || event.seq < 1 || event.seq <= lastSequence) throw new Error("Worker events are not ordered.");
          lastSequence = event.seq;
          if (event.seq <= cursor) continue;
          if (event.seq !== cursor + 1) throw new Error("Worker event gap; refresh to resynchronize.");
          events.push(event);
          cursor = event.seq;
        }
        if (!Number.isSafeInteger(page.high_watermark) || page.high_watermark < cursor || (!page.has_more && page.high_watermark !== cursor)) throw new Error("Worker watermark does not match the replayed events.");
        if (page.has_more && (cursor === (previous?.cursor ?? 0) || cursor >= page.high_watermark)) throw new Error("Worker replay did not advance.");
        const snapshot = previous && (previous.detail.worker.state_version > detail.worker.state_version || previous.detail.worker.high_watermark > detail.worker.high_watermark) ? previous.detail : detail;
        result = { detail: snapshot, events: events.slice(-200), cursor, historyResynced: previous?.historyResynced };
        store.put(result, key(scope, "snapshot"));
      } catch (error) { failure = error; tx.abort(); }
    };
    tx.oncomplete = () => { try { assertRevision(expectedRevision); resolve(result); } catch (error) { reject(error); } };
    tx.onabort = tx.onerror = () => reject(failure ?? tx.error ?? new Error("Could not commit worker replay."));
  });
}

/** Retention reset and final catch-up use the same atomic snapshot/cursor record. */
export async function commitWorkerSnapshot(scope: WorkerScope, detail: WorkerDetail, resetHistory = false, expectedRevision = workerCacheRevision()): Promise<SavedWorker> {
  assertDetail(scope, detail);
  const database = await transactionDatabase(scope, expectedRevision);
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records", "readwrite");
    const store = tx.objectStore("records");
    const request = store.get(key(scope, "snapshot"));
    let result: SavedWorker;
    let failure: unknown;
    request.onsuccess = () => {
      try {
        assertRevision(expectedRevision);
        const previous = request.result as SavedWorker | undefined;
        if (previous && (detail.worker.state_version < previous.detail.worker.state_version || detail.worker.high_watermark < previous.cursor)) throw new Error("Worker snapshot regressed. Refresh current state.");
        if (!resetHistory && detail.worker.high_watermark !== (previous?.cursor ?? 0)) throw new Error("Worker history has not caught up to the snapshot.");
        result = resetHistory
          ? { detail, events: [], cursor: detail.worker.high_watermark, historyResynced: true }
          : { detail, events: previous?.events ?? [], cursor: previous?.cursor ?? 0, historyResynced: previous?.historyResynced };
        store.put(result, key(scope, "snapshot"));
      } catch (error) { failure = error; tx.abort(); }
    };
    tx.oncomplete = () => { try { assertRevision(expectedRevision); resolve(result); } catch (error) { reject(error); } };
    tx.onabort = tx.onerror = () => reject(failure ?? tx.error ?? new Error("Could not commit worker snapshot."));
  });
}

export async function workerRequests(account: string): Promise<SavedWorkerRequest[]> {
  const revision = workerCacheRevision();
  const database = await transactionDatabase([account, "", ""], revision);
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records");
    const request = tx.objectStore("records").openCursor();
    const values: SavedWorkerRequest[] = [];
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) return;
      const parts = cursor.key as string[];
      if (parts[0] === account && parts[3] === kindKey("request")) values.push(cursor.value as SavedWorkerRequest);
      cursor.continue();
    };
    tx.oncomplete = () => resolve(workerCacheRevision() === revision ? values.sort((a, b) => b.createdAt.localeCompare(a.createdAt)) : []);
    tx.onabort = tx.onerror = () => reject(tx.error);
  });
}

/** A saved ID cannot be reassigned to another payload, including after an uncertain POST. */
export async function persistWorkerRequest(value: SavedWorkerRequest, expectedRevision = workerCacheRevision()): Promise<SavedWorkerRequest> {
  if (value.id !== value.body.request_id) throw new Error("Worker request identity does not match its body.");
  const database = await transactionDatabase(value.scope, expectedRevision);
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records", "readwrite");
    const store = tx.objectStore("records");
    const request = store.get(key(value.scope, "request", value.id));
    let result: SavedWorkerRequest;
    let failure: unknown;
    request.onsuccess = () => {
      try {
        assertRevision(expectedRevision);
        const previous = request.result as SavedWorkerRequest | undefined;
        if (previous && (previous.path !== value.path || JSON.stringify(previous.body) !== JSON.stringify(value.body))) throw new Error("This request ID already belongs to a different worker operation.");
        result = previous?.receipt && previous.receipt.status !== "pending" ? previous : { ...value, receipt: value.receipt ?? previous?.receipt };
        store.put(result, key(value.scope, "request", value.id));
      } catch (error) { failure = error; tx.abort(); }
    };
    tx.oncomplete = () => { try { assertRevision(expectedRevision); resolve(result); } catch (error) { reject(error); } };
    tx.onabort = tx.onerror = () => reject(failure ?? tx.error ?? new Error("Worker request could not be saved."));
  });
}
