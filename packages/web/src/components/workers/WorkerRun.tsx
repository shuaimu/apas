"use client";

import { useEffect, useState } from "react";
import type { JobDetail, WorkerDetail } from "@apas/protocol";
import { jobFetch } from "@/lib/jobs";
import { useJobMutation } from "@/lib/useJobs";
import { onWorkerCachePurge, readWorkerRecord, workerCacheRevision, writeWorkerRecord } from "@/lib/workerCache";
import type { WorkerScope } from "@/lib/workerCache";
import { workerHref, workerPath } from "@/lib/workers";
import { JobQuestionForm } from "../jobs/JobDetail";
import { JobReviewScreen } from "../jobs/JobReview";
import { JobError, JobLink, RequestStatus, Usage } from "../jobs/JobElements";
import type { WorkerDestination } from "./WorkerDetail";

export function WorkerRun({ scope, worker, runId, questionId, review, ready, refresh, onNavigate }: {
  scope: WorkerScope; worker: WorkerDetail; runId: string; questionId?: string; review?: boolean;
  ready: boolean; refresh: () => void; onNavigate?: (destination: WorkerDestination) => void;
}) {
  const endpoint = `${workerPath(scope[1], scope[2])}/runs/${encodeURIComponent(runId)}`;
  const scopeKey = JSON.stringify(scope);
  const [saved, setSaved] = useState<{ detail: JobDetail; identity: string; revision: number; authorized: boolean }>();
  const [error, setError] = useState("");
  const identity = `${scopeKey}:${runId}:${worker.worker.state_version}`;
  useEffect(() => onWorkerCachePurge(() => { setSaved(undefined); setError("Access changed. Retained run cleared."); }), []);
  useEffect(() => {
    let active = true;
    const revision = workerCacheRevision();
    const load = async () => {
      const scope = JSON.parse(scopeKey) as WorkerScope;
      const cached = await readWorkerRecord<JobDetail>(scope, "run", runId);
      if (active && revision === workerCacheRevision() && cached?.runs.some((run) => run.id === runId)) setSaved({ detail: cached, identity, revision, authorized: false });
      if (!ready || !active || revision !== workerCacheRevision()) return;
      const detail = await jobFetch<JobDetail>(endpoint, "detail");
      if (!detail.runs.some((run) => run.id === runId) || detail.job.project_id !== scope[1] || detail.task.worker?.worker_id !== scope[2]) throw new Error("The requested worker run is unavailable. No newer run has been substituted.");
      await writeWorkerRecord(scope, "run", detail, runId, revision);
      if (active && revision === workerCacheRevision()) { setSaved({ detail, identity, revision, authorized: true }); setError(""); }
    };
    void load().catch((failure: unknown) => { if (active && revision === workerCacheRevision()) setError(failure instanceof Error ? failure.message : "Run evidence unavailable."); });
    return () => { active = false; };
  }, [scopeKey, identity, runId, endpoint, ready, worker.worker.state_version]);
  const detail = saved?.identity === identity && saved.revision === workerCacheRevision() ? saved.detail : undefined;
  const run = detail?.runs.find((candidate) => candidate.id === runId);
  if (!detail || !run) return <section><h2>Requested run <code>{runId}</code></h2><JobError message={error} /><p>No authorized evidence for this exact run is loaded. A later run will not be substituted.</p></section>;
  const runReady = ready && saved!.authorized;
  const jobScope = [scope[0], scope[1], detail.job.id] as const;
  const current = worker.current_job?.job.current_run_id === runId;
  const question = detail.questions.find((candidate) => candidate.id === questionId && candidate.run_id === runId);
  return <section aria-label="Exact worker run">
    <h2>{detail.task.worker?.purpose === "inspection" ? "Inspection" : "Implementation"} run <code>{runId}</code></h2>
    <JobError message={error} />
    <nav className="job-actions" aria-label="Run sections">
      <JobLink href={workerHref(scope[1], scope[2], `/runs/${runId}`)} onOpen={onNavigate ? () => onNavigate({ runId }) : undefined} current={!review && !questionId}>Run evidence</JobLink>
      <JobLink href={workerHref(scope[1], scope[2], `/runs/${runId}/review`)} onOpen={onNavigate ? () => onNavigate({ runId, review: true }) : undefined} current={review}>Candidate review</JobLink>
    </nav>
    {review ? <JobReviewScreen key={identity} scope={jobScope} detail={detail} ready={runReady} endpoint={endpoint} runId={runId} /> : questionId ? question ? <JobQuestionForm key={`${identity}:${questionId}`} scope={jobScope} question={question} currentRun={worker.worker.active_run_id ?? ""} ready={runReady} answerAllowed={current && worker.allowed_actions.answer && detail.job.allowed_actions.answer} refresh={refresh} answerPath={`${endpoint}/questions/${encodeURIComponent(questionId)}/answers`} onOpenJob={() => { if (onNavigate) onNavigate({ runId, questionId }); else window.location.assign(workerHref(scope[1], scope[2], `/runs/${runId}/questions/${questionId}`)); }} /> : <p>This exact dialog is unavailable in the requested run. No answer will be redirected.</p> : <>
      <p>State: {run.state.replaceAll("_", " ")} · attempt {run.attempt} · epoch {run.epoch}</p><p>{run.reason}</p>
      <p>Job <code>{detail.job.id}</code> · mandate revision {detail.task.worker?.mandate_revision}</p>
      <p>Started {run.started_at ?? "not started"} · ended {run.ended_at ?? "not confirmed ended"}</p>
      <Usage usage={run.usage} limits={detail.job.limits} />
      <section className="job-card"><h3>Immutable admitted work</h3><p className="job-prose">{detail.task.instruction}</p><ul>{detail.task.acceptance_criteria.map((criterion, index) => <li key={index}>{criterion}</li>)}</ul><p>{detail.task.repository} · {detail.task.base_ref} · <code>{detail.task.base_sha}</code></p></section>
      <section className="job-card"><h3>Positive stop evidence</h3>{run.stop_evidence ? <><p>{run.stop_evidence.descendants_stopped ? "All descendants confirmed stopped" : "Complete termination remains unconfirmed"}</p><p>{run.stop_evidence.reason} · {run.stop_evidence.stopped_at}</p><p>Container <code>{run.stop_evidence.container_id}</code></p></> : <p>No positive stop evidence recorded. A disconnected host is not evidence that this execution stopped.</p>}</section>
      <section className="job-card"><h3>Questions and provider-confirmed decisions</h3>{detail.questions.filter((item) => item.run_id === runId).map((item) => <JobLink key={item.id} className="job-row" href={workerHref(scope[1], scope[2], `/runs/${runId}/questions/${item.id}`)} onOpen={onNavigate ? () => onNavigate({ runId, questionId: item.id }) : undefined}><span>{item.questions.map((entry) => entry.header || entry.question).join(" · ")}</span><strong>{item.state.replaceAll("_", " ")}</strong></JobLink>)}</section>
    </>}
    <PublicationRecovery scope={jobScope} endpoint={endpoint} ready={runReady} allowed={current && worker.allowed_actions.retry_publication && detail.job.allowed_actions.retry_publication} version={detail.job.state_version} refresh={refresh} onOpen={() => { if (onNavigate) onNavigate({ runId, review: true }); else window.location.assign(workerHref(scope[1], scope[2], `/runs/${runId}/review`)); }} />
  </section>;
}

function PublicationRecovery({ scope, endpoint, ready, allowed, version, refresh, onOpen }: { scope: readonly [string, string, string]; endpoint: string; ready: boolean; allowed: boolean; version: number; refresh: () => void; onOpen?: () => void }) {
  const path = `${endpoint}/retry-publication`;
  const mutation = useJobMutation(scope, path);
  const [confirm, setConfirm] = useState(false);
  const submit = async () => {
    if (!ready || !allowed || mutation.busy || mutation.unresolved) return;
    const id = crypto.randomUUID();
    await mutation.submit({ id, scope, path, body: { request_id: id, expected_version: version }, createdAt: new Date().toISOString() });
    setConfirm(false); refresh();
  };
  return <section className="job-card"><h3>Same-candidate publication recovery</h3><JobError message={mutation.error} /><RequestStatus {...mutation} canResolve={ready} canRetry={ready && allowed} onResolve={async () => { await mutation.resolve(); refresh(); }} onRetry={async () => { if (ready && allowed && mutation.request) { await mutation.submit(mutation.request); refresh(); } }} onOpenJob={onOpen} /><button disabled={!ready || !allowed || mutation.busy || mutation.unresolved} onClick={() => setConfirm(true)}>Retry draft publication only</button><p>Publication recovery reconciles this exact independently verified candidate. It never starts another model run.</p>{confirm && <div><p>Reconcile publication of this same candidate?</p><button disabled={!ready || !allowed || mutation.busy || mutation.unresolved} onClick={() => void submit()}>Confirm publication recovery</button><button onClick={() => setConfirm(false)}>Keep current result</button></div>}</section>;
}
