"use client";

import { useEffect, useMemo, useState } from "react";
import type { JobAllowedActions, JobListResponse, JobQuestion, JobQuestionAnswer } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { jobHref, jobPath } from "@/lib/jobs";
import { useJob, useJobDraft, useJobMutation } from "@/lib/useJobs";
import type { JobScope } from "@/lib/jobCache";
import { JobError, JobLink, limitLabels, Publication, RequestStatus, Usage } from "./JobElements";
import { JobReviewScreen } from "./JobReview";

export interface JobDestination {
  review?: boolean;
  questionId?: string;
}
export interface JobDetailScreenProps extends JobDestination {
  id: string;
  projectHint?: string | null;
  onNavigate?: (destination: JobDestination) => void;
  onSummary?: (job: JobListResponse["jobs"][number], ready: boolean) => void;
}
export function JobDetailScreen({ id, projectHint, questionId, review, onNavigate, onSummary }: JobDetailScreenProps) {
  const account = useStore((state) => state.userId)!;
  const state = useJob(id, projectHint);
  const { saved, ready, project, error, refresh } = state;
  const summary = saved?.detail.job;
  useEffect(() => {
    if (summary) onSummary?.(summary, ready);
  }, [summary, ready, onSummary]);
  if (!saved) return <><h1 tabIndex={-1}>Autonomous job</h1><JobError message={error} /><p role="status">{error ? "No authorized job content is available." : "Loading authorized job…"}</p><button onClick={refresh}>Refresh job</button></>;
  const { job, task, runs, questions } = saved.detail;
  const currentRun = runs.find((run) => run.id === job.current_run_id);
  const openProgress = onNavigate ? () => onNavigate({}) : undefined;
  return <>
    <h1 tabIndex={-1}>{job.title}</h1>
    <p>{job.project_name} · <code>{job.id}</code></p>
    <nav aria-label="Job sections" className="job-actions">
      <JobLink href={jobHref(project, id)} onOpen={openProgress} current={!questionId && !review}>Progress</JobLink>
      <JobLink href={jobHref(project, id, "/review")} onOpen={onNavigate ? () => onNavigate({ review: true }) : undefined} current={review}>Review</JobLink>
      {questions.filter((question) => question.state === "pending" || question.state === "uncertain").map((question, index) => <JobLink key={question.id} href={jobHref(project, id, `/questions/${question.id}`)} onOpen={onNavigate ? () => onNavigate({ questionId: question.id }) : undefined} current={questionId === question.id}>Question {index + 1}</JobLink>)}
    </nav>
    <p className="job-notice" role="status">{ready ? "Current authority checked; job replay committed." : "Offline or synchronizing this job — cached content is stale and controls are read-only."}</p>
    <JobError message={error} /><button type="button" onClick={refresh}>Refresh job and authority</button>
    {review ? <JobReviewScreen scope={[account, project, id]} detail={saved.detail} ready={ready} /> : questionId ? <>
      {questions.find((question) => question.id === questionId) ? <JobQuestionForm key={`${job.current_run_id}:${questionId}`} question={questions.find((question) => question.id === questionId)!} scope={[account, project, id]} currentRun={job.current_run_id} ready={ready} answerAllowed={job.allowed_actions.answer} refresh={refresh} onOpenJob={openProgress} /> : <p className="job-notice">This exact question is no longer available in this job. No answer will be sent to a different dialog.</p>}
    </> : <>
      <section className="job-card"><h2>Phase: {job.state.replaceAll("_", " ")}</h2>
        {job.state === "cancelling" && <p role="status">Cancellation requested. The execution has not yet been confirmed stopped.</p>}
        {job.state === "cancelled" && <p>{currentRun?.stop_evidence?.descendants_stopped ? "Stopped: the host confirmed all descendants terminated." : "Cancelled; see run evidence below for execution disposition."}</p>}
        {job.reason && <p className="job-notice">{job.reason}</p>}
        {job.state === "recovery_required" && <p>Ownership or delivery is uncertain. Refresh for reconciled evidence; do not start a replacement run. Contact the hosting operator if the recorded host cannot reconnect.</p>}
        {job.state === "blocked" && <p>Resolve the reason above with the hosting operator, then refresh readiness. The approved target and allowance are not changed automatically.</p>}
        <dl className="job-facts"><div><dt>Target</dt><dd>{job.hostname} · <code>{job.machine_id}</code></dd></div><div><dt>Host owner</dt><dd>{job.cluster_owner_user_id}</dd></div><div><dt>Current run</dt><dd><code>{job.current_run_id}</code> · attempt {currentRun?.attempt ?? "unknown"} of {job.limits.attempts}</dd></div><div><dt>Latest confirmed update</dt><dd><time dateTime={job.updated_at}>{new Date(job.updated_at).toLocaleString()}</time></dd></div></dl>
        <Usage usage={job.usage} limits={job.limits} /><p>Usage is cumulative across attempts. Reported-spend stopping is not a hard invoice cap; requests already in flight can overrun the threshold.</p>
        <details><summary>All approved resource limits</summary>{(Object.keys(limitLabels) as (keyof typeof limitLabels)[]).map((key) => <p key={key}>{limitLabels[key]}: {job.limits[key]}</p>)}</details>
      </section>
      <JobControls scope={[account, project, id]} version={job.state_version} allowed={job.allowed_actions} ready={ready} refresh={refresh} onOpenJob={openProgress} />
      <section className="job-card"><h2>Questions and confirmed decisions</h2>{questions.length ? questions.map((question) => <JobLink className="job-row" key={question.id} href={jobHref(project, id, `/questions/${question.id}`)} onOpen={onNavigate ? () => onNavigate({ questionId: question.id }) : undefined}><span>{question.questions.map((item) => item.header || item.question).join(" · ")}</span><strong>{question.state.replaceAll("_", " ")}</strong></JobLink>) : <p>No questions reported.</p>}</section>
      <Publication publication={saved.detail.publication} />
      <section className="job-card"><h2>Approved task</h2><p className="job-prose">{task.instruction}</p><ul>{task.acceptance_criteria.map((item, index) => <li key={index}>{item}</li>)}</ul><p>{task.repository} · {task.base_ref} · <code>{task.base_sha}</code></p></section>
      {saved.historyResynced && <p className="job-notice">Earlier event history is no longer replayable. This view was resynchronized from the server snapshot; retained task, decisions and review evidence remain available.</p>}
      <section className="job-card"><h2>Progress and delegated work</h2><p>Latest {saved.events.length} retained events; no invented completion percentage. Child details expand individually.</p><ol className="job-timeline">{saved.events.map((event) => <li key={event.id}><time dateTime={event.created_at}>{new Date(event.created_at).toLocaleString()}</time><p><strong>{event.kind.replaceAll("_", " ")}</strong> — {event.summary}</p>{event.run_id && <p>Run <code>{event.run_id}</code></p>}{event.detail != null && <details><summary>{event.kind === "subagent" ? "Delegated work details" : "Event evidence"}</summary><pre tabIndex={0}>{JSON.stringify(event.detail, null, 2)}</pre></details>}</li>)}</ol></section>
      <section className="job-card"><h2>Run history and stop evidence</h2>{runs.map((run) => <details key={run.id}><summary>Attempt {run.attempt} — {run.state.replaceAll("_", " ")}</summary><p>Run <code>{run.id}</code> · epoch {run.epoch} · worker {run.worker_version ?? "not reported"}</p><p>{run.reason}</p><Usage usage={run.usage} /><p>Started: {run.started_at ?? "not started"} · ended: {run.ended_at ?? "not confirmed ended"}</p>{run.stop_evidence ? <p>{run.stop_evidence.descendants_stopped ? "All descendants confirmed stopped" : "Termination not fully confirmed"} at {run.stop_evidence.stopped_at}. {run.stop_evidence.reason} · container <code>{run.stop_evidence.container_id}</code></p> : <p>No positive host stop evidence has been recorded for this run.</p>}</details>)}</section>
    </>}
  </>;
}
function JobControls({ scope, version, allowed, ready, refresh, onOpenJob }: { scope: JobScope; version: number; allowed: JobAllowedActions; ready: boolean; refresh: () => void; onOpenJob?: (jobId: string) => void }) {
  const mutation = useJobMutation(scope);
  const [confirm, setConfirm] = useState<"cancel" | "retry" | "retry-publication">();
  const retryAuthorized = mutation.request?.path.endsWith("/cancel") ? allowed.cancel
    : mutation.request?.path.endsWith("/retry") ? allowed.retry
    : mutation.request?.path.endsWith("/retry-publication") ? allowed.retry_publication
    : mutation.request?.path.endsWith("/answers") ? allowed.answer : false;
  const submit = async (operation: "cancel" | "retry" | "retry-publication") => {
    const id = crypto.randomUUID();
    await mutation.submit({ id, scope, path: `${jobPath(scope[1], scope[2])}/${operation}`, body: { request_id: id, expected_version: version }, createdAt: new Date().toISOString() });
    setConfirm(undefined); refresh();
  };
  return <section className="job-card"><h2>Job controls</h2><JobError message={mutation.error} />
    <RequestStatus {...mutation} onOpenJob={onOpenJob} canResolve={ready} canRetry={ready && retryAuthorized} onResolve={async () => { await mutation.resolve(); refresh(); }} onRetry={async () => { if (mutation.request) { await mutation.submit(mutation.request); refresh(); } }} />
    <div className="job-actions"><button disabled={!ready || !allowed.cancel || mutation.busy || mutation.unresolved} onClick={() => setConfirm("cancel")}>Request cancellation</button><button disabled={!ready || !allowed.retry || mutation.busy || mutation.unresolved} onClick={() => setConfirm("retry")}>Retry coding from approved base</button><button disabled={!ready || !allowed.retry_publication || mutation.busy || mutation.unresolved} onClick={() => setConfirm("retry-publication")}>Retry draft publication only</button></div>
    <p>Only currently authorized actions are enabled. Coding retry requires confirmed absence of the previous execution and remaining cumulative allowance. Publication retry never runs the model again.</p>
    {confirm && <div className="job-notice"><p>{confirm === "cancel" ? "Request stopping this job? It remains cancelling until positive stop evidence arrives." : confirm === "retry" ? "Start another coding attempt from the original approved base using only the remaining allowance?" : "Reconcile publication of the same independently verified candidate?"}</p><button disabled={!ready || mutation.busy || !(confirm === "cancel" ? allowed.cancel : confirm === "retry" ? allowed.retry : allowed.retry_publication)} onClick={() => void submit(confirm)}>Confirm {confirm.replaceAll("-", " ")}</button><button onClick={() => setConfirm(undefined)}>Keep current job</button></div>}
  </section>;
}
function JobQuestionForm({ question, scope, currentRun, ready, answerAllowed, refresh, onOpenJob }: { question: JobQuestion; scope: JobScope; currentRun: string; ready: boolean; answerAllowed: boolean; refresh: () => void; onOpenJob?: (jobId: string) => void }) {
  const initial = useMemo<JobQuestionAnswer[]>(() => question.questions.map((item) => ({ id: item.id, selected_option_ids: [], custom_input: null })), [question.questions]);
  const draft = useJobDraft(scope, `answer:${question.run_id}:${question.epoch}:${question.id}:${question.version}`, initial);
  const path = `${jobPath(scope[1], scope[2])}/questions/${encodeURIComponent(question.id)}/answers`;
  const mutation = useJobMutation(scope, path);
  const [error, setError] = useState("");
  const stale = question.run_id !== currentRun;
  const canAnswer = ready && answerAllowed && !stale && question.state === "pending";
  const update = (id: string, patch: Partial<JobQuestionAnswer>) => draft.update(draft.value.map((answer) => answer.id === id ? { ...answer, ...patch } : answer));
  const submit = async (event: React.FormEvent) => {
    event.preventDefault(); setError("");
    try {
      if (draft.value.length !== question.questions.length || draft.value.some((answer) => !answer.selected_option_ids.length && !answer.custom_input?.trim())) throw new Error("Answer every question with at least one choice or custom text.");
      await draft.flush();
      const id = crypto.randomUUID();
      await mutation.submit({ id, scope, path, createdAt: new Date().toISOString(), body: { request_id: id, run_id: question.run_id, epoch: question.epoch, expected_version: question.version, answers: draft.value } });
      refresh();
    } catch (failure) { setError(failure instanceof Error ? failure.message : "Answer draft could not be saved."); }
  };
  const stateLabels: Record<JobQuestion["state"], string> = {
    pending: "Awaiting a decision. No recommended option has been selected for you.",
    answer_accepted: "First-writer answer accepted, possibly on another device. Waiting for provider confirmation; do not resubmit.",
    answer_delivered: "Answer delivered. Provider confirmation is still pending.",
    confirmed: "Provider confirmed the answer shown below. If you did not submit it, it was answered elsewhere.",
    uncertain: "Answer delivery is uncertain. Do not retransmit into another dialog. Await host reconciliation.",
    cancelled: "This dialog was cancelled. Its saved answer will not be submitted.",
  };
  return <section className="job-card"><h2>Job questions</h2><p role="status">{stateLabels[question.state]}</p>{stale && <p className="job-notice">Stale run: these questions belong to an earlier attempt. Answering is disabled.</p>}
    <p>Run <code>{question.run_id}</code> · dialog <code>{question.dialog_id}</code> · version {question.version}</p>
    <JobError message={error || draft.error || mutation.error} />
    <RequestStatus {...mutation} onOpenJob={onOpenJob} canResolve={ready} canRetry={canAnswer} onResolve={async () => { await mutation.resolve(); refresh(); }} onRetry={async () => { if (mutation.request) { await mutation.submit(mutation.request); refresh(); } }} />
    {question.answers && <section aria-label="Server-recorded answers"><h3>{question.state === "confirmed" ? "Provider-confirmed answers" : "Accepted answers, awaiting provider"}</h3>{question.answers.map((answer) => <div key={answer.id}><h4>{question.questions.find((item) => item.id === answer.id)?.question ?? answer.id}</h4><ul>{answer.selected_option_ids.map((id) => <li key={id}>{question.questions.find((item) => item.id === answer.id)?.options.find((option) => option.id === id)?.label ?? id}</li>)}</ul>{answer.custom_input && <p className="job-prose">{answer.custom_input}</p>}</div>)}{question.confirmed_at && <p>Confirmed at {new Date(question.confirmed_at).toLocaleString()}</p>}</section>}
    <form onSubmit={(event) => void submit(event)}><fieldset disabled={!draft.loaded || question.state !== "pending" || stale || mutation.busy || mutation.unresolved}>
      <legend>Saved answer draft</legend>
      {question.questions.map((item) => {
        const answer = draft.value.find((value) => value.id === item.id);
        return <fieldset key={item.id}><legend>{item.header ? `${item.header}: ` : ""}{item.question}</legend><p>{item.multi ? "Select any applicable choices, or enter custom text." : "Choose one option, or enter custom text."}</p>
          {item.options.map((option, index) => <label className="job-choice" key={option.id}><input type={item.multi ? "checkbox" : "radio"} name={`question-${item.id}`} checked={answer?.selected_option_ids.includes(option.id) ?? false} onChange={(event) => update(item.id, { selected_option_ids: item.multi ? event.target.checked ? [...(answer?.selected_option_ids ?? []), option.id] : (answer?.selected_option_ids ?? []).filter((id) => id !== option.id) : [option.id], custom_input: item.multi ? answer?.custom_input : null })} /><span><strong>{option.label}</strong>{item.recommended === index && <span> (recommended, not selected automatically)</span>}{option.description && <span className="job-option-description">{option.description}</span>}{option.preview && <pre>{option.preview}</pre>}</span></label>)}
          <label>Custom text for {item.header || item.question}<textarea rows={3} value={answer?.custom_input ?? ""} onChange={(event) => update(item.id, { custom_input: event.target.value || null, selected_option_ids: item.multi ? answer?.selected_option_ids ?? [] : [] })} /></label>
        </fieldset>;
      })}
      <button type="submit" disabled={!canAnswer || Boolean(draft.error)}>Submit all answers for this dialog</button>
    </fieldset></form><p>Answers are durable first-writer decisions. A submission receipt is not provider confirmation. Local drafts may be edited offline but are never automatically sent.</p>
  </section>;
}
