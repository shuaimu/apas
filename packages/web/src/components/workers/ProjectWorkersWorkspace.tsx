"use client";

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { Bot, LayoutDashboard, X } from "lucide-react";
import type { JobSummary, WorkerConversionSource, WorkerSummary } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { useProjectWorkers } from "@/lib/useProjectWorkers";
import { useProjectJobs } from "@/lib/useProjectJobs";
import { readJobRecord } from "@/lib/jobCache";
import type { SavedJob } from "@/lib/jobCache";
import { workerCacheRevision, onWorkerCachePurge, readWorkerRecord, writeWorkerRecord } from "@/lib/workerCache";
import type { SavedWorker, WorkerScope } from "@/lib/workerCache";
import { WorkerComposer } from "./WorkerComposer";
import type { JobProjectContext } from "./WorkerComposer";
import { WorkerDetailScreen } from "./WorkerDetail";
import type { WorkerDestination } from "./WorkerDetail";
import { workerStateLabel } from "./WorkerList";
import { JobDetailScreen } from "../jobs/JobDetail";
import type { JobDestination } from "../jobs/JobDetail";
import { JobError } from "../jobs/JobElements";
import "@/app/code/jobs/[[...path]]/jobs.css";

type WorkspaceView = { kind: "interactive" | "overview" }
  | { kind: "compose"; source?: WorkerConversionSource }
  | ({ kind: "worker"; id: string } & WorkerDestination)
  | ({ kind: "result"; id: string } & JobDestination);
type Preferences = { hidden: string[]; opened: string[]; results: string[]; hiddenResults: string[]; resultDestinations?: Record<string, JobDestination>; view: WorkspaceView };
type LegacyPreferences = { hidden: string[]; opened: string[]; view: { kind: "interactive" | "compose" | "history" } | ({ kind: "job"; id: string } & JobDestination) };
type ObservedWorker = { worker: WorkerSummary; ready: boolean };
const initialPreferences = (): Preferences => ({ hidden: [], opened: [], results: [], hiddenResults: [], view: { kind: "interactive" } });

interface WorkspaceControls {
  active: boolean;
  tabs: ReactNode;
  overviewTab: ReactNode;
  content: ReactNode;
  addAutonomous?: (source?: WorkerConversionSource) => void;
  unavailableReason?: string;
  selectInteractive: () => void;
}
interface WorkspaceProps {
  project?: JobProjectContext;
  paneSelectionRequested: boolean;
  initialOverview?: boolean;
  renderOverviewPanels?: (selectInteractive: () => void) => ReactNode;
  children: (controls: WorkspaceControls) => ReactNode;
}

/** Identity changes reset protected views, never the caller's mounted terminals. */
export function ProjectWorkersWorkspace(props: WorkspaceProps) {
  const account = useStore((state) => state.userId);
  return <WorkerWorkspace key={`${account ?? ""}:${props.project?.id ?? ""}`} {...props} />;
}

function WorkerWorkspace({ project, paneSelectionRequested, initialOverview = false, renderOverviewPanels, children }: WorkspaceProps) {
  const account = useStore((state) => state.userId);
  const scope = useMemo<WorkerScope>(() => [account ?? "", project?.id ?? "", ""], [account, project?.id]);
  const revision = useSyncExternalStore(onWorkerCachePurge, workerCacheRevision, () => 0);
  const loadedRevision = useRef(revision);
  const [preferences, setPreferences] = useState<Preferences>(() => ({ ...initialPreferences(), view: { kind: initialOverview ? "overview" : "interactive" } }));
  const [viewed, setViewed] = useState<Map<string, ObservedWorker>>(() => new Map());
  const [results, setResults] = useState<Map<string, JobSummary>>(() => new Map());
  const [loaded, setLoaded] = useState(false);
  const [storageError, setStorageError] = useState("");
  const [accessChanged, setAccessChanged] = useState(false);
  const [epoch, setEpoch] = useState(0);
  const [showResults, setShowResults] = useState(false);
  const selectedBeforeRestore = useRef(false);
  const overviewRequested = useRef(initialOverview);
  const selectedTab = useRef<HTMLButtonElement>(null);
  const contentRef = useRef<HTMLElement>(null);
  const blocked = accessChanged || loadedRevision.current !== revision;
  const inventory = useProjectWorkers(blocked || !project ? null : project.id);
  const history = useProjectJobs(!blocked && showResults ? project?.id : null);
  const view = preferences.view;
  const active = view.kind !== "interactive";

  useEffect(() => onWorkerCachePurge(() => {
    setAccessChanged(true);
    setLoaded(false);
    setPreferences((previous) => ({ ...initialPreferences(), view: previous.view.kind === "interactive" ? previous.view : { kind: "overview" } }));
    setViewed(new Map());
    setResults(new Map());
    setShowResults(false);
  }), []);
  useEffect(() => {
    if (accessChanged || !account || !scope[1]) return;
    let current = true;
    const generation = workerCacheRevision();
    const valid = () => current && generation === workerCacheRevision() && useStore.getState().userId === account;
    setLoaded(false);
    void (async () => {
      let saved = await readWorkerRecord<Preferences>(scope, "draft", "project-workspace");
      if (!saved) {
        const legacy = await readJobRecord<LegacyPreferences>(scope, "draft", "project-workspace");
        if (legacy) {
          const old = legacy.view;
          saved = { ...initialPreferences(), results: legacy.opened, hiddenResults: legacy.hidden,
            resultDestinations: old.kind === "job" ? { [old.id]: { runId: old.runId, questionId: old.questionId, review: old.review } } : {},
            view: old.kind === "job" ? { ...old, kind: "result" } : old.kind === "history" ? { kind: "overview" } : old.kind === "compose" ? { kind: "compose" } : { kind: "interactive" } };
        }
      }
      if (saved?.view.kind === "compose") {
        saved = { ...saved, view: { kind: "compose", source: saved.view.source?.kind === "job" ? saved.view.source : undefined } };
      }
      const restored = new Map<string, ObservedWorker>();
      const earlier = new Map<string, JobSummary>();
      await Promise.all((saved?.opened ?? []).map(async (id) => {
        const snapshot = await readWorkerRecord<SavedWorker>([scope[0], scope[1], id], "snapshot");
        const worker = snapshot?.detail.worker;
        if (worker?.id === id && worker.project_id === scope[1]) restored.set(id, { worker, ready: false });
      }));
      await Promise.all((saved?.results ?? []).map(async (id) => {
        const snapshot = await readJobRecord<SavedJob>([scope[0], scope[1], id], "snapshot");
        const job = snapshot?.detail.job;
        if (job?.id === id && job.project_id === scope[1]) earlier.set(id, job);
      }));
      if (!valid()) return;
      loadedRevision.current = generation;
      setPreferences((previous) => {
        const restored = saved?.view ?? previous.view;
        const destination = overviewRequested.current && restored.kind === "interactive" ? { kind: "overview" as const } : restored;
        return { ...(saved ?? initialPreferences()), view: selectedBeforeRestore.current ? previous.view : destination };
      });
      setViewed(restored);
      setResults(earlier);
      setStorageError("");
      setLoaded(true);
    })().catch((failure: unknown) => {
      if (!valid()) return;
      loadedRevision.current = generation;
      setLoaded(true);
      setStorageError(failure instanceof Error ? failure.message : "Project views could not be restored.");
    });
    return () => { current = false; };
  }, [scope, epoch, accessChanged, account]);
  useEffect(() => {
    if (!loaded || !account || !scope[1] || blocked) return;
    let current = true;
    const generation = loadedRevision.current;
    void writeWorkerRecord(scope, "draft", preferences, "project-workspace", generation).catch((failure: unknown) => {
      if (current && generation === workerCacheRevision()) setStorageError(failure instanceof Error ? failure.message : "Project views were not saved.");
    });
    return () => { current = false; };
  }, [loaded, account, blocked, preferences, scope]);
  useEffect(() => {
    if (!paneSelectionRequested) return;
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous, view: { kind: "interactive" } }));
  }, [paneSelectionRequested]);
  useEffect(() => {
    if (!initialOverview || selectedBeforeRestore.current) return;
    overviewRequested.current = true;
    setPreferences((previous) => previous.view.kind === "interactive" ? { ...previous, view: { kind: "overview" } } : previous);
  }, [initialOverview]);
  useEffect(() => {
    selectedTab.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
    if (active) { contentRef.current?.focus({ preventScroll: true }); if (contentRef.current) contentRef.current.scrollTop = 0; }
  }, [view, active]);
  const rememberSummary = useCallback((worker: WorkerSummary, ready: boolean) => {
    if (blocked || worker.project_id !== scope[1] || revision !== workerCacheRevision() || useStore.getState().userId !== account) return;
    setViewed((previous) => {
      const existing = previous.get(worker.id);
      const latest = existing && existing.worker.state_version > worker.state_version ? existing.worker : worker;
      const fresh = latest === worker && ready;
      if (existing?.worker === latest && existing.ready === fresh) return previous;
      return new Map(previous).set(worker.id, { worker: latest, ready: fresh });
    });
  }, [blocked, scope, account, revision]);
  const navigate = (next: WorkspaceView) => {
    if (next.kind !== "interactive" && next.kind !== "overview" && (revision !== workerCacheRevision() || useStore.getState().userId !== account)) return;
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous, view: next,
      ...(next.kind === "result" ? { resultDestinations: { ...previous.resultDestinations, [next.id]: { runId: next.runId, questionId: next.questionId, review: next.review } } } : {}),
    }));
  };
  const selectInteractive = () => navigate({ kind: "interactive" });
  const selectOverview = () => { navigate({ kind: "overview" }); inventory.refresh(); };
  const openWorker = (id: string) => {
    if (revision !== workerCacheRevision() || useStore.getState().userId !== account) return;
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous, hidden: previous.hidden.filter((value) => value !== id), opened: [...new Set([...previous.opened, id])], view: { kind: "worker", id } }));
  };
  const openResult = (id: string) => {
    if (revision !== workerCacheRevision() || useStore.getState().userId !== account) return;
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous, hiddenResults: previous.hiddenResults.filter((value) => value !== id), results: [...new Set([...previous.results, id])], view: { kind: "result", id, ...previous.resultDestinations?.[id] } }));
  };
  const hide = (kind: "worker" | "result", id: string) => {
    selectedBeforeRestore.current = true;
    setPreferences((previous) => ({ ...previous,
      ...(kind === "worker" ? { hidden: [...new Set([...previous.hidden, id])], opened: previous.opened.filter((value) => value !== id) } : { hiddenResults: [...new Set([...previous.hiddenResults, id])], results: previous.results.filter((value) => value !== id) }),
      view: previous.view.kind === kind && previous.view.id === id ? { kind: "overview" } : previous.view,
    }));
  };
  const visible = new Map<string, WorkerSummary | undefined>();
  if (loaded && !blocked) {
    for (const worker of inventory.workers) if (!preferences.hidden.includes(worker.id)) {
      const observed = viewed.get(worker.id)?.worker;
      visible.set(worker.id, observed && observed.state_version > worker.state_version ? observed : worker);
    }
    for (const id of preferences.opened) if (!visible.has(id) && !preferences.hidden.includes(id)) visible.set(id, viewed.get(id)?.worker);
    if (view.kind === "worker" && !visible.has(view.id)) visible.set(view.id, viewed.get(view.id)?.worker);
  }
  const tabs = <>{[...visible].map(([id, worker]) => {
    const selected = view.kind === "worker" && view.id === id;
    const fresh = (selected && viewed.get(id)?.ready) || (inventory.ready && inventory.workers.some((row) => row.id === id && row.state_version === worker?.state_version));
    const title = worker?.title ?? "Autonomous worker";
    return <div key={id} className="flex shrink-0 items-center border-r border-gray-200 dark:border-gray-700">
      <button ref={selected ? selectedTab : undefined} type="button" aria-pressed={selected} aria-label={`Autonomous worker: ${title}`} onClick={() => openWorker(id)} className="flex min-h-11 max-w-56 items-center gap-2 px-3 text-left">
        <Bot className="h-4 w-4 shrink-0 text-violet-500" aria-hidden="true" /><span className="min-w-0"><span className="block truncate text-sm font-medium">{title}</span><span className="block text-xs">{!fresh && worker ? "Cached · " : ""}{worker ? statusLabel(worker) : "Checking status"}</span></span>
      </button><button type="button" onClick={() => hide("worker", id)} className="flex h-11 w-11 items-center justify-center" aria-label={`Hide autonomous worker: ${title}`} title="Hide this view only; the worker continues under its approved mandate."><X className="h-3.5 w-3.5" aria-hidden="true" /></button>
    </div>;
  })}{loaded && !blocked && [...new Set([...preferences.results, ...(view.kind === "result" ? [view.id] : [])])].filter((id) => !preferences.hiddenResults.includes(id)).map((id) => <div key={`result-${id}`} className="flex shrink-0 items-center">
    <button type="button" className="min-h-11 px-3 text-left" aria-pressed={view.kind === "result" && view.id === id} onClick={() => openResult(id)}>Earlier result: {results.get(id)?.title ?? id}<span className="block text-xs">Cached · Read-only</span></button>
    <button type="button" className="h-11 w-11" aria-label={`Hide earlier result: ${results.get(id)?.title ?? id}`} onClick={() => hide("result", id)}><X className="h-4 w-4" /></button>
  </div>)}</>;
  const unavailableReason = blocked ? "Worker access changed. Open Overview to recheck access." : !account || !project ? "Project identity is not available yet. Reconnect or reopen this project." : !loaded ? "Restoring project worker views…" : undefined;
  const overviewTab = <button ref={view.kind === "overview" ? selectedTab : undefined} type="button" onClick={selectOverview} aria-label="Project overview" aria-pressed={view.kind === "overview"} className="flex min-h-11 shrink-0 items-center gap-2 border-r border-gray-200 px-3 text-sm font-medium dark:border-gray-700"><LayoutDashboard className="h-4 w-4" aria-hidden="true" />Overview</button>;
  const content = active && <section ref={contentRef} tabIndex={-1} aria-label={view.kind === "overview" ? "Project overview" : "Project autonomous work"} className="jobs-app jobs-embedded"><div className="jobs-main">
    <div className="job-actions"><button type="button" onClick={selectInteractive}>Back to interactive agents</button>{view.kind !== "overview" && <button type="button" onClick={selectOverview}>Project overview</button>}</div>
    <JobError message={storageError} />
    {!project || !account ? <p className="job-notice">{unavailableReason}</p> : blocked ? <><h1>Worker access changed</h1><p role="alert" className="job-notice">Cached worker content and earlier results have been removed. Check current project authority before loading again.</p><button type="button" onClick={() => { loadedRevision.current = workerCacheRevision(); setEpoch((value) => value + 1); setAccessChanged(false); }}>Check current access</button></> : !loaded ? <p role="status">Restoring project worker views…</p> : <div key={epoch}>
      {view.kind === "compose" ? <WorkerComposer project={project} source={view.source} onApproved={(id, projectId) => { if (projectId === scope[1]) openWorker(id); }} /> : view.kind === "worker" ? <>
        <p>Hiding a view does not pause or stop the worker.</p><button type="button" onClick={() => hide("worker", view.id)}>Hide this worker view</button>
        <WorkerDetailScreen key={view.id} id={view.id} projectHint={project.id} runId={view.runId} questionId={view.questionId} review={view.review} onNavigate={(destination) => navigate({ kind: "worker", id: view.id, ...destination })} onSummary={rememberSummary} />
      </> : view.kind === "result" ? <><p className="job-notice">Earlier result · Read-only. This job is not a persistent worker.</p><JobDetailScreen key={view.id} id={view.id} projectHint={project.id} runId={view.runId} questionId={view.questionId} review={view.review} onNavigate={(destination) => navigate({ kind: "result", id: view.id, ...destination })} /></> : <>
        <h1>Project overview</h1><p>{project.name} · Persistent responsibilities, including idle, paused and hidden workers.</p>
        <div className="job-actions"><button type="button" onClick={() => navigate({ kind: "compose" })}>New autonomous worker</button><button type="button" disabled={inventory.loading} onClick={inventory.refresh}>Refresh project workers</button></div>
        <h2>Autonomous workers</h2>
        {!inventory.ready && <p role="status" className="job-notice">{inventory.loading ? "Checking current project access and worker state…" : "Offline or reconnecting. Cached workers are stale and read-only."}</p>}
        <JobError message={inventory.error} />
        {inventory.workers.map((listed) => { const observed = viewed.get(listed.id)?.worker; const worker = observed && observed.state_version > listed.state_version ? observed : listed; return <OverviewWorkerCard key={worker.id} worker={worker} fresh={inventory.ready && worker.state_version === listed.state_version} hidden={preferences.hidden.includes(worker.id)} onOpen={() => openWorker(worker.id)} />; })}
        {inventory.ready && !inventory.workers.length && <p className="job-notice">No autonomous workers in this project yet. Creating one requires recurring authority and a qualified host.</p>}
        {inventory.hasMore && <><p>More project workers remain; this is the loaded subset.</p><button type="button" disabled={inventory.loading || !inventory.ready} onClick={inventory.loadMore}>Load more project workers</button></>}
        <h2>Earlier results</h2><p>Prior standalone jobs retain their original identity and read-only evidence. Pane history remains in the interactive project.</p>
        <button type="button" onClick={() => { setShowResults(true); history.refresh(); }}>Load earlier project results</button>
        {showResults && <><JobError message={history.error} />{!history.ready && <p className="job-notice">Earlier results are cached or awaiting authorization.</p>}{history.jobs.map((job) => <article key={job.id} className="job-card"><h3>{job.title}</h3><p>{job.state.replaceAll("_", " ")} · Read-only</p><button type="button" onClick={() => openResult(job.id)}>Open earlier result: {job.title}</button></article>)}{history.ready && !history.jobs.length && <p>No retained earlier results.</p>}{history.hasMore && <button type="button" disabled={history.loading || !history.ready} onClick={history.loadMore}>Load more earlier results</button>}</>}
      </>}
    </div>}
    {view.kind === "overview" && !blocked && renderOverviewPanels?.(selectInteractive)}
  </div></section>;
  return children({ active, tabs, overviewTab, content, addAutonomous: !unavailableReason ? (source) => navigate({ kind: "compose", source }) : undefined, unavailableReason, selectInteractive });
}

function statusLabel(worker: WorkerSummary): string {
  return `${workerStateLabel(worker.state)}${worker.attention ? " · Attention" : ""}`;
}
function OverviewWorkerCard({ worker, fresh, hidden, onOpen }: { worker: WorkerSummary; fresh: boolean; hidden: boolean; onOpen: () => void }) {
  const [expanded, setExpanded] = useState(false);
  const descriptionId = `worker-description-${worker.id}`;
  return <article className="job-card" aria-labelledby={`worker-title-${worker.id}`}>
    <h3 id={`worker-title-${worker.id}`}>{worker.title}</h3><p><strong>{!fresh ? "Cached · " : ""}{statusLabel(worker)}</strong>{hidden && " · View hidden"}</p>
    {worker.reason && <p>{worker.reason}</p>}
    <h4>Responsibility</h4><p id={descriptionId} className={`job-prose ${expanded ? "" : "line-clamp-3"}`}>{worker.description}</p>
    <button type="button" aria-expanded={expanded} aria-controls={descriptionId} onClick={() => setExpanded((value) => !value)}>{expanded ? "Collapse description" : "Show full description"}</button>
    <dl className="job-facts"><div><dt>Host</dt><dd>{worker.hostname}</dd></div><div><dt>Current work</dt><dd>{worker.current_item_id ? `Work item ${worker.current_item_id}` : worker.active_run_id ? "Inspection / execution" : "No current execution"}</dd></div><div><dt>Next check</dt><dd>{worker.next_check_at ? <time dateTime={worker.next_check_at}>{new Date(worker.next_check_at).toLocaleString()}</time> : "Not scheduled"}</dd></div><div><dt>Last confirmed update</dt><dd><time dateTime={worker.updated_at}>{new Date(worker.updated_at).toLocaleString()}</time></dd></div></dl>
    <button type="button" onClick={onOpen} aria-label={`Open ${worker.title}`}>Open worker · Checklist, backlog and results</button>
  </article>;
}
