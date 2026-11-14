"use client";

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { Bot, LayoutDashboard, Plus, RefreshCw, X } from "lucide-react";
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
import { OverviewNotice, OverviewSection, overviewButton, overviewCard, overviewMuted, overviewPage } from "../overview/OverviewElements";
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
  selectInteractive: () => void;
}
interface WorkspaceProps {
  project?: JobProjectContext;
  paneSelectionRequested: boolean;
  initialOverview?: boolean;
  /** The quiet line under the project's name on the Overview. The default
   *  describes the desktop panels; a caller with other panels says its own. */
  overviewSummary?: string;
  /**
   * The caller's part of the Overview: the project's tabs and whatever else it
   * shows about them. The Overview is one page in the workspace's own style:
   * the project's name, these panels, then a compact "Autonomous workers"
   * section. The panels are built from `overview/OverviewElements`, not for
   * the Jobs stylesheet. Worker, result and composer views are the Jobs pages'
   * own screens and keep that style.
   */
  renderOverviewPanels?: (selectInteractive: () => void) => ReactNode;
  children: (controls: WorkspaceControls) => ReactNode;
}

/** Identity changes reset protected views, never the caller's mounted terminals. */
export function ProjectWorkersWorkspace(props: WorkspaceProps) {
  const account = useStore((state) => state.userId);
  return <WorkerWorkspace key={`${account ?? ""}:${props.project?.id ?? ""}`} {...props} />;
}

const DEFAULT_OVERVIEW_SUMMARY = "This project's tabs, what they have used, and its autonomous workers.";

function WorkerWorkspace({ project, paneSelectionRequested, initialOverview = false, overviewSummary = DEFAULT_OVERVIEW_SUMMARY, renderOverviewPanels, children }: WorkspaceProps) {
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
  const identityUnavailable = "Project identity is not available yet. Reconnect or reopen this project.";
  const overviewTab = <button ref={view.kind === "overview" ? selectedTab : undefined} type="button" onClick={selectOverview} aria-label="Project overview" aria-pressed={view.kind === "overview"} className="flex min-h-11 shrink-0 items-center gap-2 border-r border-gray-200 px-3 text-sm font-medium dark:border-gray-700"><LayoutDashboard className="h-4 w-4" aria-hidden="true" />Overview</button>;
  const recheckAccess = () => { loadedRevision.current = workerCacheRevision(); setEpoch((value) => value + 1); setAccessChanged(false); };
  const loadResults = () => { setShowResults(true); history.refresh(); };
  const listedWorkers = inventory.workers.map((listed) => {
    const observed = viewed.get(listed.id)?.worker;
    const worker = observed && observed.state_version > listed.state_version ? observed : listed;
    return { worker, fresh: inventory.ready && worker.state_version === listed.state_version, hidden: preferences.hidden.includes(worker.id) };
  });
  // The Overview as one page of the workspace: the project's tabs come first
  // because they are what is running, and autonomous workers, which are off
  // unless a deployment enables them, close the page. It has no "Back to
  // interactive agents": the tab bar above it leads to every tab, and so do
  // the caller's panels. The worker, composer and earlier-result screens
  // below keep that button, because they are pages of their own.
  const workspaceOverview = view.kind === "overview" && <section ref={contentRef} tabIndex={-1} aria-label="Project overview" className="min-h-0 w-full flex-1 overflow-y-auto bg-white text-gray-900 outline-none dark:bg-gray-900 dark:text-gray-100">
    <div className={overviewPage}>
      <header>
        <h1 className="truncate text-lg font-semibold text-gray-900 dark:text-gray-100">{project?.name ?? "Project"}</h1>
        <p className={`mt-0.5 ${overviewMuted}`}>{overviewSummary}</p>
      </header>
      {storageError && <OverviewNotice alert>{storageError}</OverviewNotice>}
      {!blocked && renderOverviewPanels?.(selectInteractive)}
      <OverviewSection
        title="Autonomous workers"
        aside={project && account && !blocked && loaded && <>
          <button type="button" className={overviewButton} disabled={inventory.loading} onClick={inventory.refresh} aria-label="Refresh project workers" title="Refresh project workers"><RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />Refresh</button>
          <button type="button" className={overviewButton} onClick={() => navigate({ kind: "compose" })}><Plus className="h-3.5 w-3.5" aria-hidden="true" />New autonomous worker</button>
        </>}
      >
        {!project || !account ? <OverviewNotice>{identityUnavailable}</OverviewNotice> : blocked ? <div className="flex flex-col items-start gap-2">
          <OverviewNotice alert><strong className="font-semibold">Worker access changed.</strong> Cached worker content and earlier results have been removed. Check current project authority before loading again.</OverviewNotice>
          <button type="button" className={overviewButton} onClick={recheckAccess}>Check current access</button>
        </div> : !loaded ? <p role="status" className={overviewMuted}>Restoring project worker views…</p> : <div key={epoch} className="flex flex-col gap-3">
          {!inventory.ready && <OverviewNotice status>{inventory.loading ? "Checking current project access and worker state…" : "Offline or reconnecting. Cached workers are stale and read-only."}</OverviewNotice>}
          {inventory.error && <OverviewNotice alert>{inventory.error}</OverviewNotice>}
          {listedWorkers.map(({ worker, fresh, hidden }) => <WorkspaceWorkerCard key={worker.id} worker={worker} fresh={fresh} hidden={hidden} onOpen={() => openWorker(worker.id)} />)}
          {inventory.ready && !inventory.workers.length && <p className="text-sm text-gray-600 dark:text-gray-300">No autonomous workers in this project yet. Creating one requires recurring authority and a qualified host.</p>}
          {inventory.hasMore && <div className="flex flex-wrap items-center gap-2">
            <p className={overviewMuted}>More project workers remain; this is the loaded subset.</p>
            <button type="button" className={overviewButton} disabled={inventory.loading || !inventory.ready} onClick={inventory.loadMore}>Load more project workers</button>
          </div>}
          <div className={`${overviewCard} px-3 py-2`}>
            <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
              <div className="min-w-0">
                <h3 className="text-sm font-medium text-gray-900 dark:text-gray-100">Earlier results</h3>
                <p className={overviewMuted}>Standalone jobs from before workers keep their original identity and read-only evidence. Tab history stays with its tab.</p>
              </div>
              <button type="button" className={overviewButton} onClick={loadResults}>Load earlier project results</button>
            </div>
            {showResults && <div className="mt-2 flex flex-col gap-2 border-t border-gray-200 pt-2 dark:border-gray-700">
              {history.error && <OverviewNotice alert>{history.error}</OverviewNotice>}
              {!history.ready && <OverviewNotice status>Earlier results are cached or awaiting authorization.</OverviewNotice>}
              {history.jobs.length > 0 && <ul className="divide-y divide-gray-100 dark:divide-gray-800">
                {history.jobs.map((job) => <li key={job.id} className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 py-1.5">
                  <span className="min-w-0"><span className="block truncate text-sm font-medium text-gray-900 dark:text-gray-100">{job.title}</span><span className={overviewMuted}>{job.state.replaceAll("_", " ")} · Read-only</span></span>
                  <button type="button" className={overviewButton} onClick={() => openResult(job.id)} aria-label={`Open earlier result: ${job.title}`}>Open</button>
                </li>)}
              </ul>}
              {history.ready && !history.jobs.length && <p className={overviewMuted}>No retained earlier results.</p>}
              {history.hasMore && <div><button type="button" className={overviewButton} disabled={history.loading || !history.ready} onClick={history.loadMore}>Load more earlier results</button></div>}
            </div>}
          </div>
        </div>}
      </OverviewSection>
    </div>
  </section>;
  const content = workspaceOverview || active && <section ref={contentRef} tabIndex={-1} aria-label="Project autonomous work" className="jobs-app jobs-embedded"><div className="jobs-main">
    <div className="job-actions"><button type="button" onClick={selectInteractive}>Back to interactive agents</button><button type="button" onClick={selectOverview}>Project overview</button></div>
    <JobError message={storageError} />
    {!project || !account ? <p className="job-notice">{identityUnavailable}</p> : blocked ? <><h1>Worker access changed</h1><p role="alert" className="job-notice">Cached worker content and earlier results have been removed. Check current project authority before loading again.</p><button type="button" onClick={recheckAccess}>Check current access</button></> : !loaded ? <p role="status">Restoring project worker views…</p> : <div key={epoch}>
      {view.kind === "compose" ? <WorkerComposer project={project} source={view.source} onApproved={(id, projectId) => { if (projectId === scope[1]) openWorker(id); }} /> : view.kind === "worker" ? <>
        <p>Hiding a view does not pause or stop the worker.</p><button type="button" onClick={() => hide("worker", view.id)}>Hide this worker view</button>
        <WorkerDetailScreen key={view.id} id={view.id} projectHint={project.id} runId={view.runId} questionId={view.questionId} review={view.review} onNavigate={(destination) => navigate({ kind: "worker", id: view.id, ...destination })} onSummary={rememberSummary} />
      </> : view.kind === "result" ? <><p className="job-notice">Earlier result · Read-only. This job is not a persistent worker.</p><JobDetailScreen key={view.id} id={view.id} projectHint={project.id} runId={view.runId} questionId={view.questionId} review={view.review} onNavigate={(destination) => navigate({ kind: "result", id: view.id, ...destination })} /></> : null}
    </div>}
  </div></section>;
  return children({ active, tabs, overviewTab, content, selectInteractive });
}

function statusLabel(worker: WorkerSummary): string {
  return `${workerStateLabel(worker.state)}${worker.attention ? " · Attention" : ""}`;
}
/** A worker on the Overview: its state, why, what it is responsible for, and
 *  where and when it runs, in the Overview's card. */
function WorkspaceWorkerCard({ worker, fresh, hidden, onOpen }: { worker: WorkerSummary; fresh: boolean; hidden: boolean; onOpen: () => void }) {
  const [expanded, setExpanded] = useState(false);
  const descriptionId = `worker-description-${worker.id}`;
  const facts: Array<[string, ReactNode]> = [
    ["Host", worker.hostname],
    ["Current work", worker.current_item_id ? `Work item ${worker.current_item_id}` : worker.active_run_id ? "Inspection / execution" : "No current execution"],
    ["Next check", worker.next_check_at ? <time dateTime={worker.next_check_at}>{new Date(worker.next_check_at).toLocaleString()}</time> : "Not scheduled"],
    ["Last confirmed update", <time key="updated" dateTime={worker.updated_at}>{new Date(worker.updated_at).toLocaleString()}</time>],
  ];
  return <article className={`${overviewCard} flex flex-col gap-2 p-3`} aria-labelledby={`worker-title-${worker.id}`}>
    <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2">
      <div className="min-w-0">
        <h3 id={`worker-title-${worker.id}`} className="text-sm font-semibold text-gray-900 dark:text-gray-100">{worker.title}</h3>
        <p className="text-xs text-gray-600 dark:text-gray-300"><strong className="inline-block font-medium first-letter:uppercase">{!fresh ? "Cached · " : ""}{statusLabel(worker)}</strong>{hidden && " · View hidden"}</p>
      </div>
      <button type="button" className={overviewButton} onClick={onOpen} aria-label={`Open ${worker.title}`} title="Checklist, backlog and results">Open worker</button>
    </div>
    {worker.reason && <p className={`text-sm ${worker.attention ? "text-amber-800 dark:text-amber-200" : "text-gray-700 dark:text-gray-300"}`}>{worker.reason}</p>}
    <div>
      <p id={descriptionId} className={`whitespace-pre-wrap text-sm text-gray-700 dark:text-gray-300 ${expanded ? "" : "line-clamp-3"}`}>{worker.description}</p>
      <button type="button" aria-expanded={expanded} aria-controls={descriptionId} onClick={() => setExpanded((value) => !value)} className="mt-1 rounded text-xs font-medium text-blue-700 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:text-blue-300">{expanded ? "Collapse description" : "Show full description"}</button>
    </div>
    <dl className="grid grid-cols-2 gap-x-4 gap-y-2 border-t border-gray-100 pt-2 lg:grid-cols-4 dark:border-gray-800">
      {facts.map(([label, value]) => <div key={label} className="min-w-0"><dt className={overviewMuted}>{label}</dt><dd className="break-words text-sm text-gray-900 dark:text-gray-100">{value}</dd></div>)}
    </dl>
  </article>;
}
