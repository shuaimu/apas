import type { JobDetail, JobEvent, JobEventsResponse } from "@apas/protocol";
import { openCache } from "@/storage/cache";
import { acceptJobPage, boundedEvents } from "./continuity";

export interface JobScope { account: string; project: string; job: string }
export interface JobSnapshot { detail: JobDetail; events: JobEvent[]; cursor: number }
let queue: Promise<unknown> = Promise.resolve();
let generation = 0;

// All writes/purges are serialized. A response from before a purge cannot restore content.
let storageAccount: string | null = null;
const revokedProjects = new Set<string>();

export function setJobStorageAccount(account: string | null): void {
  if (storageAccount !== account) {
    generation++;
    revokedProjects.clear();
    storageAccount = account;
  }
}

export function authorizeJobStorageProject(account: string, project: string): void {
  if (account === storageAccount) revokedProjects.delete(project);
}

function requireStorageAuthority(scope: JobScope): void {
  if (scope.account && (scope.account !== storageAccount || revokedProjects.has(scope.project))) {
    throw new Error("Job cache authority changed");
  }
}
function serial<T>(work: () => Promise<T>): Promise<T> {
  const result = queue.then(work, work);
  queue = result.catch(() => undefined);
  return result;
}

export function writeJobRecord(scope: JobScope, key: string, value: unknown): Promise<void> {
  const epoch = generation;
  return serial(async () => {
    if (epoch !== generation) throw new Error("Job cache authority changed");
    const db = await openCache();
    requireStorageAuthority(scope);
    await db.runAsync("INSERT OR REPLACE INTO job_records(account_id, project_id, job_id, record_key, payload) VALUES (?, ?, ?, ?, ?)", scope.account, scope.project, scope.job, key, JSON.stringify(value));
  });
}

export async function readJobRecord<T>(scope: JobScope, key: string): Promise<T | null> {
  const epoch = generation;
  await queue;
  requireStorageAuthority(scope);
  const db = await openCache();
  const row = await db.getFirstAsync<{ payload: string }>("SELECT payload FROM job_records WHERE account_id=? AND project_id=? AND job_id=? AND record_key=?", scope.account, scope.project, scope.job, key);
  if (epoch !== generation) throw new Error("Job cache authority changed");
  requireStorageAuthority(scope);
  return row ? JSON.parse(row.payload) as T : null;
}

export function commitJobSnapshot(scope: JobScope, detail: JobDetail, page?: JobEventsResponse, reset = false): Promise<JobSnapshot> {
  const epoch = generation;
  return serial(async () => {
    if (epoch !== generation) throw new Error("Job cache authority changed");
    requireStorageAuthority(scope);
    if (detail.job.id !== scope.job || detail.job.project_id !== scope.project) throw new Error("Job snapshot identity mismatch");
    const db = await openCache();
    let result: JobSnapshot = { detail, events: [], cursor: reset ? detail.high_watermark : 0 };
    await db.withExclusiveTransactionAsync(async (tx) => {
      const row = await tx.getFirstAsync<{ payload: string }>("SELECT payload FROM job_records WHERE account_id=? AND project_id=? AND job_id=? AND record_key='snapshot'", scope.account, scope.project, scope.job);
      const prior = row ? JSON.parse(row.payload) as JobSnapshot : null;
      if (page) {
        if (!prior) throw new Error("Missing job snapshot");
        result = { detail, cursor: acceptJobPage(scope.job, prior.cursor, page), events: boundedEvents(prior.events, page.events) };
      } else if (prior) {
        result.events = prior.events.filter((event) => event.seq <= detail.high_watermark);
        result.cursor = reset ? detail.high_watermark : prior.cursor;
      }
      if (epoch !== generation) throw new Error("Job cache authority changed");
      // Snapshot, accepted event window and cursor are a single atomic encrypted row.
      await tx.runAsync("INSERT OR REPLACE INTO job_records(account_id,project_id,job_id,record_key,payload) VALUES (?,?,?,'snapshot',?)", scope.account, scope.project, scope.job, JSON.stringify(result));
    });
    return result;
  });
}

export function purgeJobStorage(account?: string, project?: string): Promise<void> {
  generation++;
  if (project) revokedProjects.add(project);
  else storageAccount = null;
  return serial(async () => {
    const db = await openCache();
    if (account && project) {
      await db.runAsync("DELETE FROM job_records WHERE account_id=? AND project_id=?", account, project);
      // Global worker pages contain protected labels from multiple projects.
      // Drop all such pages on a discovered revocation, not just the visible inbox.
      await db.runAsync("DELETE FROM job_records WHERE account_id=? AND project_id='' AND job_id LIKE 'worker:%'", account);
    }
    else if (account) await db.runAsync("DELETE FROM job_records WHERE account_id=?", account);
    else await db.runAsync("DELETE FROM job_records");
  });
}

export async function jobCachedProjects(account: string): Promise<string[]> {
  const db = await openCache();
  return (await db.getAllAsync<{ project_id: string }>("SELECT DISTINCT project_id FROM job_records WHERE account_id=? AND project_id<>''", account)).map((row) => row.project_id);
}

export async function drainJobStorage(): Promise<void> { generation++; await queue; }
