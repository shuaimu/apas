import { useCallback, useEffect, useRef, useState } from "react";
import { randomUUID } from "expo-crypto";
import { useFocusEffect } from "expo-router";
import type { JobAdmissionRequest, JobAnswerRequest, JobMutationReceipt, JobMutationRequest } from "@apas/protocol";
import { ApiError } from "@/api/client";
import { jobsApi } from "./api";
import { jobControlsReady, purgeJobAccess, selectJob, synchronizeJob, unselectJob, useJobs } from "./runtime";
import { readJobRecord, writeJobRecord } from "./storage";
import type { JobScope } from "./storage";

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
  const save = (next: T) => {
    if (!scope || !loaded || useJobs.getState().account !== scope.account) return;
    setValue(next); setSaving(true); setError(null);
    const owner = scopeKey;
    const version = ++saveVersion.current;
    pending.current = writeJobRecord(scope, key, next);
    void pending.current.then(() => { if (current.current === owner && saveVersion.current === version) setSaving(false); }).catch((cause) => {
      if (current.current === owner && saveVersion.current === version) { setSaving(false); setError(`Draft was not saved: ${String(cause)}`); }
    });
  };
  const flush = async () => {
    if (!scope || !loaded || useJobs.getState().account !== scope.account) throw new Error("Protected draft storage is not ready");
    await pending.current;
    await writeJobRecord(scope, key, value);
  };
  return { value, save, loaded, saving, error, flush };
}

type SavedMutation =
  | { kind: "admit"; body: JobAdmissionRequest }
  | { kind: "answer"; question: string; body: JobAnswerRequest }
  | { kind: "cancel" | "retry" | "retry-publication"; body: JobMutationRequest };
interface MutationRecord { mutation: SavedMutation; receipt: JobMutationReceipt | null }

export function useJobMutation(scope: JobScope | null, key: string) {
  const [record, setRecord] = useState<MutationRecord | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const sending = useRef(false);
  const [canRetry, setCanRetry] = useState(false);
  const canRead = useJobs((state) => state.online && state.foreground && state.authorized);
  const latestRecord = useRef(record);
  useEffect(() => { latestRecord.current = record; }, [record]);
  const savedRequestId = record?.mutation.body.request_id;
  const serialized = scope ? JSON.stringify(scope) : "";
  const scopeKey = JSON.stringify([serialized, key]);
  const [renderedScope, setRenderedScope] = useState(scopeKey);
  const [wasReadable, setWasReadable] = useState(canRead);
  if (renderedScope !== scopeKey) {
    setRenderedScope(scopeKey); setLoaded(false); setRecord(null); setBusy(false); setMessage(null); setCanRetry(false);
  }
  if (wasReadable !== canRead) { setWasReadable(canRead); setCanRetry(false); }
  const identity = useRef(scopeKey);
  useEffect(() => {
    identity.current = scopeKey;
    return () => { identity.current = ""; };
  }, [scopeKey]);
  const resolve = useCallback(async (saved: MutationRecord, owner: JobScope) => {
    const ownerKey = JSON.stringify([JSON.stringify(owner), key]);
    try {
      const receipt = await jobsApi.receipt(saved.mutation.body.request_id);
      if (identity.current !== ownerKey || useJobs.getState().account !== owner.account) return;
      const confirmed = { ...saved, receipt };
      await writeJobRecord(owner, key, confirmed);
      if (identity.current !== ownerKey || useJobs.getState().account !== owner.account) return;
      setRecord(confirmed); setCanRetry(false);
      setMessage(receipt.message ?? `Request ${receipt.status}. Answers require separate provider confirmation.`);
    } catch (error) {
      if (identity.current !== ownerKey || useJobs.getState().account !== owner.account) return;
      setCanRetry(error instanceof ApiError && error.status === 404);
      setMessage(error instanceof ApiError && error.status === 404
        ? "No receipt found. The result is unconfirmed. Only an explicit retry with this same request identity is permitted."
        : `Receipt is unconfirmed; no request was replayed. ${String(error)}`);
    }
  }, [key]);
  useEffect(() => {
    let live = true;
    if (!serialized) return;
    const owner = JSON.parse(serialized) as JobScope;
    void readJobRecord<MutationRecord>(owner, key).then((saved) => {
      if (!live) return;
      setRecord(saved); setLoaded(true);
    }).catch((error) => { if (live) setMessage(`Request identity could not be restored: ${String(error)}`); });
    return () => { live = false; };
  }, [serialized, key, resolve]);
  useEffect(() => {
    const saved = latestRecord.current;
    if (canRead && serialized && saved) void resolve(saved, JSON.parse(serialized) as JobScope);
  }, [canRead, serialized, savedRequestId, resolve]);
  const send = async (mutation: SavedMutation) => {
    if (!scope || !loaded || sending.current) throw new Error("Request storage is not ready");
    const state = useJobs.getState();
    if (state.account !== scope.account || !state.authorized || !state.online || !state.foreground || !state.mutations || (scope.job && !jobControlsReady(scope.job))) throw new Error("Synchronize this job before changing it");
    setBusy(true); setMessage(null); setCanRetry(false);
    sending.current = true;
    const ownerKey = scopeKey;
    try {
      const saved = { mutation, receipt: null };
      await writeJobRecord(scope, key, saved);
      if (identity.current !== ownerKey || useJobs.getState().account !== scope.account) return;
      setRecord(saved);
      const receipt = mutation.kind === "admit" ? await jobsApi.admit(scope.project, mutation.body)
        : mutation.kind === "answer" ? await jobsApi.answer(scope.project, scope.job, mutation.question, mutation.body)
        : await jobsApi.mutate(scope.project, scope.job, mutation.kind, mutation.body);
      if (identity.current !== ownerKey || useJobs.getState().account !== scope.account) return;
      const confirmed = { mutation, receipt };
      await writeJobRecord(scope, key, confirmed);
      if (identity.current !== ownerKey || useJobs.getState().account !== scope.account) return;
      setRecord(confirmed); setMessage(receipt.message ?? `Request ${receipt.status}`);
      if (scope.job) await synchronizeJob(scope.job, scope.project);
    } catch (error) {
      if (error instanceof ApiError && [401, 403].includes(error.status)) await purgeJobAccess(error.status === 401 ? undefined : scope.project);
      if (identity.current === ownerKey && useJobs.getState().account === scope.account) setMessage(`Unconfirmed request; resolve its receipt before retrying. ${String(error)}`);
    } finally { sending.current = false; if (identity.current === ownerKey) setBusy(false); }
  };
  return {
    record, loaded, busy, message, canRetry,
    submit: async (mutation: SavedMutation) => { if (record) throw new Error("Resolve the saved request first"); await send(mutation); },
    retry: async () => {
      if (!canRetry) throw new Error("Resolve the original receipt before explicitly retrying");
      if (record) await send(record.mutation);
    },
    resolve: async () => { if (record && scope) await resolve(record, scope); },
    clear: async () => {
      if (!scope || !record?.receipt || record.receipt.status === "pending") return;
      await writeJobRecord(scope, key, null);
      if (identity.current === scopeKey && useJobs.getState().account === scope.account) { setRecord(null); setMessage(null); setCanRetry(false); }
    },
    requestId: randomUUID,
  };
}
