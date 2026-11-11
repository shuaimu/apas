import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { JobDetail, JobEvent, JobEventsResponse } from "@apas/protocol";
import type { JobScope, SavedJob, SavedJobRequest } from "./jobCache";
import * as cache from "./jobCache";

const owner = "7c6d2400-7b4b-455d-b732-7f066dab7a24";
const job = "06974c2c-cd7d-446b-aac3-e0f550e739f6";
const run = "37642b7c-2d81-46c6-976f-2874e85919e6";
const scope: JobScope = [owner, "project-a", job];
const stamp = "2026-11-08T00:00:00Z";

function detail(version = 1): JobDetail {
  const usage = { elapsed_seconds: 10, input_tokens: 50, output_tokens: 10, spend_micro_usd: 5, estimated: true };
  return {
    job: {
      id: job, project_id: scope[1], project_name: "Project A", title: "Bounded task",
      state: "running", state_version: version, current_run_id: run,
      cluster_owner_user_id: owner, machine_id: "86801878-e6ab-413f-8ad0-e14c2c2a634d", hostname: "test-host",
      created_at: stamp, updated_at: stamp, usage,
      limits: { elapsed_seconds: 120, spend_micro_usd: 10_000, attempts: 1, cpu_millis: 1000,
        memory_bytes: 536_870_912, workspace_bytes: 67_108_864, output_bytes: 4096,
        pids: 16, model_concurrency: 2, model_output_tokens: 1024 },
      attention_count: 0, reason: null,
      allowed_actions: { admit: false, answer: false, cancel: true, retry: false, retry_publication: false, manage_policy: false },
    },
    task: { instruction: "Add a bounded counter", acceptance_criteria: ["Counter rejects overflow"], source_url: null,
      repository: "example/project", base_ref: "main", base_sha: "a".repeat(40), worker_profile: "worker",
      worker_profile_digest: "sha256:" + "b".repeat(64), verification_profile: "checks",
      verification_profile_digest: "sha256:" + "c".repeat(64), model_profile: "model" },
    runs: [{ id: run, job_id: job, attempt: 1, epoch: 1, state: "running", usage, started_at: stamp,
      ended_at: null, worker_version: "18.18.0", candidate_sha: null, stop_evidence: null, reason: null }],
    questions: [], high_watermark: version, policy_digest: "sha256:" + "d".repeat(64), publication: null,
  };
}
function event(seq: number, jobId = job): JobEvent {
  return { id: `00000000-0000-4000-8000-${String(seq).padStart(12, "0")}`, job_id: jobId, run_id: run,
    seq, kind: "progress", summary: `Progress ${seq}`, created_at: stamp, detail: null };
}
function page(events: JobEvent[], nextSeq = events.at(-1)?.seq ?? 0): JobEventsResponse {
  return { events, next_seq: nextSeq, high_watermark: nextSeq, has_more: false, resync_required: false };
}

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(() => cache.purgeJobCache());
afterAll(() => { vi.unstubAllGlobals(); });

describe("durable job replay", () => {
  it("keeps the old snapshot and cursor when an event page has a gap", async () => {
    await cache.commitJobEvents(scope, detail(), page([event(1)]));
    await expect(cache.commitJobEvents(scope, detail(4), page([event(2), event(4)]))).rejects.toBeInstanceOf(Error);
    const retained = await cache.readJobRecord<SavedJob>(scope, "snapshot");
    expect(retained?.cursor).toBe(1);
    expect(retained?.events.map((value) => value.seq)).toEqual([1]);
    expect(retained?.detail.job.state_version).toBe(1);
    await cache.commitJobEvents(scope, detail(3), page([event(2), event(3)]));
    expect((await cache.readJobRecord<SavedJob>(scope, "snapshot"))?.cursor).toBe(3);
  });

  it("deduplicates overlapping replay while committing the new suffix", async () => {
    await cache.commitJobEvents(scope, detail(), page([event(1)]));
    await cache.commitJobEvents(scope, detail(2), page([event(1), event(2)]));
    const retained = await cache.readJobRecord<SavedJob>(scope, "snapshot");
    expect(retained?.events.map((value) => value.seq)).toEqual([1, 2]);
    expect(retained?.cursor).toBe(2);
  });

  it.each([
    ["foreign job", () => page([event(2, "97d974f1-84e7-4547-ad81-85c74f78c5a7")])],
    ["cursor beyond the accepted events", () => page([event(2)], 3)],
  ] as const)("rejects a %s without advancing durable state", async (_name, invalidPage) => {
    await cache.commitJobEvents(scope, detail(), page([event(1)]));
    await expect(cache.commitJobEvents(scope, detail(3), invalidPage())).rejects.toBeInstanceOf(Error);
    expect((await cache.readJobRecord<SavedJob>(scope, "snapshot"))?.cursor).toBe(1);
  });
});

describe("job account and access isolation", () => {
  it("purges revoked project data and cross-project inboxes without deleting unrelated account data", async () => {
    const inbox: JobScope = [owner, "", ""];
    const otherProject: JobScope = [owner, "project-b", "other-job"];
    const otherAccount: JobScope = ["other-account", scope[1], scope[2]];
    for (const target of [scope, inbox, otherProject, otherAccount]) {
      await cache.writeJobRecord(target, "draft", { account: target[0], project: target[1] });
    }
    await cache.purgeJobCache(owner, scope[1]);
    expect(await cache.readJobRecord(scope, "draft")).toBeUndefined();
    expect(await cache.readJobRecord(inbox, "draft")).toBeUndefined();
    expect(await cache.readJobRecord(otherProject, "draft")).toEqual({ account: owner, project: "project-b" });
    expect(await cache.readJobRecord(otherAccount, "draft")).toEqual({ account: "other-account", project: scope[1] });
  });

  it("rejects a delayed pre-revocation write instead of resurrecting purged task content", async () => {
    const beforeRevocation = cache.jobCacheRevision();
    await cache.writeJobRecord(scope, "draft", { instruction: "Private task" });
    await cache.purgeJobCache(owner);
    await expect(cache.writeJobRecord(scope, "draft", { instruction: "Stale private task" }, "", beforeRevocation)).rejects.toBeInstanceOf(Error);
    expect(await cache.readJobRecord(scope, "draft")).toBeUndefined();
  });

  it("blocks remount reads started by purge listeners until revoked records are deleted", async () => {
    await cache.writeJobRecord(scope, "draft", { instruction: "Private task" });
    const preferences: JobScope = [owner, scope[1], ""];
    await cache.writeJobRecord(preferences, "draft", { open: [job] }, "project-workspace");
    let restored: Promise<unknown> | undefined;
    let restoredPreferences: Promise<unknown> | undefined;
    const remove = cache.onJobCachePurge(() => {
      restored = cache.readJobRecord(scope, "draft");
      restoredPreferences = cache.readJobRecord(preferences, "draft", "project-workspace");
    });
    try {
      await cache.purgeJobCache(owner, scope[1]);
      expect(await restored).toBeUndefined();
      expect(await restoredPreferences).toBeUndefined();
    } finally { remove(); }
  });

  it("discards reads started before revocation and orders new-revision writes after deletion", async () => {
    await cache.writeJobRecord(scope, "draft", { instruction: "Old task" });
    const staleRead = cache.readJobRecord(scope, "draft");
    let saved: Promise<void> | undefined;
    const remove = cache.onJobCachePurge(() => {
      saved = cache.writeJobRecord(scope, "draft", { instruction: "New authorized draft" });
    });
    try {
      await cache.purgeJobCache(owner, scope[1]);
      await saved;
      expect(await staleRead).toBeUndefined();
      expect(await cache.readJobRecord(scope, "draft")).toEqual({ instruction: "New authorized draft" });
    } finally { remove(); }
  });

  it("recovers a request only within its actor even when request IDs collide", async () => {
    const id = "e69df5ac-6b5f-489e-bdba-badbf9e4c3e8";
    const otherAccount: JobScope = ["other-account", scope[1], scope[2]];
    for (const target of [scope, otherAccount]) {
      const request: SavedJobRequest = { id, scope: target, path: `/projects/${target[1]}/jobs/${job}/cancel`,
        body: { request_id: id, expected_version: 1 }, createdAt: stamp };
      await cache.writeJobRecord(target, "request", request, id);
    }
    expect((await cache.jobRequests(owner)).map((request) => request.scope[0])).toEqual([owner]);
    await cache.purgeJobCache(owner);
    expect(await cache.jobRequests(owner)).toEqual([]);
    expect((await cache.jobRequests("other-account")).map((request) => request.scope[0])).toEqual(["other-account"]);
  });
});
