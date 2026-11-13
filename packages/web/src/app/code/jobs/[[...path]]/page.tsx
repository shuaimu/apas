"use client";

import { Suspense, useEffect, useState } from "react";
import Link from "next/link";
import { useParams, usePathname, useRouter, useSearchParams } from "next/navigation";
import { useStore } from "@/lib/store";
import { onJobCachePurge } from "@/lib/jobCache";
import { JobList } from "@/components/jobs/JobList";
import { JobDetailScreen } from "@/components/jobs/JobDetail";
import "./jobs.css";

function JobsRoute() {
  const params = useParams<{ path?: string[] }>();
  const query = useSearchParams();
  const pathname = usePathname();
  const router = useRouter();
  const account = useStore((state) => state.userId);
  const token = useStore((state) => state.token);
  const connect = useStore((state) => state.connect);
  const [cacheEpoch, setCacheEpoch] = useState(0);
  const [accessChanged, setAccessChanged] = useState(false);
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    setMounted(true);
    if (!token || !account) { router.replace(`/login?redirect=${encodeURIComponent(`${pathname}${query.size ? `?${query}` : ""}`)}`); return; }
    connect();
  }, [token, account, connect, router, pathname, query]);
  useEffect(() => onJobCachePurge(() => { setAccessChanged(true); setCacheEpoch((value) => value + 1); }), []);
  useEffect(() => {
    const viewport = window.visualViewport;
    const update = () => {
      document.documentElement.style.setProperty("--job-viewport-height", `${viewport?.height ?? window.innerHeight}px`);
      document.documentElement.style.setProperty("--job-viewport-offset", `${viewport?.offsetTop ?? 0}px`);
    };
    update(); viewport?.addEventListener("resize", update); viewport?.addEventListener("scroll", update);
    return () => { viewport?.removeEventListener("resize", update); viewport?.removeEventListener("scroll", update); document.documentElement.style.removeProperty("--job-viewport-height"); document.documentElement.style.removeProperty("--job-viewport-offset"); };
  }, []);
  useEffect(() => { document.querySelector<HTMLElement>(".jobs-main h1")?.focus(); }, [pathname]);
  const path = params.path ?? [];
  const attention = query.get("view") === "attention";
  const exactRunPath = path.length >= 3 && path[1] === "runs";
  const validDetail = path.length === 1 || path.length === 2 && path[1] === "review" || path.length === 3 && path[1] === "questions" || exactRunPath && (path.length === 3 || path.length === 4 && path[3] === "review" || path.length === 5 && path[3] === "questions");
  const requestedRun = exactRunPath ? path[2] : query.get("run_id") ?? query.get("run") ?? undefined;
  return <div className="jobs-app">
    <a className="jobs-skip" href="#job-main">Skip to job content</a>
    <header className="jobs-header"><Link href="/" className="job-brand">APAS · Code</Link><nav aria-label="Code navigation"><Link href="/">Sessions</Link><Link href="/code/workers">Workers</Link><Link href="/code/workers?view=attention">Attention</Link><Link href="/code/jobs" aria-current={!path.length ? "page" : undefined}>Earlier results</Link></nav></header>
    <main className="jobs-main" id="job-main" key={`${account}:${cacheEpoch}`}>
      {!mounted || !token || !account ? <p role="status">Checking account…</p> : accessChanged ? <><h1 tabIndex={-1}>Earlier-result access changed</h1><p role="alert">Cached content has been removed. Recheck current authority before loading more content.</p><button onClick={() => setAccessChanged(false)}>Check current access</button></> : !path.length ? <JobList key={attention ? "attention" : "all"} attention={attention} /> : path[0] === "new" && path.length === 1 ? <><h1>Standalone job creation is retired</h1><p>Earlier drafts are not recurring approvals. Open a worker draft to explicitly review its responsibility, provider and recurring allowance.</p><Link className="job-button" href={`/code/workers/new${query.get("project") ? `?project=${encodeURIComponent(query.get("project")!)}` : ""}`}>Open unapproved worker draft</Link></> : validDetail ? <JobDetailScreen key={path.join(":")} id={path[0]} projectHint={query.get("project")} runId={requestedRun} review={exactRunPath ? path[3] === "review" : path[1] === "review"} questionId={exactRunPath ? path[3] === "questions" ? path[4] : undefined : path[1] === "questions" ? path[2] : undefined} /> : <><h1>Unknown earlier-result destination</h1><Link href="/code/jobs">Return to earlier results</Link></>}
    </main>
  </div>;
}
export default function JobsPage() {
  return <Suspense fallback={<main aria-label="Jobs" className="jobs-main"><p role="status">Loading job destination…</p></main>}><JobsRoute /></Suspense>;
}
