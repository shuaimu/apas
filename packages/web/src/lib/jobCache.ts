import type { JobDetail, JobEvent, JobEventsResponse, JobMutationReceipt, JobAdmissionRequest, JobAnswerRequest, JobMutationRequest } from "@apas/protocol";

export type JobScope = readonly [account: string, project: string, job: string];
export interface SavedJob { detail: JobDetail; events: JobEvent[]; cursor: number; historyResynced?: boolean }
export interface SavedJobRequest {
  id: string;
  scope: JobScope;
  path: string;
  body: JobAdmissionRequest | JobAnswerRequest | JobMutationRequest;
  receipt?: JobMutationReceipt;
  createdAt: string;
}
let database: Promise<IDBDatabase> | undefined;
let revision = 0;
let purgeBarrier: Promise<void> = Promise.resolve();
let purgeFailed = false;
const listeners = new Set<() => void>();
export const jobCacheRevision = () => revision;
export function onJobCachePurge(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; }
function db() {
  if (!database) database = new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open("apas-autonomous-jobs", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("records");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => { database = undefined; reject(request.error); };
    request.onblocked = () => { database = undefined; reject(new Error("Close other APAS tabs to unlock job storage.")); };
  });
  return database;
}
async function readableDatabase() {
  await purgeBarrier;
  if (purgeFailed) throw new Error("Job cache purge failed. Clear this site's data.");
  return db();
}
const key = (scope: JobScope, kind: string, id = "") => [...scope, kind, id];
export async function readJobRecord<T>(scope: JobScope, kind: string, id = ""): Promise<T | undefined> {
  const expectedRevision = revision;
  const database = await readableDatabase();
  if (revision !== expectedRevision) return undefined;
  return new Promise((resolve, reject) => {
    const request = database.transaction("records").objectStore("records").get(key(scope, kind, id));
    request.onsuccess = () => resolve(revision === expectedRevision ? request.result as T | undefined : undefined);
    request.onerror = () => reject(request.error);
  });
}
export async function writeJobRecord<T>(scope: JobScope, kind: string, value: T, id = "", expectedRevision = revision): Promise<void> {
  const database = await readableDatabase();
  if (revision !== expectedRevision) throw new Error("Job access changed. Reload before saving.");
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records", "readwrite");
    tx.objectStore("records").put(value, key(scope, kind, id));
    tx.oncomplete = () => resolve();
    tx.onabort = tx.onerror = () => reject(tx.error ?? new Error("Job draft could not be saved."));
  });
}
/** Event bytes and their cursor are one record in one committed transaction. */
export async function commitJobEvents(scope: JobScope, detail: JobDetail, page: JobEventsResponse, expectedRevision = revision): Promise<SavedJob> {
  const database = await readableDatabase();
  if (revision !== expectedRevision) throw new Error("Job access changed.");
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records", "readwrite");
    const store = tx.objectStore("records");
    const request = store.get(key(scope, "snapshot"));
    let result: SavedJob;
    let failure: Error | undefined;
    request.onsuccess = () => {
      const previous = request.result as SavedJob | undefined;
      let cursor = previous?.cursor ?? 0;
      const events = [...(previous?.events ?? [])];
      for (const event of page.events) {
        if (event.job_id !== scope[2]) { failure = new Error("Mismatched job replay."); tx.abort(); return; }
        if (event.seq <= cursor) continue;
        if (event.seq !== cursor + 1) { failure = new Error("Job event gap; refresh to resynchronize."); tx.abort(); return; }
        events.push(event); cursor = event.seq;
      }
      if (page.next_seq !== cursor) { failure = new Error("Job cursor does not match committed events."); tx.abort(); return; }
      result = { detail, events: events.slice(-200), cursor, historyResynced: previous?.historyResynced };
      store.put(result, key(scope, "snapshot"));
    };
    tx.oncomplete = () => resolve(result);
    tx.onabort = tx.onerror = () => reject(failure ?? tx.error ?? new Error("Could not commit job replay."));
  });
}
export async function jobRequests(account: string): Promise<SavedJobRequest[]> {
  const expectedRevision = revision;
  const database = await readableDatabase();
  if (revision !== expectedRevision) return [];
  return new Promise((resolve, reject) => {
    const tx = database.transaction("records");
    const request = tx.objectStore("records").openCursor();
    const values: SavedJobRequest[] = [];
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) return;
      const parts = cursor.key as string[];
      if (parts[0] === account && parts[3] === "request") values.push(cursor.value as SavedJobRequest);
      cursor.continue();
    };
    tx.oncomplete = () => resolve(revision === expectedRevision ? values.sort((a, b) => b.createdAt.localeCompare(a.createdAt)) : []);
    tx.onabort = tx.onerror = () => reject(tx.error);
  });
}
export async function purgeJobCache(account?: string, project?: string): Promise<void> {
  const previous = purgeBarrier;
  const purge = (async () => {
    // Publish the barrier before notifying listeners: a remounted scope must not
    // restore records while this deletion is still waiting for IndexedDB.
    await previous.catch(() => undefined);
    try {
      const database = await db();
      await new Promise<void>((resolve, reject) => {
        const tx = database.transaction("records", "readwrite");
        const store = tx.objectStore("records");
        const request = store.openCursor();
        request.onsuccess = () => {
          const cursor = request.result;
          if (!cursor) return;
          const parts = cursor.key as string[];
          // Cross-project inbox pages can contain revoked project content too.
          if ((!account || parts[0] === account) && (!project || parts[1] === project || parts[1] === "")) cursor.delete();
          cursor.continue();
        };
        tx.oncomplete = () => resolve();
        tx.onabort = tx.onerror = () => reject(tx.error);
      });
      if (!account && !project) purgeFailed = false;
    } catch (failure) {
      // A later unrelated scoped purge cannot make failed-to-delete bytes safe.
      purgeFailed = true;
      throw failure;
    }
  })();
  purgeBarrier = purge;
  revision += 1;
  listeners.forEach((listener) => listener());
  await purge;
}
