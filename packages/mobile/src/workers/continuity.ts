import type { WorkerDetail, WorkerEvent, WorkerEventsResponse, WorkerTargetsResponse, WorkerMandateSpec, JobDetail, WorkerMutationReceipt, JobMutationReceipt } from "@apas/protocol";

export function acceptWorkerPage(project: string, worker: string, cursor: number, page: WorkerEventsResponse): number {
  if (page.resync_required || page.high_watermark < cursor) throw new Error("Worker history requires resynchronization");
  let next = cursor;
  for (const event of page.events) {
    if (event.worker_id !== worker || event.project_id !== project || event.seq !== next + 1 || event.seq > page.high_watermark) throw new Error("Worker event identity or sequence mismatch");
    next = event.seq;
  }
  if (!page.has_more && next !== page.high_watermark) throw new Error("Worker event gap");
  if (page.has_more && next === cursor) throw new Error("Worker replay made no progress");
  return next;
}
export function workerSnapshot(detail: WorkerDetail, events: WorkerEvent[], cursor: number) {
  if (detail.worker.high_watermark !== cursor) throw new Error("Worker snapshot and event watermark differ");
  return { detail, events: events.slice(-200), cursor };
}
export function eligibleWorkerTarget(targets: WorkerTargetsResponse | undefined, spec: WorkerMandateSpec): boolean {
  const target = targets?.targets.find((value) => value.machine_id === spec.machine_id);
  const profile = target?.profiles.find((value) => value.key === spec.worker_profile);
  const policy = target?.policy.policy;
  return Boolean(target?.available && target.allowed_actions.admit && policy?.enabled && policy.recurring?.enabled
    && policy.worker_profiles.includes(spec.worker_profile) && policy.model_profiles.includes(spec.model_profile)
    && policy.verification_profiles.includes(spec.verification_profile) && profile?.ready
    && profile.model_profiles.some((value) => value.key === spec.model_profile)
    && profile.verification_profiles.some((value) => value.key === spec.verification_profile));
}
export function exactWorkerRun(detail: JobDetail, worker: string, run: string): JobDetail {
  if (detail.task.worker?.worker_id !== worker || !detail.runs.some((value) => value.id === run && value.job_id === detail.job.id)) throw new Error("The requested worker execution is not the returned execution");
  return detail;
}

export function validateWorkerReceipt(receipt: WorkerMutationReceipt | JobMutationReceipt, request: string, project: string, worker: string, run?: string): void {
  if (receipt.request_id !== request) throw new Error("Receipt request identity mismatch");
  if (run) {
    // Execution receipts retain the existing job journal contract, which has
    // run/job identities but no project_id field.
    if (receipt.run_id != null && receipt.run_id !== run) throw new Error("Receipt execution identity mismatch");
  } else if (receipt.project_id !== project || (worker && receipt.worker_id != null && receipt.worker_id !== worker)) {
    throw new Error("Receipt worker/project identity mismatch");
  }
}
