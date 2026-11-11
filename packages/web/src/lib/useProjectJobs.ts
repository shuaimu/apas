"use client";

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { ProjectJobListResponse } from "@apas/protocol";
import { useStore } from "./store";
import { jobCacheRevision, onJobCachePurge, purgeJobCache, readJobRecord, writeJobRecord } from "./jobCache";
import type { JobScope } from "./jobCache";
import { JobHttpError, jobFetch, jobPath, onJobMessage } from "./jobs";
import { useJobOnline } from "./useJobs";

type Jobs = ProjectJobListResponse["jobs"];
type Mode = "refresh" | "more";
interface Inventory {
  scope: string;
  revision: number;
  socket: WebSocket | null;
  jobs: Jobs;
  nextCursor: string | null;
  pages: number;
  ready: boolean;
  loading: boolean;
  error: string;
}
const emptyJobs: Jobs = [];
const readOnlyActions: Jobs[number]["allowed_actions"] = {
  admit: false, answer: false, cancel: false, retry: false, retry_publication: false, manage_policy: false,
};

/** A refresh may overlap a previously loaded page, but must never regress its durable version. */
function mergeJobs(incoming: Jobs, previous: Jobs): Jobs {
  const known = new Map(previous.map((job) => [job.id, job]));
  const merged = new Map<string, Jobs[number]>();
  for (const job of incoming) {
    const older = merged.get(job.id) ?? known.get(job.id);
    merged.set(job.id, older && older.state_version > job.state_version
      ? { ...older, allowed_actions: readOnlyActions }
      : job);
  }
  return [...merged.values()];
}

export function useProjectJobs(projectId: string | null | undefined) {
  const account = useStore((state) => state.userId);
  const socket = useStore((state) => state.ws);
  const online = useJobOnline();
  const revision = useSyncExternalStore(onJobCachePurge, jobCacheRevision, () => 0);
  const scopeKey = JSON.stringify([account, projectId]);
  const [inventory, setInventory] = useState<Inventory>();
  const current = useRef<Inventory | undefined>(undefined);
  const initialRevision = useRef(revision);
  const previousRevision = useRef(revision);
  const control = useRef<{ scope: string; revision: number; run: (mode: Mode) => void } | undefined>(undefined);

  useEffect(() => {
    const purged = previousRevision.current !== revision;
    previousRevision.current = revision;
    if (!account || !projectId) {
      current.current = undefined;
      setInventory(undefined);
      return;
    }
    const scope: JobScope = [account, projectId, ""];
    const retained = current.current?.scope === scopeKey && current.current.revision === revision
      ? current.current : undefined;
    let view: Inventory = {
      scope: scopeKey, revision, socket, jobs: retained?.jobs ?? [],
      nextCursor: retained?.nextCursor ?? null, pages: retained?.pages ?? 1,
      ready: false, loading: online && !purged,
      error: purged ? "Job access changed. Refresh to check current access." : "",
    };
    let active = true;
    let busy = true;
    let refreshPending = false;
    const valid = () => active && revision === jobCacheRevision() && useStore.getState().userId === account;
    const publish = (changes: Partial<Inventory>) => {
      if (!valid()) return;
      view = { ...view, ...changes };
      current.current = view;
      setInventory(view);
    };
    publish({});

    const run = async (mode: Mode) => {
      if (!valid() || !online) return;
      if (busy) {
        if (mode === "refresh") { refreshPending = true; publish({ ready: false }); }
        return;
      }
      if (mode === "more" && !view.nextCursor) return;
      busy = true;
      publish({ loading: true, ready: false, error: "" });
      try {
        let cursor = mode === "more" ? view.nextCursor : null;
        const count = mode === "more" ? 1 : view.pages;
        let jobs: Jobs = mode === "more" ? [...view.jobs] : [];
        let pages = mode === "more" ? view.pages : 0;
        let firstPage: ProjectJobListResponse | undefined;
        // Only recheck pages the user already opened; history is never fetched to exhaustion.
        for (let index = 0; index < count; index += 1) {
          const page = await jobFetch<ProjectJobListResponse>(`${jobPath(projectId)}${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ""}`, "project_list");
          if (!valid()) return;
          if (page.jobs.some((job) => job.project_id !== projectId)) throw new Error("The server returned jobs from a different project.");
          if (mode === "refresh" && index === 0) firstPage = page;
          jobs.push(...page.jobs);
          pages += 1;
          cursor = page.next_cursor ?? null;
          if (!cursor) break;
        }
        jobs = mergeJobs(jobs, view.jobs);
        if (firstPage) {
          await writeJobRecord(scope, "list", { ...firstPage, jobs: mergeJobs(firstPage.jobs, jobs) }, "project", revision);
          if (!valid()) return;
        }
        publish({ jobs, nextCursor: cursor, pages, ready: !refreshPending, error: "" });
      } catch (failure) {
        if (!valid()) return;
        if (failure instanceof JobHttpError && [401, 403, 404, 410].includes(failure.status)) {
          publish({ jobs: [], nextCursor: null, pages: 1 });
          try { await purgeJobCache(account, projectId); }
          catch { if (active) setInventory({ ...view, revision: jobCacheRevision(), error: "Access is unavailable and browser cache purge failed. Clear this site's data." }); }
        } else {
          publish({ error: failure instanceof Error ? failure.message : "Project jobs could not be loaded." });
        }
      } finally {
        busy = false;
        if (valid()) {
          publish({ loading: false });
          if (refreshPending) { refreshPending = false; void run("refresh"); }
        }
      }
    };
    control.current = { scope: scopeKey, revision, run: (mode) => { void run(mode); } };
    const restore = async () => {
      // A purge notifies before its deletion transaction finishes. Never reread that old cache.
      if (!retained && initialRevision.current === revision && !purged) {
        try {
          const cached = await readJobRecord<ProjectJobListResponse>(scope, "list", "project");
          if (!valid()) return;
          // Ignore obsolete summary-only pages without purging unrelated cached job detail.
          if (cached && cached.jobs.every((job) => job.project_id === projectId && typeof job.description === "string")) {
            publish({ jobs: mergeJobs(cached.jobs, []), nextCursor: cached.next_cursor ?? null });
          }
        } catch (failure) {
          publish({ error: failure instanceof Error ? failure.message : "Cached jobs could not be loaded." });
        }
      }
      busy = false;
      refreshPending = false;
      if (!purged) await run("refresh");
    };
    void restore();
    const remove = onJobMessage((message) => {
      if (message.type === "job_changed" && message.project_id === projectId) void run("refresh");
    });
    // Durable list reads repair dropped hints and recheck current content authority.
    const timer = window.setInterval(() => { void run("refresh"); }, 20_000);
    return () => {
      active = false;
      remove();
      clearInterval(timer);
      if (control.current?.run && control.current.scope === scopeKey && control.current.revision === revision) control.current = undefined;
    };
  }, [account, projectId, scopeKey, online, socket, revision]);

  const refresh = useCallback(() => {
    if (control.current?.scope === scopeKey && control.current.revision === revision) control.current.run("refresh");
  }, [scopeKey, revision]);
  const loadMore = useCallback(() => {
    if (control.current?.scope === scopeKey && control.current.revision === revision) control.current.run("more");
  }, [scopeKey, revision]);
  // Render-time scope/revision guards hide old content before effects clean up in-flight work.
  const visible = inventory?.scope === scopeKey && inventory.revision === revision ? inventory : undefined;
  const ready = Boolean(online && visible?.ready && visible.socket === socket);
  const rows = visible?.jobs ?? emptyJobs;
  const jobs = useMemo(() => ready ? rows : rows.map((job) => ({ ...job, allowed_actions: readOnlyActions })), [ready, rows]);
  return { jobs, ready, loading: visible?.loading ?? false, error: visible?.error ?? "", hasMore: Boolean(visible?.nextCursor), loadMore, refresh };
}
