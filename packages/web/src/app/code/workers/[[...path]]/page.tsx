"use client";

import { Suspense, useEffect, useState } from "react";
import Link from "next/link";
import { useParams, usePathname, useRouter, useSearchParams } from "next/navigation";
import type { WorkerConversionSource } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { onWorkerCachePurge } from "@/lib/workerCache";
import { workerHref } from "@/lib/workers";
import { parseWorkerDestination } from "@/lib/workerDestination";
import { WorkerList } from "@/components/workers/WorkerList";
import { WorkerComposer } from "@/components/workers/WorkerComposer";
import { WorkerDetailScreen } from "@/components/workers/WorkerDetail";
import "../../jobs/[[...path]]/jobs.css";

function WorkersRoute() {
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
  useEffect(() => onWorkerCachePurge(() => { setAccessChanged(true); setCacheEpoch((value) => value + 1); }), []);
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
  const destination = parseWorkerDestination(params.path ?? []);
  const attention = query.get("view") === "attention";
  const source: WorkerConversionSource | undefined = query.get("source_job") ? { kind: "job", job_id: query.get("source_job")! } : undefined;
  return <div className="jobs-app">
    <a className="jobs-skip" href="#worker-main">Skip to worker content</a>
    <header className="jobs-header"><Link href="/" className="job-brand">APAS · Code</Link><nav aria-label="Code navigation"><Link href="/">Sessions</Link><Link href="/code/workers" aria-current={destination.kind === "list" && !attention ? "page" : undefined}>Workers</Link><Link href="/code/workers?view=attention" aria-current={destination.kind === "list" && attention ? "page" : undefined}>Attention</Link><Link href="/code/workers/new" aria-current={destination.kind === "new" ? "page" : undefined}>New worker</Link></nav></header>
    <main className="jobs-main" id="worker-main" key={`${account}:${cacheEpoch}`}>
      {!mounted || !token || !account ? <p role="status">Checking account…</p> : accessChanged ? <><h1 tabIndex={-1}>Worker access changed</h1><p role="alert">Protected cached content has been removed. Recheck current authority before loading any worker or earlier result.</p><button onClick={() => setAccessChanged(false)}>Check current access</button></> : destination.kind === "list" ? <WorkerList key={attention ? "attention" : "all"} attention={attention} /> : destination.kind === "new" ? <WorkerComposer key={`${query.get("project") ?? ""}:${query.get("source_job") ?? ""}`} initialProject={query.get("project")} source={source} onApproved={(id, project) => router.push(workerHref(project, id))} /> : destination.kind === "worker" ? <WorkerDetailScreen key={`${destination.id}:${destination.runId ?? ""}:${destination.questionId ?? ""}:${destination.review ?? false}`} id={destination.id} projectHint={query.get("project")} runId={destination.runId} review={destination.review} questionId={destination.questionId} /> : <><h1>Unknown worker destination</h1><p>No latest-run substitution was made.</p><Link href="/code/workers">Return to workers</Link></>}
    </main>
  </div>;
}
export default function WorkersPage() {
  return <Suspense fallback={<main aria-label="Workers" className="jobs-main"><p role="status">Loading worker destination…</p></main>}><WorkersRoute /></Suspense>;
}
