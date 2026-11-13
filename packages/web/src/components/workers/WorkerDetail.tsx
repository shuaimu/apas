"use client";

import { useEffect, useState } from "react";
import type { WorkerDetail, WorkerItem, WorkerItemDraft, WorkerItemRequest, WorkerItemsResponse, WorkerRunsResponse, WorkerSummary } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { workerFetch, workerHref, workerPath } from "@/lib/workers";
import { useWorker, useWorkerDraft, useWorkerMutation } from "@/lib/useWorkers";
import type { WorkerMutationState as Mutation } from "@/lib/useWorkers";
import { onWorkerCachePurge, readWorkerRecord, workerCacheRevision, writeWorkerRecord } from "@/lib/workerCache";
import type { WorkerScope } from "@/lib/workerCache";
import { JobError, JobLink, limitLabels, Usage } from "../jobs/JobElements";
import { WorkerComposer } from "./WorkerComposer";
import { WorkerRun } from "./WorkerRun";

export interface WorkerDestination { runId?: string; questionId?: string; review?: boolean }
export interface WorkerDetailScreenProps extends WorkerDestination {
  id: string; projectHint?: string | null;
  onNavigate?: (destination: WorkerDestination) => void;
  onSummary?: (summary: WorkerSummary, ready: boolean) => void;
}
export function WorkerDetailScreen({ id, projectHint, runId, questionId, review, onNavigate, onSummary }: WorkerDetailScreenProps) {
  const account = useStore((state) => state.userId);
  const { saved, ready, project, error, refresh } = useWorker(id, projectHint);
  const detail = saved?.detail;
  useEffect(() => { if (detail) onSummary?.(detail.worker, ready); }, [detail, ready, onSummary]);
  if (!account || !detail || detail.worker.id !== id || detail.worker.project_id !== project) return <><h1>Autonomous worker</h1><JobError message={error} /><p>No authorized worker content is loaded.</p><button onClick={refresh}>Refresh worker</button></>;
  const scope: WorkerScope = [account, project, id];
  return <>
    <h1 tabIndex={-1}>{detail.worker.title}</h1><p>Worker <code>{id}</code> · project <code>{project}</code></p>
    <p className="job-notice" role="status">{ready ? "Current authority checked; worker replay committed." : "Offline or synchronizing — cached content is stale and controls are read-only."}</p>
    <JobError message={error} /><button onClick={refresh}>Refresh worker and authority</button>
    {runId ? <><JobLink href={workerHref(project, id)} onOpen={onNavigate ? () => onNavigate({}) : undefined}>Responsibility and backlog</JobLink><WorkerRun key={`${account}:${project}:${id}:${runId}`} scope={scope} worker={detail} runId={runId} questionId={questionId} review={review} ready={ready} refresh={refresh} onNavigate={onNavigate} /></> : <WorkerOverview key={`${account}:${project}:${id}`} scope={scope} detail={detail} ready={ready} refresh={refresh} onNavigate={onNavigate} />}
    {saved.historyResynced && <p className="job-notice">Earlier event history expired. The worker was resynchronized from its retained snapshot; current mandate, checkpoint, accounting and run evidence remain separate.</p>}
  </>;
}

function WorkerOverview({ scope, detail, ready, refresh, onNavigate }: { scope: WorkerScope; detail: WorkerDetail; ready: boolean; refresh: () => void; onNavigate?: (destination: WorkerDestination) => void }) {
  const { worker, mandate, checkpoint, allowed_actions: allowed, current_job: current } = detail;
  const [editingMandate, setEditingMandate] = useState(false);
  const selected = current?.task.worker?.items.find((item) => item.id === worker.current_item_id);
  return <>
    <section className="job-card"><h2>Status: {worker.state.replaceAll("_", " ")}</h2><p>Desired state: {worker.desired_state}</p>{worker.reason && <p className="job-notice">{worker.reason}</p>}
      {worker.state === "pausing" && <p>Future dispatch is fenced. The admitted run may finish; this is not yet Paused.</p>}
      {worker.state === "stopping" && <p>Stop requested. Complete termination has not yet been positively confirmed.</p>}
      {worker.state === "recovery_required" && <p>Execution ownership or delivery remains uncertain. Reconcile this recorded host; do not launch a replacement.</p>}
      {worker.state === "waiting_review" && <p>The delivered draft awaits review. No model is running merely to poll the pull request.</p>}
      <dl className="job-facts"><div><dt>Exact host</dt><dd>{worker.hostname} · <code>{worker.machine_id}</code></dd></div><div><dt>Host owner</dt><dd>{worker.cluster_owner_user_id}</dd></div><div><dt>Last confirmed update</dt><dd><time dateTime={worker.updated_at}>{new Date(worker.updated_at).toLocaleString()}</time></dd></div><div><dt>Next eligible check</dt><dd>{worker.next_check_at ?? "Not scheduled; see status and reason"}</dd></div></dl>
    </section>
    <section className="job-card"><h2>Description and standing checklist</h2><p className="job-prose">{mandate.spec.description}</p><ol>{mandate.spec.checklist.map((item, index) => <li key={index}>{item}</li>)}</ol><p>Mandate revision {mandate.revision} · approved {mandate.approved_at} by {mandate.approved_by}</p><p>{mandate.repository} · base branch {mandate.spec.base_ref} · inspection interval {mandate.spec.interval_seconds}s</p><p>Future runs resolve the committed tip of this approved branch, then pin their own immutable base.</p><dl className="job-facts"><div><dt>Execution profile</dt><dd>{mandate.spec.worker_profile}</dd></div><div><dt>Model profile</dt><dd>{mandate.spec.model_profile}</dd></div><div><dt>Independent verifier</dt><dd>{mandate.spec.verification_profile}</dd></div></dl><button disabled={!ready || !allowed.edit_mandate} onClick={() => setEditingMandate(true)}>Review mandate replacement</button>
      {editingMandate && <><button onClick={() => setEditingMandate(false)}>Close mandate editor</button>{ready && allowed.edit_mandate ? <WorkerComposer project={{ id: scope[1], name: scope[1] }} worker={detail} onApproved={() => { setEditingMandate(false); refresh(); }} /> : <p>Mandate editing is unavailable until current authority and allowed actions are restored.</p>}</>}
    </section>
    <section className="job-card"><h2>Latest checklist observations</h2>{checkpoint ? <><p className="job-prose">{checkpoint.summary}</p><p>Observed in run <code>{checkpoint.run_id}</code> · mandate {checkpoint.mandate_revision} · backlog {checkpoint.backlog_revision} · {checkpoint.recorded_at}</p><p>Committed base <code>{checkpoint.base_sha}</code></p><ul>{checkpoint.checklist.map((observation, index) => <li key={index}><strong>{checkpoint.mandate_revision === mandate.revision ? mandate.spec.checklist[observation.index] ?? `Checklist item ${observation.index + 1}` : `Earlier mandate checklist item ${observation.index + 1}`}</strong><p className="job-prose">{observation.observation}</p></li>)}</ul><RunLink scope={scope} runId={checkpoint.run_id} onNavigate={onNavigate}>Open inspected evidence</RunLink></> : <p>No accepted inspection or implementation checkpoint yet.</p>}<p>Observations apply only to the named run and mandate revision, not permanent completion of this responsibility.</p></section>
    <section className="job-card"><h2>Current work</h2>{worker.current_item_id ? <><p>Item <code>{worker.current_item_id}</code></p>{selected ? <ItemEvidence item={selected} /> : <p>The selected item is identified above; its immutable admitted task is shown below when retained.</p>}</> : <p>No work item is selected. An inspection may legitimately find no work.</p>}{current && <><h3>Immutable admitted task</h3><p className="job-prose">{current.task.instruction}</p><ul>{current.task.acceptance_criteria.map((criterion, index) => <li key={index}>{criterion}</li>)}</ul><p>Base <code>{current.task.base_sha}</code></p><Usage usage={current.job.usage} limits={current.job.limits} /><RunLink scope={scope} runId={current.job.current_run_id} onNavigate={onNavigate}>Open current run and decisions</RunLink></>}</section>
    <WorkerBudgetView detail={detail} />
    <WorkerControls scope={scope} detail={detail} ready={ready} refresh={refresh} />
    <WorkerBacklog scope={scope} detail={detail} ready={ready} refresh={refresh} onNavigate={onNavigate} />
    <WorkerHistory scope={scope} version={worker.state_version} ready={ready} onNavigate={onNavigate} />
  </>;
}

function WorkerBudgetView({ detail }: { detail: WorkerDetail }) {
  const { budget, mandate } = detail;
  const limits = mandate.spec.limits;
  return <section className="job-card"><h2>Recurring and per-run budgets</h2><p>Rolling 24-hour window begins {budget.window_started_at || "not reported"}. Allowance automatically renews as accounted usage leaves the window until paused; pause, resume and mandate edits never reset usage or item attempts.</p><Usage usage={budget.usage} />
    <dl className="job-facts"><div><dt>24-hour elapsed ceiling</dt><dd>{limits.elapsed_seconds_24h}s</dd></div><div><dt>24-hour reported-spend threshold</dt><dd>${(limits.spend_micro_usd_24h / 1_000_000).toFixed(6)}</dd></div><div><dt>24-hour executions</dt><dd>{budget.runs} / {limits.runs_24h}</dd></div><div><dt>Reserved elapsed</dt><dd>{budget.reserved_elapsed_seconds}s</dd></div><div><dt>Reserved reported spend</dt><dd>${(budget.reserved_spend_micro_usd / 1_000_000).toFixed(6)}</dd></div><div><dt>Reserved executions</dt><dd>{budget.reserved_runs}</dd></div><div><dt>Budget eligibility</dt><dd>{budget.eligible_at ?? "No eligibility time reported"}</dd></div><div><dt>Cumulative attempts per item</dt><dd>{limits.attempts_per_item}</dd></div></dl>
    {budget.accounting_uncertain && <p className="job-notice">Accounting is uncertain, not zero. Unresolved reservations remain held even across a window boundary.</p>}
    {(budget.usage.elapsed_seconds > limits.elapsed_seconds_24h || (budget.usage.spend_micro_usd != null && budget.usage.spend_micro_usd > limits.spend_micro_usd_24h) || budget.runs > limits.runs_24h) && <p className="job-notice">Reported usage exceeds the approved rolling ceiling. Actual overruns are retained, not hidden.</p>}
    <p>Inspection, implementation, delegated work and explicit retries all consume this allowance. Delayed or estimated provider spend is not an exact invoice cap; requests in flight may overrun thresholds.</p><h3>All per-run resource ceilings</h3><dl className="job-facts">{(Object.keys(limitLabels) as (keyof typeof limitLabels)[]).map((key) => <div key={key}><dt>{limitLabels[key]}</dt><dd>{limits.per_run[key]}</dd></div>)}</dl>
  </section>;
}

function MutationStatus({ mutation, ready, canRetry, refresh }: { mutation: Mutation; ready: boolean; canRetry: boolean; refresh: () => void }) {
  const request = mutation.request;
  return <><JobError message={mutation.error} />{request && <section className="job-notice" aria-label="Durable worker request"><p role="status">{request.receipt ? `Request ${request.receipt.status}` : "Response unconfirmed — resolve the original receipt before retrying."}</p><p>Request <code>{request.id}</code></p>{request.receipt?.error_code && <p>{request.receipt.error_code}</p>}<p>Durable acceptance is not confirmation that execution started or stopped.</p>{mutation.unresolved && <><button disabled={!ready || mutation.busy} onClick={() => void mutation.resolve().then(refresh)}>Check original receipt</button>{mutation.retryAllowed && <button disabled={!ready || !canRetry || mutation.busy} onClick={() => void mutation.submit(request).then(refresh)}>Retry saved request with the same ID</button>}</>}</section>}</>;
}
const lifecycle = { pause: "Pause after this run", stop: "Stop now", resume: "Resume", wake: "Check now" } as const;
function WorkerControls({ scope, detail, ready, refresh }: { scope: WorkerScope; detail: WorkerDetail; ready: boolean; refresh: () => void }) {
  const mutation = useWorkerMutation(scope);
  const [confirm, setConfirm] = useState<keyof typeof lifecycle>();
  const authorized = (operation: string) => operation in lifecycle && detail.allowed_actions[operation as keyof typeof lifecycle];
  const submit = async (operation: keyof typeof lifecycle) => {
    if (!ready || !authorized(operation) || mutation.busy || mutation.unresolved) return;
    const id = crypto.randomUUID();
    await mutation.submit({ id, scope, path: `${workerPath(scope[1], scope[2])}/${operation}`, body: { request_id: id, expected_version: detail.worker.state_version }, createdAt: new Date().toISOString() });
    setConfirm(undefined); refresh();
  };
  return <section className="job-card"><h2>Worker controls</h2><MutationStatus mutation={mutation} ready={ready} canRetry={authorized(mutation.request?.path.split("/").at(-1) ?? "")} refresh={refresh} /><div className="job-actions">{(Object.keys(lifecycle) as (keyof typeof lifecycle)[]).map((operation) => <button key={operation} disabled={!ready || !detail.allowed_actions[operation] || mutation.busy || mutation.unresolved} onClick={() => setConfirm(operation)}>{lifecycle[operation]}</button>)}</div><p>Pause fences future dispatch but lets the admitted run finish. Stop also requests complete termination and fences publication. Resume preserves consumed budgets and never silently retries a failed item. Check now respects cadence, capacity and allowance.</p>{confirm && <div className="job-notice"><p>Confirm {lifecycle[confirm]}? The server's resulting state, not this request, establishes execution disposition.</p><button disabled={!ready || !detail.allowed_actions[confirm] || mutation.busy || mutation.unresolved} onClick={() => void submit(confirm)}>Confirm {lifecycle[confirm]}</button><button onClick={() => setConfirm(undefined)}>Keep current worker</button></div>}</section>;
}

function useWorkerPage<T extends WorkerItemsResponse | WorkerRunsResponse>(scope: WorkerScope, kind: "items" | "runs", cursor: string, ready: boolean, version: number) {
  const scopeKey = JSON.stringify(scope);
  const identity = `${scopeKey}:${kind}:${cursor}:${version}`;
  const [saved, setSaved] = useState<{ identity: string; revision: number; data: T; authorized: boolean }>();
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  useEffect(() => onWorkerCachePurge(() => { setSaved(undefined); setLoading(false); setError("Access changed. Retained pages cleared."); }), []);
  useEffect(() => {
    let active = true;
    const revision = workerCacheRevision();
    setLoading(true); setError("");
    const load = async () => {
      const scope = JSON.parse(scopeKey) as WorkerScope;
      const cache = await readWorkerRecord<T>(scope, kind, cursor);
      if (!active || revision !== workerCacheRevision()) return;
      if (cache) setSaved({ identity, revision, data: cache, authorized: false });
      if (!ready) return;
      const data = await workerFetch<T>(`${workerPath(scope[1], scope[2])}/${kind}${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ""}`, kind);
      await writeWorkerRecord(scope, kind, data, cursor, revision);
      if (active && revision === workerCacheRevision()) setSaved({ identity, revision, data, authorized: true });
    };
    void load().catch((failure: unknown) => { if (active && revision === workerCacheRevision()) setError(failure instanceof Error ? failure.message : "Page unavailable."); }).finally(() => { if (active && revision === workerCacheRevision()) setLoading(false); });
    return () => { active = false; };
  }, [scopeKey, identity, kind, cursor, ready, version]);
  const current = saved?.identity === identity && saved.revision === workerCacheRevision() ? saved : undefined;
  return { data: current?.data, ready: ready && !loading && Boolean(current?.authorized), error, loading };
}
function usePages() {
  const [cursor, setCursor] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  return { cursor, hasPrevious: history.length > 0, previous: () => { setCursor(history.at(-1) ?? ""); setHistory((values) => values.slice(0, -1)); }, next: (cursorNext: string) => { setHistory((values) => [...values, cursor]); setCursor(cursorNext); } };
}
function ItemEvidence({ item }: { item: WorkerItem }) {
  return <><h3>{item.item.title}</h3><p>{item.state.replaceAll("_", " ")} · priority {item.item.priority} · {item.origin} · revision {item.revision} · cumulative attempts {item.attempts}</p><p className="job-prose">{item.item.instruction}</p><ul>{item.item.acceptance_criteria.map((criterion, index) => <li key={index}>{criterion}</li>)}</ul><h4>Discovery and scope rationale</h4><p className="job-prose">{item.item.scope_rationale}</p><ul>{item.item.evidence.map((evidence, index) => <li key={index}>{evidence}</li>)}</ul><p>Dependencies: {item.item.dependencies.join(", ") || "None"}</p>{item.outcome && <p>Outcome: {item.outcome}</p>}</>;
}
function WorkerBacklog({ scope, detail, ready, refresh, onNavigate }: { scope: WorkerScope; detail: WorkerDetail; ready: boolean; refresh: () => void; onNavigate?: (destination: WorkerDestination) => void }) {
  const pages = usePages();
  const page = useWorkerPage<WorkerItemsResponse>(scope, "items", pages.cursor, ready, detail.worker.state_version);
  const path = `${workerPath(scope[1], scope[2])}/items`;
  const mutation = useWorkerMutation(scope);
  const [editor, setEditor] = useState<{ item?: WorkerItem }>();
  const [decision, setDecision] = useState<{ item: WorkerItem; operation: "dismiss" | "retry" }>();
  const canEdit = page.ready && detail.allowed_actions.edit_backlog;
  const canDecide = (item: WorkerItem, operation: "dismiss" | "retry") => page.ready && (operation === "retry" ? detail.allowed_actions.retry && item.state === "blocked" && item.attempts < detail.mandate.spec.limits.attempts_per_item : detail.allowed_actions.edit_backlog && (item.state === "ready" || item.state === "blocked"));
  const decide = async () => {
    if (!decision || !canDecide(decision.item, decision.operation) || mutation.busy || mutation.unresolved) return;
    const id = crypto.randomUUID();
    await mutation.submit({ id, scope, path: `${path}/${encodeURIComponent(decision.item.id)}/${decision.operation}`, body: { request_id: id, expected_version: detail.worker.state_version }, createdAt: new Date().toISOString() });
    setDecision(undefined); refresh();
  };
  const pendingItem = page.data?.items.find((item) => mutation.request?.path.includes(`/${item.id}/`));
  const pendingOperation = mutation.request?.path.endsWith("/retry") ? "retry" : "dismiss";
  const retryAllowed = mutation.request?.path === path ? canEdit : Boolean(pendingItem && canDecide(pendingItem, pendingOperation));
  return <section className="job-card"><h2>Backlog</h2><p>Only this bounded page is shown. Claimed work is immutable; edits do not change an active execution.</p><JobError message={page.error} /><MutationStatus mutation={mutation} ready={page.ready} canRetry={retryAllowed} refresh={refresh} /><button disabled={!canEdit || mutation.busy || mutation.unresolved} onClick={() => setEditor({})}>Add backlog item</button>
    {page.data?.items.map((item) => <article key={item.id} className="job-card"><ItemEvidence item={item} /><p>Item <code>{item.id}</code></p>{item.run_id && <RunLink scope={scope} runId={item.run_id} onNavigate={onNavigate}>Open item's exact result</RunLink>}<div className="job-actions"><button disabled={!canEdit || !["ready", "blocked"].includes(item.state) || mutation.busy || mutation.unresolved} onClick={() => setEditor({ item })}>Edit or reprioritize {item.item.title}</button><button disabled={!canDecide(item, "dismiss") || mutation.busy || mutation.unresolved} onClick={() => setDecision({ item, operation: "dismiss" })}>Dismiss {item.item.title}</button><button disabled={!canDecide(item, "retry") || mutation.busy || mutation.unresolved} onClick={() => setDecision({ item, operation: "retry" })}>Retry {item.item.title}</button></div></article>)}
    {page.data && !page.data.items.length && <p>No backlog items on this page.</p>}{page.loading && <p role="status">Loading backlog page…</p>}
    <div className="job-actions"><button disabled={!pages.hasPrevious || page.loading} onClick={pages.previous}>Previous backlog page</button><button disabled={!page.ready || !page.data?.next_cursor} onClick={() => pages.next(page.data!.next_cursor!)}>Next backlog page</button></div>
    {editor && <ItemEditor key={`${editor.item?.id ?? "new"}:${editor.item?.revision ?? 0}`} scope={scope} item={editor.item} version={detail.worker.state_version} ready={canEdit && (!editor.item || Boolean(page.data?.items.some((item) => item.id === editor.item!.id && item.revision === editor.item!.revision && ["ready", "blocked"].includes(item.state))))} mutation={mutation} onClose={() => setEditor(undefined)} refresh={refresh} />}
    {decision && <div className="job-notice"><p>{decision.operation === "retry" ? "Explicitly retry this blocked item using only remaining worker and cumulative item allowance? This does not reset its attempts." : "Dismiss this unclaimed item? This does not stop or rewrite an active execution."}</p><button disabled={!canDecide(decision.item, decision.operation) || mutation.busy || mutation.unresolved} onClick={() => void decide()}>Confirm {decision.operation}</button><button onClick={() => setDecision(undefined)}>Keep item</button></div>}
  </section>;
}
const emptyItem: WorkerItemDraft = { title: "", instruction: "", acceptance_criteria: [], scope_rationale: "", evidence: [], priority: 0, dependencies: [] };
function ItemEditor({ scope, item, version, ready, mutation, onClose, refresh }: { scope: WorkerScope; item?: WorkerItem; version: number; ready: boolean; mutation: Mutation; onClose: () => void; refresh: () => void }) {
  const draft = useWorkerDraft(scope, `item:${item?.id ?? "new"}:${item?.revision ?? 0}`, item?.item ?? emptyItem);
  const [error, setError] = useState("");
  const update = (patch: Partial<WorkerItemDraft>) => draft.update({ ...draft.value, ...patch });
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!ready || !draft.loaded || mutation.busy || mutation.unresolved) return;
    const revision = workerCacheRevision();
    try {
      await draft.flush();
      if (revision !== workerCacheRevision()) throw new Error("Access changed while saving. No item was submitted.");
      const id = crypto.randomUUID();
      const body: WorkerItemRequest = { request_id: id, expected_version: version, item_id: item?.id ?? crypto.randomUUID(), expected_item_revision: item?.revision ?? null, item: { ...draft.value, acceptance_criteria: draft.value.acceptance_criteria.filter((value) => value.trim()), evidence: draft.value.evidence.filter((value) => value.trim()), dependencies: draft.value.dependencies.filter((value) => value.trim()) } };
      const result = await mutation.submit({ id, scope, path: `${workerPath(scope[1], scope[2])}/items`, body, createdAt: new Date().toISOString() });
      if (revision !== workerCacheRevision()) return;
      if (result?.receipt?.status === "accepted" || result?.receipt?.status === "completed") { if (!item) draft.update(emptyItem); onClose(); }
      refresh();
    } catch (failure) { setError(failure instanceof Error ? failure.message : "Item draft could not be saved."); }
  };
  return <form onSubmit={(event) => void submit(event)} className="job-card"><h3>{item ? "Edit unclaimed item and priority" : "Add backlog item"}</h3><JobError message={error || draft.error} /><fieldset disabled={!draft.loaded || mutation.busy || mutation.unresolved}><legend>Saved item draft</legend><label>Title<input required maxLength={200} value={draft.value.title} onChange={(event) => update({ title: event.target.value })} /></label><label>Instruction<textarea required maxLength={8192} value={draft.value.instruction} onChange={(event) => update({ instruction: event.target.value })} /></label><label>Acceptance criteria, one per line<textarea required value={draft.value.acceptance_criteria.join("\n")} onChange={(event) => update({ acceptance_criteria: event.target.value.split("\n") })} /></label><label>Scope rationale<textarea required maxLength={2048} value={draft.value.scope_rationale} onChange={(event) => update({ scope_rationale: event.target.value })} /></label><label>Discovery evidence, one per line<textarea value={draft.value.evidence.join("\n")} onChange={(event) => update({ evidence: event.target.value.split("\n") })} /></label><label>Priority<input type="number" required step={1} min={-2147483648} max={2147483647} value={draft.value.priority} onChange={(event) => update({ priority: Number(event.target.value) })} /></label><label>Dependency item IDs, one per line<textarea value={draft.value.dependencies.join("\n")} onChange={(event) => update({ dependencies: event.target.value.split("\n") })} /></label><button type="submit" disabled={!ready || Boolean(draft.error)}>Save backlog item</button></fieldset><button type="button" onClick={onClose}>Close item editor</button><p>Drafts are not automatically submitted. A stale revision conflicts and refreshes current work; it never rewrites a running task.</p></form>;
}
function WorkerHistory({ scope, version, ready, onNavigate }: { scope: WorkerScope; version: number; ready: boolean; onNavigate?: (destination: WorkerDestination) => void }) {
  const pages = usePages();
  const page = useWorkerPage<WorkerRunsResponse>(scope, "runs", pages.cursor, ready, version);
  return <section className="job-card"><h2>Runs and results</h2><p>Execution history is paginated independently of the backlog. A completed inspection or delivered draft does not complete the worker.</p><JobError message={page.error} />{page.data?.runs.map((entry) => <article key={entry.run.id} className="job-card"><h3>{entry.purpose} · {entry.run.state.replaceAll("_", " ")}</h3><p>Run <code>{entry.run.id}</code> · mandate {entry.mandate_revision}{entry.item_id && <> · item <code>{entry.item_id}</code></>}</p><p>{entry.run.reason}</p><p>{entry.run.started_at ?? "Not started"} · {entry.run.ended_at ?? "End not confirmed"}</p><Usage usage={entry.run.usage} /><RunLink scope={scope} runId={entry.run.id} onNavigate={onNavigate}>Open exact run</RunLink><JobLink href={workerHref(scope[1], scope[2], `/runs/${entry.run.id}/review`)} onOpen={onNavigate ? () => onNavigate({ runId: entry.run.id, review: true }) : undefined}>Review this result</JobLink></article>)}{page.data && !page.data.runs.length && <p>No runs on this page.</p>}{page.loading && <p role="status">Loading run history…</p>}<div className="job-actions"><button disabled={!pages.hasPrevious || page.loading} onClick={pages.previous}>Previous run page</button><button disabled={!page.ready || !page.data?.next_cursor} onClick={() => pages.next(page.data!.next_cursor!)}>Next run page</button></div></section>;
}
function RunLink({ scope, runId, onNavigate, children }: { scope: WorkerScope; runId: string; onNavigate?: (destination: WorkerDestination) => void; children: React.ReactNode }) {
  return <JobLink href={workerHref(scope[1], scope[2], `/runs/${runId}`)} onOpen={onNavigate ? () => onNavigate({ runId }) : undefined}>{children}</JobLink>;
}
