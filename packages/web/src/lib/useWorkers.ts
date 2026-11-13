"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useStore } from "./store";
import { useJobOnline } from "./useJobs";
import { onWorkerCachePurge, persistWorkerRequest, purgeWorkerCache, readWorkerRecord, workerCacheRevision, workerRequests, writeWorkerRecord } from "./workerCache";
import type { SavedWorker, SavedWorkerRequest, WorkerScope } from "./workerCache";
import { onWorkerMessage, resolveWorkerProject, resolveWorkerRequest, sendWorkerRequest, synchronizeWorker, WorkerHttpError } from "./workers";

export function useWorkerOnline() {
  const online = useJobOnline();
  const capable = useStore((state) => state.negotiatedCapabilities.has("autonomous_workers_v1"));
  const active = useStore((state) => state.accountStatus === "active");
  return online && capable && active;
}
interface WorkerView {
  key: string;
  revision: number;
  socket: WebSocket | null;
  project: string;
  saved?: SavedWorker;
  synced: boolean;
  error: string;
}
export function useWorker(id: string, projectHint?: string | null) {
  const account = useStore((state) => state.userId);
  const token = useStore((state) => state.token);
  const socket = useStore((state) => state.ws);
  const online = useWorkerOnline();
  const revision = useSyncExternalStore(onWorkerCachePurge, workerCacheRevision, () => 0);
  const key = JSON.stringify([account, token, id, projectHint]);
  const [view, setView] = useState<WorkerView>();
  const current = useRef<WorkerView | undefined>(undefined);
  const previousRevision = useRef(revision);
  const blocked = useRef(false);
  const control = useRef<{ key: string; revision: number; refresh: () => void } | undefined>(undefined);

  useEffect(() => {
    if (previousRevision.current !== revision) blocked.current = true;
    previousRevision.current = revision;
    if (!account || !id) { current.current = undefined; setView(undefined); return; }
    let active = true;
    let busy = true;
    let pending = false;
    let subscribed = false;
    const retained = current.current?.key === key && current.current.revision === revision ? current.current : undefined;
    let state: WorkerView = { key, revision, socket, project: projectHint ?? retained?.project ?? "", saved: retained?.saved, synced: false, error: blocked.current ? "Worker access changed. Refresh to check current access." : "" };
    const valid = () => active && workerCacheRevision() === revision && useStore.getState().userId === account && useStore.getState().token === token;
    const publish = (changes: Partial<WorkerView>) => {
      if (!valid()) return;
      state = { ...state, ...changes };
      current.current = state;
      setView(state);
    };
    publish({});
    const run = async () => {
      if (!valid() || !online || blocked.current) return;
      if (busy) { pending = true; publish({ synced: false }); return; }
      busy = true;
      publish({ synced: false, error: "" });
      try {
        const project = state.project || await resolveWorkerProject(id, projectHint);
        if (!valid()) return;
        publish({ project });
        const saved = await synchronizeWorker([account, project, id]);
        if (!valid()) return;
        publish({ saved, synced: !pending });
        if (socket?.readyState === WebSocket.OPEN && useStore.getState().ws === socket) {
          socket.send(JSON.stringify({ type: "subscribe_worker", project_id: project, worker_id: id, after_seq: saved.cursor }));
          subscribed = true;
        }
      } catch (failure) {
        if (valid()) publish({ synced: false, error: failure instanceof Error ? failure.message : "Could not synchronize worker." });
      } finally {
        busy = false;
        if (valid() && pending) { pending = false; void run(); }
      }
    };
    control.current = { key, revision, refresh: () => { blocked.current = false; void run(); } };
    const restore = async () => {
      if (!blocked.current && state.project && !retained?.saved) {
        try {
          const saved = await readWorkerRecord<SavedWorker>([account, state.project, id], "snapshot");
          if (saved?.detail.worker.id === id && saved.detail.worker.project_id === state.project) publish({ saved });
        } catch (failure) { publish({ error: failure instanceof Error ? failure.message : "Worker cache could not be restored." }); }
      }
      busy = false;
      pending = false;
      if (valid()) await run();
    };
    void restore();
    const stop = onWorkerMessage((message) => {
      if (!valid() || blocked.current || !online || useStore.getState().ws !== socket || message.type === "worker_inbox_changed") return;
      const summary = message.type === "worker_snapshot" ? message.detail.worker : undefined;
      const messageId = summary?.id ?? (message.type === "worker_event" ? message.event.worker_id : message.type === "worker_subscription_rejected" ? message.worker_id : "");
      const project = summary?.project_id ?? (message.type === "worker_event" ? message.event.project_id : message.type === "worker_subscription_rejected" ? message.project_id : "");
      if (messageId !== id || project !== state.project || !subscribed) return;
      if (message.type === "worker_subscription_rejected" && !message.resync_required) {
        publish({ saved: undefined, synced: false, error: `Worker access rejected (${message.code}).` });
        blocked.current = true;
        void purgeWorkerCache(account, project).catch(() => useStore.getState().showToast("Worker cache purge failed. Clear this site's browser data.", "error"));
        return;
      }
      if (message.type === "worker_event" && state.saved && message.event.seq <= state.saved.cursor) return;
      if (summary && state.saved && summary.high_watermark <= state.saved.cursor && summary.state_version <= state.saved.detail.worker.state_version) return;
      void run();
    });
    const timer = window.setInterval(() => { void run(); }, 15_000);
    return () => {
      active = false;
      stop();
      clearInterval(timer);
      if (subscribed && socket?.readyState === WebSocket.OPEN && useStore.getState().ws === socket && useStore.getState().userId === account && useStore.getState().token === token) socket.send(JSON.stringify({ type: "unsubscribe_worker", worker_id: id }));
      if (control.current?.key === key && control.current.revision === revision) control.current = undefined;
    };
  }, [account, token, id, projectHint, key, online, socket, revision]);

  const refresh = useCallback(() => {
    if (control.current?.key === key && control.current.revision === revision) control.current.refresh();
  }, [key, revision]);
  const visible = view?.key === key && view.revision === revision ? view : undefined;
  return { saved: visible?.saved, project: visible?.project ?? projectHint ?? "", ready: Boolean(online && visible?.synced && visible.socket === socket), error: visible?.error ?? "", refresh };
}

/** Final submit awaits the exact latest draft write; no timer can lose the final edit. */
export function useWorkerDraft<T>(scope: WorkerScope, id: string, initial: T) {
  const account = useStore((state) => state.userId);
  const revision = useSyncExternalStore(onWorkerCachePurge, workerCacheRevision, () => 0);
  const key = JSON.stringify([scope, id, revision]);
  const [draft, setDraft] = useState<{ key: string; value: T; loaded: boolean; error: string }>();
  const initialValue = useRef(initial);
  initialValue.current = initial;
  const liveKey = useRef(key);
  liveKey.current = key;
  const writes = useRef<Promise<void>>(Promise.resolve());
  const latest = useRef<{ key: string; value: T }>({ key, value: initial });
  const previousRevision = useRef(revision);
  const scopeKey = JSON.stringify(scope);
  useEffect(() => {
    const scope = JSON.parse(scopeKey) as WorkerScope;
    let active = true;
    const purged = previousRevision.current !== revision;
    previousRevision.current = revision;
    latest.current = { key, value: initialValue.current };
    writes.current = Promise.resolve();
    setDraft({ key, value: initialValue.current, loaded: false, error: purged ? "Access changed. Draft cleared from this view." : "" });
    if (purged || !account || account !== scope[0]) return;
    void readWorkerRecord<T>(scope, "draft", id).then((saved) => {
      if (!active || liveKey.current !== key || workerCacheRevision() !== revision || useStore.getState().userId !== account) return;
      const value = saved ?? initialValue.current;
      latest.current = { key, value };
      setDraft({ key, value, loaded: true, error: "" });
    }).catch((failure: unknown) => {
      if (active && liveKey.current === key && workerCacheRevision() === revision) setDraft({ key, value: initialValue.current, loaded: false, error: failure instanceof Error ? failure.message : "Draft storage is unavailable." });
    });
    return () => { active = false; };
  }, [scopeKey, id, key, revision, account]);
  const visible = draft?.key === key && account === scope[0] ? draft : undefined;
  const update = (next: T) => {
    if (!visible?.loaded || workerCacheRevision() !== revision || useStore.getState().userId !== scope[0]) return;
    latest.current = { key, value: next };
    setDraft({ key, value: next, loaded: true, error: "" });
    writes.current = writeWorkerRecord(scope, "draft", next, id, revision);
    void writes.current.catch((failure: unknown) => {
      if (liveKey.current === key && workerCacheRevision() === revision) setDraft((value) => value?.key === key ? { ...value, error: failure instanceof Error ? failure.message : "Draft was not saved." } : value);
    });
  };
  const flush = async () => {
    if (!visible?.loaded || liveKey.current !== key || workerCacheRevision() !== revision || useStore.getState().userId !== scope[0]) throw new Error("Worker draft access changed. Nothing was submitted.");
    await writes.current;
    if (latest.current.key !== key) throw new Error("Worker draft scope changed. Nothing was submitted.");
    await writeWorkerRecord(scope, "draft", latest.current.value, id, revision);
  };
  return { value: visible?.value ?? initial, update, flush, loaded: visible?.loaded ?? false, error: visible?.error ?? "" };
}

interface MutationView {
  key: string;
  request?: SavedWorkerRequest;
  busy: boolean;
  error: string;
  retryAllowed: boolean;
  recovered: boolean;
}
export function useWorkerMutation(scope: WorkerScope, path?: string, approvalPreviewId?: string): WorkerMutationState {
  const account = useStore((state) => state.userId);
  const token = useStore((state) => state.token);
  const online = useWorkerOnline();
  const revision = useSyncExternalStore(onWorkerCachePurge, workerCacheRevision, () => 0);
  const scopeKey = JSON.stringify(scope);
  const key = JSON.stringify([scopeKey, path, approvalPreviewId, revision, account, token]);
  const [view, setView] = useState<MutationView>();
  const liveKey = useRef(key);
  liveKey.current = key;
  const busy = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const valid = () => mounted.current && liveKey.current === key && workerCacheRevision() === revision && useStore.getState().userId === scope[0] && useStore.getState().token === token;
  const publish = (changes: Partial<MutationView>) => {
    if (valid()) setView((value) => ({ key, busy: false, error: "", retryAllowed: false, recovered: false, ...(value?.key === key ? value : {}), ...changes }));
  };
  useEffect(() => {
    const scope = JSON.parse(scopeKey) as WorkerScope;
    let active = true;
    busy.current = false;
    setView({ key, busy: false, error: "", retryAllowed: false, recovered: false });
    if (!account || account !== scope[0]) return;
    const current = () => active && liveKey.current === key && workerCacheRevision() === revision && useStore.getState().userId === account && useStore.getState().token === token;
    void workerRequests(account).then(async (requests) => {
      if (!current()) return;
      const request = requests.find((item) => JSON.stringify(item.scope) === scopeKey && (!path || item.path === path) && ((!item.receipt || item.receipt.status === "pending") || Boolean(approvalPreviewId && "preview_id" in item.body && item.body.preview_id === approvalPreviewId)));
      const next: MutationView = { key, request, busy: false, error: "", retryAllowed: false, recovered: true };
      setView(next);
      if (!request || request.receipt && request.receipt.status !== "pending" || !online) return;
      try {
        const resolved = await resolveWorkerRequest(request);
        if (current()) setView({ ...next, request: resolved });
      } catch (failure) {
        if (!current()) return;
        const missing = failure instanceof WorkerHttpError && failure.status === 404;
        setView({ ...next, retryAllowed: missing, error: missing ? "No receipt found. Refresh current worker state, then explicitly retry the saved request." : "Receipt is unconfirmed. Check again before retrying." });
      }
    }).catch((failure: unknown) => {
      if (current()) setView({ key, busy: false, error: failure instanceof Error ? failure.message : "Worker request recovery failed.", retryAllowed: false, recovered: false });
    });
    return () => { active = false; };
  }, [key, scopeKey, account, token, path, approvalPreviewId, revision, online]);
  const visible = view?.key === key && account === scope[0] ? view : undefined;
  const resolve = async () => {
    if (!visible?.request || !valid() || !online || busy.current) return;
    busy.current = true;
    publish({ busy: true, error: "", retryAllowed: false });
    try { const request = await resolveWorkerRequest(visible.request); publish({ request, recovered: true }); }
    catch (failure) {
      const missing = failure instanceof WorkerHttpError && failure.status === 404;
      publish({ retryAllowed: missing, error: missing ? "No receipt found; same-ID retry is available after synchronization." : "Still unconfirmed. No mutation has been replayed." });
    } finally { if (valid()) { busy.current = false; publish({ busy: false }); } }
  };
  const submit = async (next: SavedWorkerRequest) => {
    if (!valid() || !online || busy.current || !visible?.recovered) return undefined;
    if (JSON.stringify(next.scope) !== scopeKey || path && next.path !== path) { publish({ error: "The request does not belong to this worker view." }); return undefined; }
    if (visible.request && (!visible.request.receipt || visible.request.receipt.status === "pending") && (!visible.retryAllowed || visible.request.id !== next.id)) {
      publish({ error: "Resolve the original receipt before explicitly retrying the same request." }); return undefined;
    }
    busy.current = true;
    publish({ busy: true, error: "", retryAllowed: false });
    let persisted = false;
    try {
      const request = await persistWorkerRequest(next, revision);
      if (!valid()) return undefined;
      persisted = true;
      publish({ request });
      const saved = await sendWorkerRequest(request);
      if (!valid()) return undefined;
      publish({ request: saved });
      return saved;
    } catch (failure) {
      publish({ error: `${failure instanceof Error ? failure.message : "Worker request failed."} ${persisted ? "Check the original receipt before trying again." : "Nothing was submitted."}` });
      return undefined;
    } finally { if (valid()) { busy.current = false; publish({ busy: false }); } }
  };
  return { request: visible?.request, busy: visible?.busy ?? false, error: visible?.error ?? "", retryAllowed: visible?.retryAllowed ?? false, resolve, submit, unresolved: !visible?.recovered || Boolean(visible.request && (!visible.request.receipt || visible.request.receipt.status === "pending")) };
}
export interface WorkerMutationState {
  request?: SavedWorkerRequest;
  busy: boolean;
  error: string;
  retryAllowed: boolean;
  resolve: () => Promise<void>;
  submit: (next: SavedWorkerRequest) => Promise<SavedWorkerRequest | undefined>;
  unresolved: boolean;
}
