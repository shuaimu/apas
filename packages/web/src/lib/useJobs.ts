"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "./store";
import { jobCacheRevision, jobRequests, onJobCachePurge, purgeJobCache, readJobRecord, writeJobRecord } from "./jobCache";
import type { JobScope, SavedJob, SavedJobRequest } from "./jobCache";
import { JobHttpError, onJobMessage, resolveJobProject, resolveJobRequest, sendJobRequest, synchronizeJob } from "./jobs";

export function useJobOnline() {
  const connected = useStore((state) => state.connected);
  const authenticated = useStore((state) => state.isAuthenticated);
  const [online, setOnline] = useState(false);
  const [foreground, setForeground] = useState(true);
  useEffect(() => {
    const update = () => { setOnline(navigator.onLine); setForeground(document.visibilityState === "visible"); };
    update();
    window.addEventListener("online", update); window.addEventListener("offline", update);
    document.addEventListener("visibilitychange", update);
    return () => { window.removeEventListener("online", update); window.removeEventListener("offline", update); document.removeEventListener("visibilitychange", update); };
  }, []);
  return online && foreground && connected && authenticated;
}
export function useJob(id: string, projectHint?: string | null) {
  const account = useStore((state) => state.userId);
  const socket = useStore((state) => state.ws);
  const capable = useStore((state) => state.negotiatedCapabilities.has("autonomous_jobs_v1"));
  const online = useJobOnline();
  const [saved, setSaved] = useState<SavedJob>();
  const [project, setProject] = useState(projectHint ?? "");
  const [synced, setSynced] = useState(false);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => { setSynced(false); setRevision((value) => value + 1); }, []);
  useEffect(() => onJobCachePurge(() => { setSaved(undefined); setSynced(false); setError("Job access changed. Refresh to check current access."); }), []);
  useEffect(() => {
    let active = true;
    let resolvedProject = projectHint;
    if (!account || !id) return;
    setSynced(false);
    const run = async () => {
      const resolved = await resolveJobProject(id, projectHint);
      resolvedProject = resolved;
      if (!active) return;
      setProject(resolved);
      const scope: JobScope = [account, resolved, id];
      const cached = await readJobRecord<SavedJob>(scope, "snapshot");
      if (active && cached) setSaved(cached);
      if (!online) return;
      const current = await synchronizeJob(scope);
      if (!active) return;
      setSaved(current); setError(""); setSynced(true);
      if (capable && socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "subscribe_job", project_id: resolved, job_id: id, after_seq: current.cursor }));
    };
    void run().catch(async (failure: unknown) => {
      if (failure instanceof JobHttpError && [403, 404, 410].includes(failure.status)) {
        if (resolvedProject) {
          try { await purgeJobCache(account, resolvedProject); }
          catch { if (active) setError("Access is unavailable and browser cache purge failed. Clear this site's data."); return; }
        }
        if (active) setSaved(undefined);
      }
      if (active) { setSynced(false); setError(failure instanceof Error ? failure.message : "Could not synchronize job."); }
    });
    return () => {
      active = false;
      if (socket?.readyState === WebSocket.OPEN && capable) socket.send(JSON.stringify({ type: "unsubscribe_job", job_id: id }));
    };
  }, [account, id, projectHint, online, revision, socket, capable]);
  useEffect(() => {
    const stop = onJobMessage((message) => {
      const messageId = message.type === "job_snapshot" ? message.detail.job.id : message.type === "job_event" ? message.event.job_id : message.job_id;
      if (messageId !== id) return;
      if (message.type === "job_subscription_rejected" && !message.resync_required) {
        setSynced(false); setSaved(undefined); setError(message.message);
        if (account && project) void purgeJobCache(account, project).catch(() => setError("Access revoked; browser cache purge failed. Clear site data."));
        return;
      }
      if (message.type === "job_snapshot" && saved && message.detail.high_watermark <= saved.cursor && message.detail.job.state_version <= saved.detail.job.state_version) return;
      if (message.type === "job_event" && saved && message.event.seq <= saved.cursor) return;
      refresh();
    });
    // Durable replay repairs a dropped live frame, even when the socket stayed open.
    const timer = window.setInterval(() => { if (online) refresh(); }, 15_000);
    return () => { stop(); clearInterval(timer); };
  }, [account, id, online, project, refresh, saved]);
  return { saved, project, ready: online && synced, error, refresh };
}

/** IndexedDB orders writes; submit awaits the exact final draft transaction. */
export function useJobDraft<T>(scope: JobScope, id: string, initial: T) {
  const [value, setValue] = useState(initial);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const writes = useRef<Promise<void>>(Promise.resolve());
  const initialValue = useRef(initial);
  initialValue.current = initial;
  const scopeKey = JSON.stringify(scope);
  useEffect(() => {
    let active = true;
    setLoaded(false); setError("");
    void readJobRecord<T>(JSON.parse(scopeKey) as JobScope, "draft", id).then((saved) => {
      if (active) { setValue(saved ?? initialValue.current); setLoaded(true); }
    }).catch((failure: unknown) => { if (active) setError(failure instanceof Error ? failure.message : "Draft storage is unavailable."); });
    return () => { active = false; };
  }, [scopeKey, id]);
  useEffect(() => onJobCachePurge(() => { setLoaded(false); setError("Access changed. Draft cleared from this view."); setValue(initialValue.current); }), []);
  const update = (next: T) => {
    setValue(next);
    const revision = jobCacheRevision();
    // Queue the transaction in IndexedDB now, not behind a JavaScript promise
    // that navigation can discard before the latest edits reach storage.
    writes.current = writeJobRecord(scope, "draft", next, id, revision);
    void writes.current.then(() => setError("")).catch((failure: unknown) => setError(failure instanceof Error ? failure.message : "Draft was not saved."));
  };
  const flush = async () => { const revision = jobCacheRevision(); await writes.current; await writeJobRecord(scope, "draft", value, id, revision); };
  return { value, update, flush, loaded, error };
}
export function useJobMutation(scope: JobScope, path?: string, admissionPreviewId?: string) {
  const [request, setRequest] = useState<SavedJobRequest>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [retryAllowed, setRetryAllowed] = useState(false);
  const [recovered, setRecovered] = useState(false);
  const scopeKey = JSON.stringify(scope);
  const account = scope[0];
  useEffect(() => {
    let active = true;
    setRequest(undefined); setRetryAllowed(false); setRecovered(false); setError("");
    void jobRequests(account).then(async (requests) => {
      const pending = requests.find((item) => JSON.stringify(item.scope) === scopeKey && (!path || item.path === path) && ((!item.receipt || item.receipt.status === "pending") || Boolean(admissionPreviewId && "preview_id" in item.body && item.body.preview_id === admissionPreviewId)));
      if (!active) return;
      if (!pending) { setRecovered(true); return; }
      setRequest(pending);
      if (pending.receipt && pending.receipt.status !== "pending") { setRecovered(true); return; }
      try { const resolved = await resolveJobRequest(pending); if (active) setRequest(resolved); }
      catch (failure) {
        if (!active) return;
        setRetryAllowed(failure instanceof JobHttpError && failure.status === 404);
        setError(failure instanceof JobHttpError && failure.status === 404 ? "No receipt found. Refresh current job state, then explicitly retry the saved request." : "Receipt is unconfirmed. Check again before retrying.");
      }
      if (active) setRecovered(true);
    }).catch((failure: unknown) => { if (active) setError(failure instanceof Error ? failure.message : "Request recovery failed."); });
    return () => { active = false; };
  }, [scopeKey, path, account, admissionPreviewId]);
  useEffect(() => onJobCachePurge(() => { setRequest(undefined); setRetryAllowed(false); setRecovered(false); setError("Access changed."); }), []);
  const resolve = async () => {
    if (!request) return;
    setBusy(true); setError(""); setRetryAllowed(false);
    try { setRequest(await resolveJobRequest(request)); }
    catch (failure) {
      setRetryAllowed(failure instanceof JobHttpError && failure.status === 404);
      setError(failure instanceof JobHttpError && failure.status === 404 ? "No receipt found; same-ID retry is available after synchronization." : "Still unconfirmed. No mutation has been replayed.");
    } finally { setBusy(false); }
  };
  const submit = async (next: SavedJobRequest) => {
    setBusy(true); setError(""); setRetryAllowed(false);
    let persisted = false;
    try {
      // Publish the durable identity to UI only after persistence; transport also awaits it.
      await writeJobRecord(next.scope, "request", next, next.id);
      persisted = true;
      setRequest(next);
      const saved = await sendJobRequest(next); setRequest(saved); return saved;
    } catch (failure) {
      setError(`${failure instanceof Error ? failure.message : "Request failed."} ${persisted ? "Check the original receipt before trying again." : "The request could not be saved; nothing was submitted."}`);
      return undefined;
    } finally { setBusy(false); }
  };
  return { request, busy, error, retryAllowed, resolve, submit, unresolved: !recovered || Boolean(request && (!request.receipt || request.receipt.status === "pending")) };
}
