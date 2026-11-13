import { useCallback, useEffect, useRef, useState } from "react";
import { useFocusEffect } from "expo-router";
import type { JobAdmissionRequest, JobAnswerRequest, JobMutationReceipt, JobMutationRequest } from "@apas/protocol";
import { jobsApi } from "./api";
import { jobControlsReady, selectJob, unselectJob, useJobs } from "./runtime";
import { readJobRecord, writeJobRecord } from "./storage";
import type { JobScope } from "./storage";

import { authorityCurrent, workerAuthority } from "@/workers/runtime";
export function useJob(job: string, project?: string) {
  const account = useJobs((state) => state.account);
  const entry = useJobs((state) => state.entries[job]);
  const online = useJobs((state) => state.online && state.foreground && state.authorized);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    if (!useJobs.getState().account) return;
    setError(null);
    try { await selectJob(job, project); } catch (cause) { setError(String(cause)); }
  }, [job, project]);
  useFocusEffect(useCallback(() => { if (account && online) void refresh(); return () => unselectJob(job); }, [refresh, account, online, job]));
  return { entry, error: error ?? entry?.error, refresh, ready: online && Boolean(entry?.ready) && jobControlsReady(job) };
}

export function useJobDraft<T>(scope: JobScope | null, key: string, initial: T) {
  const [value, setValue] = useState(initial);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const serialized = scope ? JSON.stringify(scope) : "";
  const scopeKey = JSON.stringify([serialized, key]);
  const [renderedScope, setRenderedScope] = useState(scopeKey);
  if (renderedScope !== scopeKey) {
    setRenderedScope(scopeKey); setLoaded(false); setValue(initial); setSaving(false); setError(null);
  }
  const current = useRef(scopeKey);
  useEffect(() => {
    current.current = scopeKey;
    return () => { current.current = ""; };
  }, [scopeKey]);
  const pending = useRef(Promise.resolve());
  const saveVersion = useRef(0);
  useEffect(() => {
    let live = true;
    if (!serialized) return;
    void readJobRecord<T>(JSON.parse(serialized) as JobScope, key).then((saved) => {
      if (live) { if (saved !== null) setValue(saved); setLoaded(true); }
    }).catch((cause) => { if (live) setError(`Draft could not be restored: ${String(cause)}`); });
    return () => { live = false; };
  }, [serialized, key]);
  const save = useCallback((next: T) => {
    if (!serialized || !loaded || current.current !== scopeKey) return;
    const scope = JSON.parse(serialized) as JobScope;
    if (useJobs.getState().account !== scope.account) return;
    setValue(next); setSaving(true); setError(null);
    const owner = scopeKey;
    const version = ++saveVersion.current;
    pending.current = writeJobRecord(scope, key, next);
    void pending.current.then(() => { if (current.current === owner && saveVersion.current === version) setSaving(false); }).catch((cause) => {
      if (current.current === owner && saveVersion.current === version) { setSaving(false); setError(`Draft was not saved: ${String(cause)}`); }
    });
  }, [serialized, loaded, scopeKey, key]);
  const flush = async () => {
    if (!scope || !loaded || current.current !== scopeKey || useJobs.getState().account !== scope.account) throw new Error("Protected draft storage is not ready");
    await pending.current;
    if (current.current !== scopeKey || useJobs.getState().account !== scope.account) throw new Error("Protected draft authority changed");
    await writeJobRecord(scope, key, value);
  };
  return { value, save, loaded, saving, error, flush };
}

type SavedMutation =
  | { kind: "admit"; body: JobAdmissionRequest }
  | { kind: "answer"; question: string; body: JobAnswerRequest }
  | { kind: "cancel" | "retry" | "retry-publication"; body: JobMutationRequest };
interface MutationRecord { mutation: SavedMutation; receipt: JobMutationReceipt | null }

export function useJobReceipt(scope: JobScope | null, key: string) {
  const [stored, setStored] = useState<{ owner: string; record: MutationRecord | null; message: string | null } | null>(null);
  const serialized = scope ? JSON.stringify(scope) : "";
  const owner = JSON.stringify([serialized, key]);
  const record = stored?.owner === owner ? stored.record : null;
  const message = stored?.owner === owner ? stored.message : null;
  const canRead = useJobs((state) => state.online && state.foreground && state.authorized);
  const currentScope = useRef("");
  useEffect(() => {
    let live = true;
    currentScope.current = owner;
    if (serialized) void readJobRecord<MutationRecord>(JSON.parse(serialized) as JobScope, key)
      .then((record) => { if (live) setStored({ owner, record, message: null }); })
      .catch((error) => { if (live) setStored({ owner, record: null, message: String(error) }); });
    return () => { live = false; currentScope.current = ""; };
  }, [serialized, key, owner]);
  const resolve = async () => {
    if (!scope || !record || !canRead) return;
    const stamp = workerAuthority();
    try {
      const receipt = await jobsApi.receipt(record.mutation.body.request_id);
      if (!authorityCurrent(stamp) || currentScope.current !== owner || useJobs.getState().account !== scope.account) return;
      const confirmed = { ...record, receipt };
      await writeJobRecord(scope, key, confirmed);
      if (!authorityCurrent(stamp) || currentScope.current !== owner) return;
      setStored({ owner, record: confirmed, message: `Earlier request ${receipt.status}. Retired operations cannot be replayed.` });
    } catch (error) { if (authorityCurrent(stamp) && currentScope.current === owner) setStored({ owner, record, message: `Receipt remains unconfirmed; no request was resent. ${String(error)}` }); }
  };
  return { record, message, resolve };
}
