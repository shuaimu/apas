import type { JobDetail, JobEvent } from "@apas/protocol";
import { commitJobSnapshot, purgeJobStorage, readJobRecord, setJobStorageAccount, writeJobRecord } from "./storage";

const mockRows = new Map<string, string>();
let mockFailWrite = false;
const mockDb = {
  getFirstAsync: jest.fn(async (_sql: string, ...args: string[]) => {
    const key = JSON.stringify(args.length === 3 ? [...args, "snapshot"] : args);
    const payload = mockRows.get(key);
    return payload ? { payload } : null;
  }),
  runAsync: jest.fn(async (sql: string, ...args: string[]) => {
    if (mockFailWrite) throw new Error("device storage full");
    if (sql.startsWith("DELETE")) {
      for (const key of mockRows.keys()) {
        const [account, project, job] = JSON.parse(key) as string[];
        const globalWorkerPage = sql.includes("job_id LIKE 'worker:%'");
        if (globalWorkerPage ? account === args[0] && project === "" && job.startsWith("worker:") : !args.length || (account === args[0] && (args.length === 1 || project === args[1]))) mockRows.delete(key);
      }
    } else {
      const key = JSON.stringify(args.length === 4 ? [...args.slice(0, 3), "snapshot"] : args.slice(0, 4));
      mockRows.set(key, args[args.length - 1]);
    }
    return { changes: 1 };
  }),
  withExclusiveTransactionAsync: jest.fn(async (work: (tx: unknown) => Promise<void>) => {
    const before = new Map(mockRows);
    try { await work(mockDb); } catch (error) { mockRows.clear(); for (const [key, value] of before) mockRows.set(key, value); throw error; }
  }),
};
jest.mock("@/storage/cache", () => ({ openCache: async () => mockDb }));
const scope = { account: "actor-a", project: "project", job: "job" };
const detail = { job: { id: "job", project_id: "project" }, high_watermark: 2 } as JobDetail;
const event: JobEvent = { id: "event", job_id: "job", run_id: null, seq: 1, kind: "progress", summary: "work", detail: null, created_at: "2026-01-01T00:00:00Z" };

beforeEach(async () => { mockFailWrite = false; await purgeJobStorage(); setJobStorageAccount(scope.account); });

test("failed event persistence does not advance the committed cursor", async () => {
  await commitJobSnapshot(scope, detail);
  mockFailWrite = true;
  await expect(commitJobSnapshot(scope, detail, { events: [event], next_seq: 1, high_watermark: 2, has_more: true, resync_required: false })).rejects.toThrow("storage full");
  mockFailWrite = false;
  const saved = await readJobRecord<{ cursor: number; events: JobEvent[] }>(scope, "snapshot");
  expect(saved?.cursor).toBe(0);
  expect(saved?.events).toEqual([]);
  const accepted = await commitJobSnapshot(scope, detail, { events: [event], next_seq: 1, high_watermark: 2, has_more: true, resync_required: false });
  expect(accepted.cursor).toBe(1);
  expect(accepted.events).toEqual([event]);
});

test("a newer snapshot does not skip events missing from the local journal", async () => {
  await commitJobSnapshot(scope, detail);
  const next = await commitJobSnapshot(scope, { ...detail, high_watermark: 50 });
  expect(next.cursor).toBe(0);
  const resync = await commitJobSnapshot(scope, { ...detail, high_watermark: 50 }, undefined, true);
  expect(resync.cursor).toBe(50);
});

test("drafts and uncertain identities are actor scoped and revoked writes cannot restore them", async () => {
  await writeJobRecord(scope, "answer", { request_id: "original", text: "private" });
  await expect(readJobRecord({ ...scope, account: "actor-b" }, "answer")).rejects.toThrow("authority changed");
  await purgeJobStorage(scope.account, scope.project);
  await expect(writeJobRecord(scope, "answer", { request_id: "original" })).rejects.toThrow("authority changed");
  await expect(readJobRecord(scope, "answer")).rejects.toThrow("authority changed");
});

test("account change fences already queued writes and full purge removes all evidence", async () => {
  await writeJobRecord(scope, "artifact:diff", "private diff");
  const pending = writeJobRecord(scope, "request", { request_id: "uncertain" });
  setJobStorageAccount("actor-b");
  await expect(pending).rejects.toThrow("authority changed");
  await purgeJobStorage();
  expect(mockRows.size).toBe(0);
});

test("worker scopes preserve exact run/question records and purge later global labels", async () => {
  const worker = { ...scope, job: "worker:responsibility" };
  await writeJobRecord(worker, "run:old:question:dialog:epoch:3", { text: "protected earlier answer" });
  await writeJobRecord(worker, "run:new:question:dialog:epoch:4", { text: "different answer" });
  await writeJobRecord({ ...scope, project: "", job: "worker:" }, "list:attention:later-page", { description: "protected label" });
  await writeJobRecord({ ...scope, project: "other", job: "worker:other" }, "snapshot", { title: "still authorized" });
  expect(await readJobRecord(worker, "run:old:question:dialog:epoch:3")).toEqual({ text: "protected earlier answer" });
  await purgeJobStorage(scope.account, scope.project);
  expect(await readJobRecord({ ...scope, project: "", job: "worker:" }, "list:attention:later-page")).toBeNull();
  expect(await readJobRecord({ ...scope, project: "other", job: "worker:other" }, "snapshot")).toEqual({ title: "still authorized" });
  await expect(writeJobRecord(worker, "run:old:artifact", "late response")).rejects.toThrow("authority changed");
});
