"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Bot, LayoutDashboard, X } from "lucide-react";
import type { JobListResponse } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { useProjectJobs } from "@/lib/useProjectJobs";
import { jobCacheRevision, onJobCachePurge, readJobRecord, writeJobRecord } from "@/lib/jobCache";
import type { JobScope, SavedJob } from "@/lib/jobCache";
import { JobComposer } from "./JobComposer";
import type { JobProjectContext } from "./JobComposer";
import { JobDetailScreen } from "./JobDetail";
import type { JobDestination } from "./JobDetail";
import { JobError } from "./JobElements";
import "@/app/code/jobs/[[...path]]/jobs.css";

type WorkspaceView = { kind: "interactive" | "compose" | "history" } | ({ kind: "job"; id: string } & JobDestination);
type Preferences = { hidden: string[]; opened: string[]; view: WorkspaceView };
type JobSummary = JobListResponse["jobs"][number];
type ObservedJob = { job: JobSummary; ready: boolean };
const initialPreferences = (): Preferences => ({ hidden: [], opened: [], view: { kind: "interactive" } });

interface WorkspaceControls {
  active: boolean;
  tabs: ReactNode;
  overviewTab: ReactNode;
  content: ReactNode;
  addAutonomous?: () => void;
  unavailableReason?: string;
  selectInteractive: () => void;
}

/** Job views share presentation with panes, never their identity or lifecycle. */
export function ProjectJobsWorkspace({ project, paneSelectionRequested, initialOverview = false, renderOverviewPanels, children }: {
  project?: JobProjectContext;
  paneSelectionRequested: boolean;
  initialOverview?: boolean;
  renderOverviewPanels?: (selectInteractive: () => void) => ReactNode;
  children: (controls: WorkspaceControls) => ReactNode;
}) {
  const account = useStore((state) => state.userId);
  const scope = useMemo<JobScope>(() => [account ?? "", project?.id ?? "", ""], [account, project?.id]);
  const initialView = useRef<"history" | "interactive">(initialOverview ? "history" : "interactive");
  const [preferences, setPreferences] = useState<Preferences>(() => ({
    ...initialPreferences(), view: { kind: initialView.current },
  }));
  const [viewedJobs, setViewedJobs] = useState<Map<string, ObservedJob>>(() => new Map());
  const [loaded, setLoaded] = useState(false);
  const [storageError, setStorageError] = useState("");
  const [accessChanged, setAccessChanged] = useState(false);
  const [epoch, setEpoch] = useState(0);
  const selectedBeforeRestore = useRef(false);
  const loadedRevision = useRef(jobCacheRevision());
  const selectedTab = useRef<HTMLButtonElement>(null);
  const contentRef = useRef<HTMLElement>(null);
  const inventory = useProjectJobs(accessChanged ? null : project?.id);
  const view = preferences.view;
  const active = view.kind !== "interactive";

  useEffect(() => onJobCachePurge(() => {
    setAccessChanged(true);
    setLoaded(false);
    setPreferences((previous) => ({ ...initialPreferences(), view: previous.view }));
    setViewedJobs(new Map());
  }), []);
  useEffect(() => {
    let current = true;
    const revision = jobCacheRevision();
    if (accessChanged) return;
    setLoaded(false);
    void readJobRecord<Preferences>(scope, "draft", "project-workspace").then(async (saved) => {
      // An explicitly opened result can be older than the bounded list page.
      // Restore its cached label without scanning history or fetching every job.
      const restored = new Map<string, ObservedJob>();
      await Promise.all((saved?.opened ?? []).map(async (id) => {
        const snapshot = await readJobRecord<SavedJob>([scope[0], scope[1], id], "snapshot");
        const job = snapshot?.detail.job;
        if (job?.id === id && job.project_id === scope[1]) restored.set(id, { job, ready: false });
      }));
      if (!current || revision !== jobCacheRevision()) return;
      loadedRevision.current = revision;
      setPreferences((previous) => {
        const restoredView = saved?.view ?? previous.view;
        // Migrate the old Overview pane preference, without replacing a saved
        // job/composer destination or a selection made during cache restoration.
        const view = initialView.current === "history" && restoredView.kind === "interactive"
          ? { kind: "history" as const } : restoredView;
        return { ...(saved ?? initialPreferences()), view: selectedBeforeRestore.current ? previous.view : view };
      });
      setViewedJobs(restored);
      setStorageError("");
      setLoaded(true);
    }).catch((failure: unknown) => {
      if (!current || revision !== jobCacheRevision()) return;
      loadedRevision.current = revision;
      setLoaded(true);
      setStorageError(failure instanceof Error ? failure.message : "Project view preferences could not be restored.");
    });
    return () => { current = false; };
  }, [scope, epoch, accessChanged]);
  useEffect(() => {
    if (!loaded || !account || !project?.id || accessChanged) return;
    let current = true;
    void writeJobRecord(scope, "draft", preferences, "project-workspace", loadedRevision.current)
      .catch((failure: unknown) => { if (current) setStorageError(failure instanceof Error ? failure.message : "Project view preferences were not saved."); });
    return () => { current = false; };
  }, [account, project?.id, scope, preferences, loaded, accessChanged]);
  useEffect(() => {
    if (!paneSelectionRequested) return;
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous, view: { kind: "interactive" } }));
  }, [paneSelectionRequested]);
  useEffect(() => {
    if (!initialOverview || selectedBeforeRestore.current) return;
    initialView.current = "history";
    setPreferences((previous) => previous.view.kind === "interactive"
      ? { ...previous, view: { kind: "history" } } : previous);
  }, [initialOverview]);
  useEffect(() => {
    selectedTab.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
    if (active) {
      contentRef.current?.focus({ preventScroll: true });
      if (contentRef.current) contentRef.current.scrollTop = 0;
    }
  }, [view, active]);
  const rememberSummary = useCallback((job: JobSummary, ready: boolean) => {
    if (accessChanged || job.project_id !== scope[1] || loadedRevision.current !== jobCacheRevision()) return;
    setViewedJobs((previous) => {
      const existing = previous.get(job.id);
      const latest = existing && existing.job.state_version > job.state_version ? existing.job : job;
      const fresh = latest === job && ready;
      if (existing?.job === latest && existing.ready === fresh) return previous;
      return new Map(previous).set(job.id, { job: latest, ready: fresh });
    });
  }, [scope, accessChanged]);

  const navigate = (next: WorkspaceView) => {
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous, view: next }));
  };
  const selectInteractive = () => navigate({ kind: "interactive" });
  const selectOverview = () => {
    navigate({ kind: "history" });
    inventory.refresh();
  };
  const openJob = (id: string) => {
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({
      hidden: previous.hidden.filter((hidden) => hidden !== id),
      opened: previous.opened.includes(id) ? previous.opened : [...previous.opened, id],
      view: { kind: "job", id },
    }));
    inventory.refresh();
  };
  const hideJob = (id: string) => {
    setPreferences((previous) => ({
      hidden: previous.hidden.includes(id) ? previous.hidden : [...previous.hidden, id],
      opened: previous.opened.filter((opened) => opened !== id),
      view: previous.view.kind === "job" && previous.view.id === id ? { kind: "interactive" } : previous.view,
    }));
  };
  const visible = new Map<string, JobSummary | undefined>();
  if (loaded && !accessChanged) {
    // Surface current work and attention automatically; ordinary archived
    // results belong in history until the user opens them.
    for (const job of inventory.jobs) {
      const retainedResult = ["review_ready", "cancelled", "failed"].includes(job.state);
      const observed = viewedJobs.get(job.id);
      if (!preferences.hidden.includes(job.id) && (!retainedResult || job.attention_count > 0 || preferences.opened.includes(job.id))) visible.set(job.id, observed && observed.job.state_version > job.state_version ? observed.job : job);
    }
    for (const id of preferences.opened) if (!visible.has(id) && !preferences.hidden.includes(id)) visible.set(id, viewedJobs.get(id)?.job);
    if (view.kind === "job" && !visible.has(view.id)) visible.set(view.id, viewedJobs.get(view.id)?.job);
  }
  const unavailableReason = !account || !project ? "Project identity is not available yet. Reconnect or reopen this project." : !loaded ? "Restoring project job views…" : undefined;
  const tabs = [...visible].map(([id, job]) => {
    const selected = view.kind === "job" && view.id === id;
    const observed = viewedJobs.get(id);
    const fresh = (selected && observed?.ready && observed.job.state_version === job?.state_version)
      || (inventory.ready && inventory.jobs.some((listed) => listed.id === id && listed.state_version === job?.state_version));
    const title = job?.title ?? "Autonomous job";
    return <div key={id} className={`flex shrink-0 items-center border-r border-gray-200 dark:border-gray-700 ${selected ? "bg-white dark:bg-gray-900 border-b-2 border-b-violet-500" : "text-gray-500 dark:text-gray-400"}`}>
      <button ref={selected ? selectedTab : undefined} type="button" aria-pressed={selected} aria-label={`Autonomous job: ${title}`} onClick={() => openJob(id)} className="flex min-h-11 max-w-56 items-center gap-2 px-3 text-left hover:bg-gray-200 dark:hover:bg-gray-700/50" title={`${title} · ${job ? statusLabel(job) : "Checking status"}`}>
        <Bot className="h-4 w-4 shrink-0 text-violet-500" aria-hidden="true" />
        <span className="min-w-0"><span className="block truncate text-sm font-medium">{title}</span><span className={`inline-block rounded px-1 text-[11px] ${job?.state === "needs_input" ? "bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-200" : job?.state === "review_ready" ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-200" : "bg-violet-100 text-violet-800 dark:bg-violet-950 dark:text-violet-200"}`}>{!fresh && job ? "Cached · " : ""}{job ? statusLabel(job) : "Checking status"}</span></span>
      </button>
      <button type="button" onClick={() => hideJob(id)} className="flex h-11 w-11 shrink-0 items-center justify-center hover:bg-gray-200 dark:hover:bg-gray-700/50" aria-label={`Hide autonomous job: ${title}`} title="Hide this view only. Reopen from Overview; execution is not cancelled."><X className="h-3.5 w-3.5" aria-hidden="true" /></button>
    </div>;
  });
  const overviewTab = <button
    ref={view.kind === "history" ? selectedTab : undefined}
    type="button"
    onClick={selectOverview} aria-label="Project overview" aria-pressed={view.kind === "history"}
    title="Project overview — autonomous jobs, descriptions and status"
    className={`flex min-h-11 shrink-0 items-center gap-2 border-r border-gray-200 px-3 text-sm font-medium dark:border-gray-700 ${view.kind === "history" ? "border-b-2 border-b-violet-500 bg-white text-gray-900 dark:bg-gray-900 dark:text-gray-100" : "text-gray-600 hover:bg-gray-200 dark:text-gray-300 dark:hover:bg-gray-700/50"}`}
  ><LayoutDashboard className="h-4 w-4" aria-hidden="true" />Overview</button>;
  const content = active && <section ref={contentRef} tabIndex={-1} aria-label={view.kind === "history" ? "Project overview" : "Project autonomous work"} className="jobs-app jobs-embedded">
    <div className="jobs-main">
      <div className="job-actions"><button type="button" onClick={selectInteractive}>Back to interactive agents</button>{view.kind !== "history" && <button type="button" onClick={selectOverview}>Project overview</button>}{view.kind === "job" && <button type="button" onClick={() => hideJob(view.id)}>Hide this job view</button>}</div>
      {view.kind === "job" && <p className="text-sm">Hiding this view does not stop the job. Use its Cancel control to request cancellation.</p>}
      <JobError message={storageError} />
      {!project || !account ? <>{view.kind === "history" && <h1>Project overview</h1>}<p className="job-notice">{unavailableReason}</p></> : accessChanged ? <>
        <h1 tabIndex={-1}>Job access changed</h1><p role="alert" className="job-notice">Cached job content has been removed. Recheck current project authority before loading it again.</p><button type="button" onClick={() => { selectedBeforeRestore.current = true; setEpoch((value) => value + 1); setAccessChanged(false); }}>Check current access</button>
      </> : <div key={epoch}>
        {view.kind === "compose" ? <JobComposer projectContext={project} onOpenJob={openJob} /> : view.kind === "job" ? <JobDetailScreen key={view.id} id={view.id} projectHint={project.id} review={view.review} questionId={view.questionId} onNavigate={(destination) => navigate({ kind: "job", id: view.id, ...destination })} onSummary={rememberSummary} /> : <>
          <h1 tabIndex={-1}>Project overview</h1>
          <p>{project.name} · Autonomous agents and their assigned work. Hidden views and completed results remain here.</p>
          <div className="job-actions"><button type="button" disabled={!loaded} onClick={() => navigate({ kind: "compose" })}>New autonomous agent</button><button type="button" disabled={inventory.loading} onClick={inventory.refresh}>Refresh project jobs</button></div>
          <h2>Autonomous agents</h2>
          {!inventory.ready && <p role="status" className="job-notice">{inventory.loading ? "Checking current project access and job state…" : "Offline or reconnecting. Cached jobs are stale and read-only."}</p>}
          <JobError message={inventory.error} />
          {inventory.jobs.map((listed) => {
            const observed = viewedJobs.get(listed.id);
            const job = observed && observed.job.state_version > listed.state_version ? observed.job : listed;
            return <OverviewJobCard
              key={job.id} job={job} description={listed.description}
              fresh={inventory.ready && job.state_version === listed.state_version}
              hidden={preferences.hidden.includes(job.id)} onOpen={() => openJob(job.id)}
            />;
          })}
          {inventory.ready && !inventory.jobs.length && <p className="job-notice">No autonomous jobs in this project yet. A qualified worker and job admission authority are required to start one.</p>}
          {inventory.hasMore && <><p>Older jobs remain. Load the next page to include them in this overview.</p><button type="button" disabled={inventory.loading || !inventory.ready} onClick={inventory.loadMore}>Load older project jobs</button></>}
        </>}
      </div>}
      {view.kind === "history" && !accessChanged && renderOverviewPanels?.(selectInteractive)}
    </div>
  </section>;
  return children({ active, tabs, overviewTab, content, addAutonomous: account && project && loaded && !accessChanged ? () => navigate({ kind: "compose" }) : undefined, unavailableReason: accessChanged ? "Job access changed. Open Overview to recheck access." : unavailableReason, selectInteractive });
}

function statusLabel(job: JobSummary): string {
  const phase = job.state === "needs_input" ? "Pending answer" : job.state.replaceAll("_", " ");
  return `${phase}${job.attention_count ? ` · ${job.attention_count} attention` : ""}`;
}

function OverviewJobCard({ job, description, fresh, hidden, onOpen }: {
  job: JobSummary;
  description?: string;
  fresh: boolean;
  hidden: boolean;
  onOpen: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const descriptionId = `job-description-${job.id}`;
  return <article className="job-card" aria-labelledby={`job-title-${job.id}`}>
    <h3 id={`job-title-${job.id}`}>{job.title}</h3>
    <p><strong className="rounded bg-violet-100 px-2 py-1 text-violet-900 dark:bg-violet-950 dark:text-violet-200">{!fresh ? "Cached · " : ""}{statusLabel(job)}</strong>{hidden && <span> · View hidden</span>}</p>
    {job.reason && <p>{job.reason}</p>}
    <h4>Job description</h4>
    {description ? <>
      <p id={descriptionId} className={`job-prose ${expanded ? "" : "line-clamp-3"}`}>{description}</p>
      <button type="button" aria-expanded={expanded} aria-controls={descriptionId} onClick={() => setExpanded((value) => !value)}>
        {expanded ? "Collapse description" : "Show full description"}
      </button>
    </> : <p className="job-notice">The full description is not in this cached record. Refresh or open the job to retrieve its approved task.</p>}
    <dl className="job-facts">
      <div><dt>Host</dt><dd>{job.hostname}</dd></div>
      <div><dt>Last confirmed update</dt><dd><time dateTime={job.updated_at}>{new Date(job.updated_at).toLocaleString()}</time></dd></div>
    </dl>
    <button type="button" onClick={onOpen} aria-label={`Open ${job.title}`}>Open job</button>
  </article>;
}
