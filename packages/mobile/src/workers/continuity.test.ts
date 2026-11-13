import type { JobDetail, JobMutationReceipt, WorkerDetail, WorkerEvent, WorkerEventsResponse, WorkerMandateSpec, WorkerMutationReceipt, WorkerSummary, WorkerTargetsResponse } from "@apas/protocol";
import { acceptWorkerPage, eligibleWorkerTarget, exactWorkerRun, validateWorkerReceipt, workerSnapshot } from "./continuity";
import { parseWorkerLink, workerAttentionDestination } from "./links";

const worker = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const run = "28dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const question = "38dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const event = (seq: number): WorkerEvent => ({ worker_id: worker, project_id: "project", seq, state_version: seq, kind: "checkpoint", summary: "Inspection retained", job_id: null, run_id: run, question_id: null, created_at: "2026-01-01T00:00:00Z" });
const page = (events: WorkerEvent[], high_watermark: number): WorkerEventsResponse => ({ events, high_watermark, has_more: false, resync_required: false });

test("ordered worker catch-up rejects foreign project/worker, gaps and stale cursors", () => {
  expect(acceptWorkerPage("project", worker, 0, page([event(1), event(2)], 2))).toBe(2);
  expect(() => acceptWorkerPage("project", worker, 0, page([event(2)], 2))).toThrow();
  expect(() => acceptWorkerPage("project", worker, 0, page([{ ...event(1), worker_id: question }], 1))).toThrow();
  expect(() => acceptWorkerPage("project", worker, 0, page([{ ...event(1), project_id: "other" }], 1))).toThrow();
  expect(() => acceptWorkerPage("project", worker, 2, page([], 1))).toThrow();
  expect(() => acceptWorkerPage("project", worker, 0, { ...page([], 1), has_more: true })).toThrow();
  expect(() => acceptWorkerPage("project", worker, 0, { ...page([], 0), resync_required: true })).toThrow();
});

test("atomic snapshot requires exact accepted watermark and bounds event memory", () => {
  const detail = { worker: { id: worker, high_watermark: 250 } } as WorkerDetail;
  const events = Array.from({ length: 250 }, (_, index) => event(index + 1));
  expect(() => workerSnapshot(detail, events, 249)).toThrow();
  const snapshot = workerSnapshot(detail, events, 250);
  expect(snapshot.cursor).toBe(250);
  expect(snapshot.events).toHaveLength(200);
  expect(snapshot.events[0].seq).toBe(51);
});

test("exact historical execution never defaults to latest run or another worker", () => {
  const detail = { task: { worker: { worker_id: worker } }, job: { id: "job", current_run_id: question }, runs: [{ id: run, job_id: "job" }, { id: question, job_id: "job" }] } as JobDetail;
  expect(exactWorkerRun(detail, worker, run)).toBe(detail);
  expect(() => exactWorkerRun(detail, question, run)).toThrow();
  expect(() => exactWorkerRun(detail, worker, worker)).toThrow();
});

test("exact target selection does not silently fall back and recurring permission is independent", () => {
  const spec = { machine_id: "chosen", worker_profile: "isolated", model_profile: "model", verification_profile: "verify" } as WorkerMandateSpec;
  const targets = { project_id: "project", unavailable_reasons: [], targets: [{ machine_id: "chosen", available: true, allowed_actions: { admit: true }, policy: { policy: { enabled: true, worker_profiles: ["isolated"], model_profiles: ["model"], verification_profiles: ["verify"], recurring: { enabled: true } } }, profiles: [{ key: "isolated", ready: true, model_profiles: [{ key: "model" }], verification_profiles: [{ key: "verify" }] }] }] } as unknown as WorkerTargetsResponse;
  expect(eligibleWorkerTarget(targets, spec)).toBe(true);
  expect(eligibleWorkerTarget(targets, { ...spec, machine_id: "missing" })).toBe(false);
  expect(eligibleWorkerTarget(targets, { ...spec, model_profile: "other-provider" })).toBe(false);
  targets.targets[0].policy.policy.model_profiles = [];
  expect(eligibleWorkerTarget(targets, spec)).toBe(false);
  targets.targets[0].policy.policy.model_profiles = ["model"];
  const recurring = targets.targets[0].policy.policy.recurring;
  if (!recurring) throw new Error("The allowed-policy fixture must include recurring permission");
  recurring.enabled = false;
  expect(eligibleWorkerTarget(targets, spec)).toBe(false);
  delete targets.targets[0].policy.policy.recurring;
  expect(eligibleWorkerTarget(targets, spec)).toBe(false);
});

test("worker deep links preserve run/question identity and cannot invoke lifecycle operations", () => {
  expect(parseWorkerLink(`apas://code/workers/${worker}/runs/${run}/questions/${question}`)).toEqual({ worker, run, question, review: false });
  expect(parseWorkerLink(`https://apas.mpaxos.com/code/workers/${worker}/runs/${run}/review`)).toEqual({ worker, run, review: true });
  expect(parseWorkerLink(`apas://code/workers/${worker}`)).toEqual({ worker, review: false });
  for (const link of [`apas://code/workers/${worker}/resume`, `apas://code/workers/${worker}/review`, `apas://code/workers/${worker}/runs/missing/review`, `https://evil.invalid/code/workers/${worker}`, `apas://code/session/${worker}`, `apas://code/workers/${worker}/runs/${run}/questions/${question}/answers`]) expect(parseWorkerLink(link)).toBeNull();
});

test("worker receipt recovery and execution receipt recovery retain their distinct journals", () => {
  const lifecycle: WorkerMutationReceipt = { request_id: "pause-intent", operation: "pause", status: "accepted", project_id: "project", worker_id: worker, state_version: 3, error_code: null };
  expect(() => validateWorkerReceipt(lifecycle, "pause-intent", "project", worker)).not.toThrow();
  expect(() => validateWorkerReceipt(lifecycle, "different-intent", "project", worker)).toThrow();
  expect(() => validateWorkerReceipt(lifecycle, "pause-intent", "other-project", worker)).toThrow();
  const answer: JobMutationReceipt = { request_id: "answer-intent", operation: "answer", status: "accepted", job_id: "job", run_id: run };
  expect(() => validateWorkerReceipt(answer, "answer-intent", "project", worker, run)).not.toThrow();
  expect(() => validateWorkerReceipt(answer, "answer-intent", "project", worker, question)).toThrow();
});

test("Attention destinations use server-selected historical identities, never the latest active run", () => {
  const summary = { id: worker, project_id: "project", state: "waiting_review", attention_run_id: run, attention_question_id: null, active_run_id: "later-run" } as WorkerSummary;
  expect(workerAttentionDestination(summary)).toEqual({ pathname: "/(code)/workers/[workerId]/runs/[runId]/review", params: { workerId: worker, project: "project", runId: run } });
  expect(workerAttentionDestination({ ...summary, attention_question_id: question }).params).toEqual({ workerId: worker, project: "project", runId: run, questionId: question });
  expect(workerAttentionDestination({ ...summary, attention_run_id: null }).pathname).toBe("/(code)/workers/[workerId]");
});
