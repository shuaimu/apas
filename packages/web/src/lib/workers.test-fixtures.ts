import type { WorkerDetail, WorkerEvent, WorkerEventsResponse, WorkerMutationReceipt, WorkerSummary } from "@apas/protocol";
import type { SavedWorkerRequest, WorkerScope } from "./workerCache";

export const workerAccount = "7c6d2400-7b4b-455d-b732-7f066dab7a24";
export const workerId = "06974c2c-cd7d-446b-aac3-e0f550e739f6";
export const workerScope: WorkerScope = [workerAccount, "project-a", workerId];
export const workerStamp = "2026-11-08T00:00:00Z";
export function workerSummary(version = 1, project = workerScope[1], id = workerId): WorkerSummary {
  return {
    id, project_id: project, title: "Repository maintenance", description: "Inspect the standing checklist.\n保留完整说明。",
    state: "idle", reason: null, desired_state: "active", mandate_revision: 1, backlog_revision: 1,
    state_version: version, high_watermark: version, next_check_at: workerStamp,
    active_job_id: null, active_run_id: null, current_item_id: null,
    machine_id: "86801878-e6ab-413f-8ad0-e14c2c2a634d", cluster_owner_user_id: workerAccount,
    hostname: "worker-host", attention: false, created_at: workerStamp, updated_at: workerStamp,
  };
}
export function workerDetail(version = 1): WorkerDetail {
  return {
    worker: workerSummary(version), checkpoint: null, current_job: null,
    mandate: {
      revision: 1, repository: "example/project", worker_profile_digest: "sha256:" + "a".repeat(64),
      verification_profile_digest: "sha256:" + "b".repeat(64), policy_digest: "sha256:" + "c".repeat(64),
      approved_by: workerAccount, approved_at: workerStamp,
      spec: {
        title: "Repository maintenance", description: "Inspect the standing checklist.", checklist: ["Find missing boundary tests"],
        machine_id: "86801878-e6ab-413f-8ad0-e14c2c2a634d", worker_profile: "worker", verification_profile: "checks",
        model_profile: "model", base_ref: "main", interval_seconds: 900,
        limits: {
          per_run: { elapsed_seconds: 120, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000,
            memory_bytes: 536_870_912, workspace_bytes: 67_108_864, output_bytes: 4096, pids: 16,
            model_concurrency: 2, model_output_tokens: 1024 },
          elapsed_seconds_24h: 7200, spend_micro_usd_24h: 20_000_000, runs_24h: 24, attempts_per_item: 1,
        },
      },
    },
    budget: {
      usage: { elapsed_seconds: 1, input_tokens: 1, output_tokens: 1, spend_micro_usd: 1, estimated: false },
      runs: 1, reserved_elapsed_seconds: 0, reserved_spend_micro_usd: 0, reserved_runs: 0,
      accounting_uncertain: false, eligible_at: null, window_started_at: workerStamp,
    },
    allowed_actions: { edit_mandate: true, edit_backlog: true, pause: true, stop: false, resume: false, wake: true, retry: false, answer: false, retry_publication: false },
  };
}
export function workerEvent(seq: number): WorkerEvent {
  return { worker_id: workerId, project_id: workerScope[1], seq, state_version: seq, kind: "state_changed", summary: `State ${seq}`, job_id: null, run_id: null, question_id: null, created_at: workerStamp };
}
export function workerPage(events: WorkerEvent[], highWatermark = events.at(-1)?.seq ?? 0, hasMore = false): WorkerEventsResponse {
  return { events, high_watermark: highWatermark, has_more: hasMore, resync_required: false };
}
export function workerRequest(): SavedWorkerRequest {
  const id = "e69df5ac-6b5f-489e-bdba-badbf9e4c3e8";
  return { id, scope: workerScope, path: `/projects/${workerScope[1]}/workers/${workerId}/pause`, body: { request_id: id, expected_version: 1 }, createdAt: workerStamp };
}
export function workerReceipt(): WorkerMutationReceipt {
  return { request_id: workerRequest().id, operation: "pause", status: "accepted", worker_id: workerId, project_id: workerScope[1], state_version: 2, error_code: null };
}
