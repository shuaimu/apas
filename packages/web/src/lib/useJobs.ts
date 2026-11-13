"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
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
  const token = useStore((state) => state.token);
  const cacheRevision = useSyncExternalStore(onJobCachePurge, jobCacheRevision, () => 0);
  const identity = JSON.stringify([account, token, id, projectHint, cacheRevision]);
  const [savedIdentity, setSavedIdentity] = useState("");
  const blocked = useRef(false);
  const socket = useStore((state) => state.ws);
  const capable = useStore((state) => state.negotiatedCapabilities.has("autonomous_jobs_v1"));
  const online = useJobOnline();
  const [saved, setSaved] = useState<SavedJob>();
  const [project, setProject] = useState(projectHint ?? "");
  const [synced, setSynced] = useState(false);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => { blocked.current = false; setSynced(false); setRevision((value) => value + 1); }, []);
  useEffect(() => onJobCachePurge(() => { blocked.current = true; setSaved(undefined); setSynced(false); setError("Job access changed. Refresh to check current access."); }), []);
  useEffect(() => {
    let active = true;
    let resolvedProject = projectHint;
    if (!account || !id || blocked.current) return;
    const valid = () => active && cacheRevision === jobCacheRevision() && useStore.getState().userId === account && useStore.getState().token === token;
    setSynced(false);
    const run = async () => {
      const resolved = await resolveJobProject(id, projectHint);
      resolvedProject = resolved;
      if (!valid()) return;
      setProject(resolved);
      const scope: JobScope = [account, resolved, id];
      const cached = await readJobRecord<SavedJob>(scope, "snapshot");
      if (!valid()) return;
      if (cached) { setSaved(cached); setSavedIdentity(identity); }
      if (!online) return;
      const current = await synchronizeJob(scope);
      if (!valid()) return;
      setSaved(current); setSavedIdentity(identity); setError(""); setSynced(true);
      if (capable && socket?.readyState === WebSocket.OPEN && useStore.getState().ws === socket) socket.send(JSON.stringify({ type: "subscribe_job", project_id: resolved, job_id: id, after_seq: current.cursor }));
    };
    void run().catch(async (failure: unknown) => {
      if (!valid()) return;
      if (failure instanceof JobHttpError && [403, 404, 410].includes(failure.status)) {
        if (resolvedProject) {
          try { await purgeJobCache(account, resolvedProject); }
          catch { if (valid()) setError("Access is unavailable and browser cache purge failed. Clear this site's data."); return; }
        }
        if (valid()) setSaved(undefined);
      }
      if (valid()) { setSynced(false); setError(failure instanceof Error ? failure.message : "Could not synchronize job."); }
    });
    return () => {
      active = false;
      if (socket?.readyState === WebSocket.OPEN && capable && useStore.getState().ws === socket && useStore.getState().userId === account && useStore.getState().token === token) socket.send(JSON.stringify({ type: "unsubscribe_job", job_id: id }));
    };
  }, [account, token, id, projectHint, online, revision, socket, capable, cacheRevision, identity]);
  useEffect(() => {
    const stop = onJobMessage((message) => {
      if (blocked.current || cacheRevision !== jobCacheRevision() || useStore.getState().userId !== account || useStore.getState().token !== token || useStore.getState().ws !== socket) return;
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
    const timer = window.setInterval(() => { if (online && !blocked.current && cacheRevision === jobCacheRevision()) refresh(); }, 15_000);
    return () => { stop(); clearInterval(timer); };
  }, [account, token, id, online, project, refresh, saved, socket, cacheRevision]);
  const visible = savedIdentity === identity;
  return { saved: visible ? saved : undefined, project: visible ? project : projectHint ?? "", ready: visible && online && synced, error: visible || !saved ? error : "", refresh };
}

/** IndexedDB orders writes; submit awaits the exact final draft transaction. */
export function useJobDraft<T>(scope: JobScope, id: string, initial: T) {
  const account = useStore((state) => state.userId);
  const revision = useSyncExternalStore(onJobCachePurge, jobCacheRevision, () => 0);
  const key = JSON.stringify([scope, id, revision]);
  const [draft, setDraft] = useState<{ key: string; value: T; loaded: boolean; error: string }>();
  const writes = useRef<Promise<void>>(Promise.resolve());
  const initialValue = useRef(initial);
  initialValue.current = initial;
  const liveKey = useRef(key);
  liveKey.current = key;
  const latest = useRef<{ key: string; value: T }>({ key, value: initial });
  const scopeKey = JSON.stringify(scope);
  const previousRevision = useRef(revision);
  useEffect(() => {
    const scope = JSON.parse(scopeKey) as JobScope;
    let active = true;
    const purged = previousRevision.current !== revision;
    previousRevision.current = revision;
    latest.current = { key, value: initialValue.current };
    writes.current = Promise.resolve();
    setDraft({ key, value: initialValue.current, loaded: false, error: purged ? "Access changed. Draft cleared from this view." : "" });
    if (purged || !account || account !== scope[0]) return;
    void readJobRecord<T>(scope, "draft", id).then((saved) => {
      if (!active || liveKey.current !== key || revision !== jobCacheRevision() || useStore.getState().userId !== account) return;
      const value = saved ?? initialValue.current;
      latest.current = { key, value };
      setDraft({ key, value, loaded: true, error: "" });
    }).catch((failure: unknown) => {
      if (active && liveKey.current === key && revision === jobCacheRevision()) setDraft({ key, value: initialValue.current, loaded: false, error: failure instanceof Error ? failure.message : "Draft storage is unavailable." });
    });
    return () => { active = false; };
  }, [scopeKey, id, key, revision, account]);
  const visible = draft?.key === key && account === scope[0] ? draft : undefined;
  const update = (next: T) => {
    if (!visible?.loaded || liveKey.current !== key || revision !== jobCacheRevision() || useStore.getState().userId !== scope[0]) return;
    latest.current = { key, value: next };
    setDraft({ key, value: next, loaded: true, error: "" });
    writes.current = writeJobRecord(scope, "draft", next, id, revision);
    void writes.current.catch((failure: unknown) => {
      if (liveKey.current === key && revision === jobCacheRevision()) setDraft((value) => value?.key === key ? { ...value, error: failure instanceof Error ? failure.message : "Draft was not saved." } : value);
    });
  };
  const flush = async () => {
    if (!visible?.loaded || liveKey.current !== key || revision !== jobCacheRevision() || useStore.getState().userId !== scope[0]) throw new Error("Job draft access changed. Nothing was submitted.");
    await writes.current;
    if (latest.current.key !== key) throw new Error("Job draft scope changed. Nothing was submitted.");
    await writeJobRecord(scope, "draft", latest.current.value, id, revision);
    if (revision !== jobCacheRevision() || useStore.getState().userId !== scope[0]) throw new Error("Job draft access changed. Nothing was submitted.");
  };
  return { value: visible?.value ?? initial, update, flush, loaded: visible?.loaded ?? false, error: visible?.error ?? "" };
}
export function useJobMutation(scope: JobScope, path?: string, admissionPreviewId?: string) {
  const account = useStore((state) => state.userId);
  const token = useStore((state) => state.token);
  const online = useJobOnline();
  const revision = useSyncExternalStore(onJobCachePurge, jobCacheRevision, () => 0);
  const scopeKey = JSON.stringify(scope);
  const key = JSON.stringify([scopeKey, path, admissionPreviewId, revision, account, token]);
  const [view, setView] = useState<{ key: string; request?: SavedJobRequest; busy: boolean; error: string; retryAllowed: boolean; recovered: boolean }>();
  const liveKey = useRef(key);
  liveKey.current = key;
  const inFlight = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const valid = () => mounted.current && liveKey.current === key && revision === jobCacheRevision() && useStore.getState().userId === scope[0] && useStore.getState().token === token;
  useEffect(() => {
    const scope = JSON.parse(scopeKey) as JobScope;
    let active = true;
    inFlight.current = false;
    setView({ key, busy: false, error: "", retryAllowed: false, recovered: false });
    if (!account || account !== scope[0]) return;
    const current = () => active && liveKey.current === key && revision === jobCacheRevision() && useStore.getState().userId === account && useStore.getState().token === token;
    void jobRequests(account).then(async (requests) => {
      if (!current()) return;
      const request = requests.find((item) => JSON.stringify(item.scope) === scopeKey && (!path || item.path === path) && ((!item.receipt || item.receipt.status === "pending") || Boolean(admissionPreviewId && "preview_id" in item.body && item.body.preview_id === admissionPreviewId)));
      const next = { key, request, busy: false, error: "", retryAllowed: false, recovered: true };
      setView(next);
      if (!request || request.receipt && request.receipt.status !== "pending" || !online) return;
      try { const resolved = await resolveJobRequest(request); if (current()) setView({ ...next, request: resolved }); }
      catch (failure) {
        if (!current()) return;
        const missing = failure instanceof JobHttpError && failure.status === 404;
        setView({ ...next, retryAllowed: missing, error: missing ? "No receipt found. Refresh current job state, then explicitly retry the saved request." : "Receipt is unconfirmed. Check again before retrying." });
      }
    }).catch((failure: unknown) => {
      if (current()) setView({ key, busy: false, error: failure instanceof Error ? failure.message : "Request recovery failed.", retryAllowed: false, recovered: false });
    });
    return () => { active = false; };
  }, [key, scopeKey, path, account, token, admissionPreviewId, revision, online]);
  const visible = view?.key === key && account === scope[0] ? view : undefined;
  const resolve = async () => {
    if (!visible?.request || !valid() || !online || inFlight.current) return;
    inFlight.current = true;
    setView({ ...visible, busy: true, error: "", retryAllowed: false });
    try { const request = await resolveJobRequest(visible.request); if (valid()) setView({ ...visible, request, busy: false, error: "", retryAllowed: false }); }
    catch (failure) {
      if (valid()) {
        const missing = failure instanceof JobHttpError && failure.status === 404;
        setView({ ...visible, busy: false, retryAllowed: missing, error: missing ? "No receipt found; same-ID retry is available after synchronization." : "Still unconfirmed. No mutation has been replayed." });
      }
    } finally { if (valid()) inFlight.current = false; }
  };
  const submit = async (next: SavedJobRequest) => {
    if (!visible?.recovered || !valid() || !online || inFlight.current) return undefined;
    if (JSON.stringify(next.scope) !== scopeKey || path && next.path !== path) return undefined;
    if (visible.request && (!visible.request.receipt || visible.request.receipt.status === "pending") && (!visible.retryAllowed || visible.request.id !== next.id)) return undefined;
    inFlight.current = true;
    setView({ ...visible, busy: true, error: "", retryAllowed: false });
    let persisted = false;
    try {
      await writeJobRecord(next.scope, "request", next, next.id, revision);
      if (!valid()) return undefined;
      persisted = true;
      setView({ ...visible, request: next, busy: true, error: "", retryAllowed: false });
      const saved = await sendJobRequest(next);
      if (!valid()) return undefined;
      setView({ ...visible, request: saved, busy: false, error: "", retryAllowed: false });
      return saved;
    } catch (failure) {
      if (valid()) setView({ ...visible, request: persisted ? next : visible.request, busy: false, retryAllowed: false, error: `${failure instanceof Error ? failure.message : "Request failed."} ${persisted ? "Check the original receipt before trying again." : "The request could not be saved; nothing was submitted."}` });
      return undefined;
    } finally { if (valid()) inFlight.current = false; }
  };
  return { request: visible?.request, busy: visible?.busy ?? false, error: visible?.error ?? "", retryAllowed: visible?.retryAllowed ?? false, resolve, submit, unresolved: !visible?.recovered || Boolean(visible.request && (!visible.request.receipt || visible.request.receipt.status === "pending")) };
}
