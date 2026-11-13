"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import type { WorkerState } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { useProjectWorkers } from "@/lib/useProjectWorkers";
import { workerHref, resolveWorkerRequest } from "@/lib/workers";
import { workerCacheRevision, workerRequests, onWorkerCachePurge } from "@/lib/workerCache";
import type { SavedWorkerRequest } from "@/lib/workerCache";
import { JobError } from "../jobs/JobElements";

const statuses: WorkerState[] = ["queued", "preparing", "inspecting", "working", "needs_input", "verifying", "publishing", "idle", "waiting_review", "waiting_budget", "pausing", "paused", "stopping", "blocked", "recovery_required"];
export function workerStateLabel(state: WorkerState): string {
  switch (state) {
    case "needs_input": return "Waiting for answer";
    case "waiting_review": return "Waiting for review";
    case "waiting_budget": return "Waiting for budget";
    case "recovery_required": return "Recovery required";
    default: return state.replaceAll("_", " ");
  }
}

export function WorkerList({ attention = false }: { attention?: boolean }) {
  const account = useStore((state) => state.userId);
  const [status, setStatus] = useState<WorkerState | "">("");
  const inventory = useProjectWorkers(undefined, attention ? "attention" : status || undefined);
  const [requests, setRequests] = useState<SavedWorkerRequest[]>([]);
  const [error, setError] = useState("");
  const [recovering, setRecovering] = useState(false);
  useEffect(() => {
    let active = true;
    const revision = workerCacheRevision();
    if (account) void workerRequests(account).then((values) => {
      if (active && revision === workerCacheRevision()) setRequests(values.filter((value) => !value.receipt || value.receipt.status === "pending"));
    }).catch((failure: unknown) => { if (active && revision === workerCacheRevision()) setError(failure instanceof Error ? failure.message : "Saved requests could not be loaded."); });
    const remove = onWorkerCachePurge(() => { setRequests([]); setError(""); });
    return () => { active = false; remove(); };
  }, [account]);
  const recover = async (request: SavedWorkerRequest) => {
    const revision = workerCacheRevision();
    setRecovering(true); setError("");
    try {
      const resolved = await resolveWorkerRequest(request);
      if (revision === workerCacheRevision()) setRequests((values) => values.map((value) => value.id === request.id ? resolved : value));
    } catch (failure) {
      if (revision === workerCacheRevision()) setError(failure instanceof Error ? failure.message : "The original receipt could not be resolved.");
    } finally { setRecovering(false); }
  };
  return <>
    <h1 tabIndex={-1}>{attention ? "Attention" : "Autonomous workers"}</h1>
    <p>{attention ? "Decisions, draft review, blocked work and recovery across accessible projects. Opening a destination does not answer, resume or retry work." : "Persistent responsibilities discover bounded work within their approved mandate. Idle and paused workers remain here independently of interactive sessions."}</p>
    <div className="job-actions"><Link href="/code/workers/new" className="job-button">New autonomous worker</Link><Link href="/code/jobs" className="job-button">Earlier results</Link><button disabled={inventory.loading} onClick={inventory.refresh}>Refresh workers and authority</button></div>
    {!attention && <label>Status<select value={status} onChange={(event) => setStatus(event.target.value as WorkerState | "")}><option value="">All states</option>{statuses.map((value) => <option key={value} value={value}>{workerStateLabel(value)}</option>)}</select></label>}
    {!inventory.ready && <p className="job-notice" role="status">Offline or synchronizing — cached workers are stale and read-only. Remote revocations cannot be discovered while offline.</p>}
    <JobError message={inventory.error || error} />
    {!!requests.length && <section className="job-card"><h2>Interrupted request recovery</h2><p>Resolve the original durable identity before submitting again. No POST is replayed automatically.</p>{requests.map((request) => <article key={request.id}><p>Request <code>{request.id}</code> · {request.receipt?.status ?? "unconfirmed"}</p>{request.receipt?.error_code && <p role="alert">{request.receipt.error_code}</p>}<button disabled={!inventory.ready || recovering} onClick={() => void recover(request)}>Resolve original receipt</button>{request.receipt?.worker_id ? <Link className="job-button" href={workerHref(request.scope[1], request.receipt.worker_id)}>Open original worker</Link> : <Link className="job-button" href={request.scope[2] ? workerHref(request.scope[1], request.scope[2]) : `/code/workers/new?project=${encodeURIComponent(request.scope[1])}`}>Open saved request context</Link>}</article>)}</section>}
    <p>{inventory.workers.length} loaded workers{attention ? " needing attention" : ""}; this is a loaded subset, not an authoritative total.</p>
    <section aria-label={attention ? "Workers needing attention" : "Worker list"}>{inventory.workers.map((worker) => <article className="job-card" key={worker.id}>
      <h2><Link href={workerHref(worker.project_id, worker.id)}>{worker.title}</Link></h2>
      <p>{worker.project_id} · {worker.hostname} · <strong>{workerStateLabel(worker.state)}</strong>{!inventory.ready && " · cached"}</p>
      <details><summary>Full responsibility and standing checklist</summary><p className="job-prose">{worker.description}</p><Link href={workerHref(worker.project_id, worker.id)}>Open standing checklist and backlog</Link></details>
      {worker.reason && <p className="job-notice">{worker.reason}</p>}
      <p>Last confirmed update: <time dateTime={worker.updated_at}>{new Date(worker.updated_at).toLocaleString()}</time></p>
      {worker.next_check_at && <p>Next eligible check: <time dateTime={worker.next_check_at}>{new Date(worker.next_check_at).toLocaleString()}</time></p>}
      {worker.current_item_id && <p>Current work item: <code>{worker.current_item_id}</code></p>}
      <div className="job-actions">
        <Link className="job-button" href={workerHref(worker.project_id, worker.id)}>Open worker</Link>
        {worker.attention_run_id && <Link className="job-button" href={workerHref(worker.project_id, worker.id, `/runs/${encodeURIComponent(worker.attention_run_id)}${worker.attention_question_id ? `/questions/${encodeURIComponent(worker.attention_question_id)}` : worker.state === "waiting_review" ? "/review" : ""}`)}>Open exact {worker.attention_question_id ? "question" : worker.state === "waiting_review" ? "review" : "run evidence"}</Link>}
        {worker.active_run_id && worker.active_run_id !== worker.attention_run_id && <Link className="job-button" href={workerHref(worker.project_id, worker.id, `/runs/${encodeURIComponent(worker.active_run_id)}`)}>Open current run and questions</Link>}
      </div>
    </article>)}</section>
    {!inventory.loading && !inventory.workers.length && <p>No accessible workers in this loaded view.</p>}
    {inventory.hasMore && <button disabled={inventory.loading || !inventory.ready} onClick={inventory.loadMore}>Load more workers</button>}
  </>;
}
