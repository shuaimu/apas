"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import type { JobListResponse } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { jobFetch, jobHref, onJobMessage, resolveJobRequest } from "@/lib/jobs";
import { jobCacheRevision, jobRequests, readJobRecord, writeJobRecord } from "@/lib/jobCache";
import type { JobScope, SavedJobRequest } from "@/lib/jobCache";
import { useJobOnline } from "@/lib/useJobs";
import { JobError, Usage } from "./JobElements";

export function JobList({ attention }: { attention: boolean }) {
  const account = useStore((state) => state.userId)!;
  const online = useJobOnline();
  const [page, setPage] = useState<JobListResponse>();
  const [cursor, setCursor] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [requests, setRequests] = useState<SavedJobRequest[]>([]);
  const [fresh, setFresh] = useState(false);
  const refresh = useCallback(async () => {
    setLoading(true); setFresh(false);
    const scope: JobScope = [account, "", ""];
    const key = `${attention ? "attention" : "all"}:${cursor}`;
    const revision = jobCacheRevision();
    try {
      const cache = await readJobRecord<JobListResponse>(scope, "list", key);
      if (cache && revision === jobCacheRevision()) setPage(cache);
      const pending = (await jobRequests(account)).filter((request) => !request.receipt || request.receipt.status === "pending");
      if (revision === jobCacheRevision()) setRequests(pending);
      if (!online) return;
      const response = await jobFetch<JobListResponse>(`/projects/jobs?limit=40${attention ? "&status=attention" : ""}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`, "list");
      await writeJobRecord(scope, "list", response, key, revision);
      if (revision === jobCacheRevision()) { setPage(response); setFresh(true); setError(""); }
      // Only receipt reads are automatic; a lost POST is never replayed here.
      const resolved = await Promise.all(pending.map(async (request) => {
        try { return await resolveJobRequest(request); } catch { return request; }
      }));
      if (revision === jobCacheRevision()) setRequests(resolved);
    } catch (failure) { if (revision === jobCacheRevision()) setError(failure instanceof Error ? failure.message : "Jobs could not be loaded."); }
    finally { setLoading(false); }
  }, [account, attention, cursor, online]);
  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    const remove = onJobMessage((message) => { if (online && message.type === "job_changed") void refresh(); });
    const timer = setInterval(() => { if (online) void refresh(); }, 20_000);
    return () => { remove(); clearInterval(timer); };
  }, [refresh, online]);
  return <>
    <h1 tabIndex={-1}>{attention ? "Attention" : "Autonomous jobs"}</h1>
    <p>{attention ? "Decisions, recovery and review that need your attention." : "Bounded coding work, independent verification and draft pull requests — separate from interactive sessions."}</p>
    <div className="job-actions"><Link href="/code/jobs/new" className="job-button">New job</Link><button disabled={loading || !online} onClick={() => void refresh()}>Refresh jobs</button></div>
    {!fresh && <p className="job-notice" role="status">{online ? "Checking current access and durable job state…" : "Offline or reconnecting. Cached jobs are stale and read-only."}</p>}
    <JobError message={error} />
    {requests.length > 0 && <section className="job-card"><h2>Interrupted request recovery</h2><p>These saved identities are resolved before another submission. No request is replayed automatically.</p>{requests.map((request) => <div key={request.id}><p>Request <code>{request.id}</code> · {request.receipt?.status ?? "unconfirmed"}</p>{request.receipt?.job_id ? <Link className="job-button" href={jobHref(request.scope[1], request.receipt.job_id)}>Open original job</Link> : <Link className="job-button" href={request.scope[2] ? jobHref(request.scope[1], request.scope[2]) : `/code/jobs/new?project=${encodeURIComponent(request.scope[1])}`}>Resolve saved request</Link>}</div>)}</section>}
    <section aria-label={attention ? "Jobs needing attention" : "Job list"}>
      {page?.jobs.map((job) => <article className="job-card" key={job.id}><h2><Link href={jobHref(job.project_id, job.id)}>{job.title}</Link></h2><p>{job.project_name} · {job.hostname}</p><p><strong>{job.state.replaceAll("_", " ")}</strong>{job.attention_count ? ` · ${job.attention_count} attention items` : ""}</p>{job.reason && <p>{job.reason}</p>}<Usage usage={job.usage} limits={job.limits} /><p>Updated <time dateTime={job.updated_at}>{new Date(job.updated_at).toLocaleString()}</time></p><Link className="job-button" href={jobHref(job.project_id, job.id)}>Open job details</Link></article>)}
      {page && !page.jobs.length && <p className="job-notice">{attention ? "No jobs currently need your attention." : "No authorized jobs on this page. Create a job when an operator has enabled a qualified target."}</p>}
    </section>
    <div className="job-actions"><button disabled={loading || !history.length} onClick={() => { setPage(undefined); setCursor(history[history.length - 1]); setHistory((items) => items.slice(0, -1)); }}>Previous jobs page</button><button disabled={loading || !online || !page?.next_cursor} onClick={() => { setHistory((items) => [...items, cursor]); setCursor(page!.next_cursor!); setPage(undefined); }}>Next jobs page</button></div>
  </>;
}
