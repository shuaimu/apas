import { router, useLocalSearchParams } from "expo-router";
import { ErrorNotice, SectionTitle } from "@/components/ui";
import { Allowance, Copy, JobAction } from "@/jobs/ui";
import { workersApi } from "./api";
import { exactWorkerRun } from "./continuity";
import { useWorker, useWorkerEvidence, useWorkerMutation } from "./hooks";
import { useWorkers } from "./runtime";
import { WorkerPage } from "./ui";

export function useExactWorkerRun(worker: string, run: string, project?: string) {
  const selected = useWorker(worker, project); const account = useWorkers((state) => state.account);
  const actualProject = selected.entry?.detail.worker.project_id;
  const scope = account && actualProject ? { account, project: actualProject, worker } : null;
  const execution = useWorkerEvidence(scope, `run:${run}`, async (signal) => exactWorkerRun(await workersApi.run(actualProject!, worker, run, signal), worker, run), Boolean(selected.entry?.ready));
  return { selected, scope, execution };
}
export default function WorkerRunScreen() {
  const { workerId, runId, project } = useLocalSearchParams<{ workerId: string; runId: string; project?: string }>();
  const { selected, scope, execution } = useExactWorkerRun(workerId, runId, project);
  const request = useWorkerMutation(scope, `run:${runId}:publication-request`);
  const detail = execution.data; const run = detail?.runs.find((value) => value.id === runId);
  return <WorkerPage title="Exact worker execution"><JobAction onPress={() => router.push({ pathname: "/(code)/workers/[workerId]", params: { workerId, project: scope?.project ?? project } })}>Worker responsibility</JobAction>{(selected.error || execution.error) && <ErrorNotice message={selected.error ?? String(execution.error)} />}{detail && run && <><Copy>Worker {workerId}{"\n"}Run {runId}; job {detail.job.id}; epoch {run.epoch}{"\n"}Purpose {detail.task.worker?.purpose}; mandate {detail.task.worker?.mandate_revision}{"\n"}State {run.state.replaceAll("_", " ")}; reason {run.reason ?? "none"}{"\n"}Started {run.started_at ?? "not started"}; ended {run.ended_at ?? "not confirmed"}</Copy><SectionTitle>Immutable execution instructions</SectionTitle><Copy>{detail.task.instruction}{"\n"}{detail.task.acceptance_criteria.join("\n")}{"\n"}Repository {detail.task.repository}; base {detail.task.base_ref} @ {detail.task.base_sha}{"\n"}Candidate {run.candidate_sha ?? "none"}</Copy><Allowance limits={detail.job.limits} usage={run.usage} /><SectionTitle>Trusted stop evidence</SectionTitle><Copy>{run.stop_evidence ? `${run.stop_evidence.stopped_at}: ${run.stop_evidence.reason}; descendants stopped: ${run.stop_evidence.descendants_stopped}` : "No positive stop evidence retained; do not infer safe termination."}</Copy><SectionTitle>Exact run questions</SectionTitle>{detail.questions.filter((question) => question.run_id === runId).map((question) => <JobAction key={question.id} onPress={() => router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]/questions/[questionId]", params: { workerId, runId, questionId: question.id, project: scope!.project } })}>Question {question.id} · {question.state.replaceAll("_", " ")}</JobAction>)}<JobAction onPress={() => router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]/review", params: { workerId, runId, project: scope!.project } })}>Exact candidate, independent checks, artifacts and PR disposition</JobAction><JobAction disabled={!selected.ready || !selected.entry?.detail.allowed_actions.retry_publication || !detail.job.allowed_actions.retry_publication || detail.job.current_run_id !== runId || !request.loaded || Boolean(request.record) || request.busy} onPress={() => { void request.submit({ kind: "retry-publication", run: runId, body: { request_id: request.requestId(), expected_version: detail.job.state_version } }); }}>Recover publication of this same candidate (no coding retry)</JobAction></>}{request.message && <ErrorNotice message={request.message} />}{request.record && <><JobAction disabled={!selected.ready} onPress={() => { void request.resolve(); }}>Resolve original publication receipt</JobAction><JobAction disabled={!selected.ready || !request.canRetry} onPress={() => { void request.retry(); }}>Explicitly retry saved publication request</JobAction>{request.record.receipt && request.record.receipt.status !== "pending" && <JobAction onPress={() => { void request.clear(); }}>Acknowledge publication receipt</JobAction>}</>}</WorkerPage>;
}
