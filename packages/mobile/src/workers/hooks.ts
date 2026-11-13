import { useCallback, useEffect, useRef, useState } from "react";
import { useFocusEffect } from "expo-router";
import { useQuery } from "@tanstack/react-query";
import { randomUUID } from "expo-crypto";
import type { WorkerApprovalRequest, WorkerItemRequest, WorkerMutationRequest, WorkerMutationReceipt, JobAnswerRequest, JobMutationRequest, JobMutationReceipt } from "@apas/protocol";
import { ApiError } from "@/api/client";
import { queryClient } from "@/query/client";
import { useJobDraft } from "@/jobs/hooks";
import { authorizeJobStorageProject, readJobRecord, writeJobRecord } from "@/jobs/storage";
import { workersApi } from "./api";
import { authorityCurrent, handleWorkerError, selectWorker, synchronizeWorker, unselectWorker, useWorkers, workerAuthority, workerControlsReady, workerStorageScope } from "./runtime";
import { validateWorkerReceipt } from "./continuity";
import type { WorkerScope } from "./runtime";

export function useWorker(worker: string, project?: string) {
  const entry = useWorkers((state) => state.entries[worker]);
  const account = useWorkers((state) => state.account);
  const online = useWorkers((state) => state.online && state.foreground && state.authorized);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => { if (!worker) return; setError(null); try { await selectWorker(worker, project); } catch (cause) { setError(String(cause)); } }, [worker, project]);
  useFocusEffect(useCallback(() => { if (account && worker && online) void refresh(); return () => unselectWorker(worker); }, [worker, account, online, refresh]));
  return { entry, error, refresh, ready: online && workerControlsReady(worker) };
}
export function useWorkerDraft<T>(scope: WorkerScope | null, key: string, initial: T) {
  const revoked = useWorkers((state) => scope ? Boolean(state.revoked[scope.project]) : false);
  return useJobDraft(scope && !revoked ? workerStorageScope(scope) : null, key, initial);
}
export function useWorkerEvidence<T>(scope: WorkerScope | null, key: string, load: (signal: AbortSignal) => Promise<T>, ready: boolean) {
  const online = useWorkers((state) => state.online && state.foreground && state.authorized);
  const revoked = useWorkers((state) => scope ? Boolean(state.revoked[scope.project]) : false);
  const query = useQuery({ queryKey: ["workers", scope?.account, scope?.project, scope?.worker, key, online, ready], enabled: Boolean(scope) && !revoked, retry: false,
    queryFn: async ({ signal }) => {
      if (!scope) throw new Error("Worker account is unavailable");
      const stamp = workerAuthority();
      if (!online || !ready) { const cached = await readJobRecord<T>(workerStorageScope(scope), key); if (!authorityCurrent(stamp) || signal.aborted) throw new Error("Worker authority changed"); if (cached === null) throw new Error("Not cached; authorize and synchronize this exact worker"); return cached; }
      try {
        const value = await load(signal);
        if (!authorityCurrent(stamp) || signal.aborted) throw new Error("Worker authority changed");
        if (scope.project) {
          authorizeJobStorageProject(scope.account, scope.project);
          useWorkers.setState((state) => ({ revoked: Object.fromEntries(Object.entries(state.revoked).filter(([id]) => id !== scope.project)) }));
        }
        if (!scope.project && value && typeof value === "object" && "workers" in value && Array.isArray(value.workers)) {
          for (const worker of value.workers) {
            if (typeof worker.project_id !== "string") throw new Error("Invalid worker inventory project");
            await writeJobRecord(workerStorageScope({ ...scope, project: worker.project_id }), "inventory-project", true);
            if (!authorityCurrent(stamp)) throw new Error("Worker authority changed");
          }
        }
        await writeJobRecord(workerStorageScope(scope), key, value);
        if (!authorityCurrent(stamp)) throw new Error("Worker authority changed");
        return value;
      }
      catch (error) { if (authorityCurrent(stamp)) await handleWorkerError(error, scope.project || undefined); throw error; }
    },
  });
  return { ...query, data: scope && !revoked ? query.data : undefined };
}
export type WorkerMutation =
  | { kind: "approve"; body: WorkerApprovalRequest }
  | { kind: "pause" | "stop" | "resume" | "wake"; body: WorkerMutationRequest }
  | { kind: "item"; body: WorkerItemRequest }
  | { kind: "dismiss" | "retry"; item: string; body: WorkerMutationRequest }
  | { kind: "answer"; run: string; question: string; body: JobAnswerRequest }
  | { kind: "retry-publication"; run: string; body: JobMutationRequest };
export interface WorkerRequestRecord { mutation: WorkerMutation; receipt: WorkerMutationReceipt | JobMutationReceipt | null }
export async function dispatchWorkerMutation(scope: WorkerScope, mutation: WorkerMutation) {
  switch (mutation.kind) {
    case "approve": return workersApi.approve(scope.project, scope.worker, mutation.body);
    case "item": return workersApi.item(scope.project, scope.worker, mutation.body);
    case "dismiss": case "retry": return workersApi.itemAction(scope.project, scope.worker, mutation.item, mutation.kind, mutation.body);
    case "answer": return workersApi.answer(scope.project, scope.worker, mutation.run, mutation.question, mutation.body);
    case "retry-publication": return workersApi.retryPublication(scope.project, scope.worker, mutation.run, mutation.body);
    default: return workersApi.mutate(scope.project, scope.worker, mutation.kind, mutation.body);
  }
}
export function useWorkerMutation(scope: WorkerScope | null, key: string) {
  const saved = useWorkerDraft<WorkerRequestRecord | null>(scope, key, null);
  const readable = useWorkers((state) => state.authorized && state.online && state.foreground);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [canRetry, setCanRetry] = useState(false);
  const sending = useRef(false);
  const identity = JSON.stringify([scope, key]);
  const [renderedIdentity, setRenderedIdentity] = useState(identity);
  if (renderedIdentity !== identity) {
    setRenderedIdentity(identity); setBusy(false); setMessage(null); setCanRetry(false);
  }
  const serializedScope = scope ? JSON.stringify(scope) : "";
  const record = saved.value;
  const save = saved.save;
  const recovery = useRef("");
  const mounted = useRef(identity);
  useEffect(() => { mounted.current = identity; return () => { mounted.current = ""; }; }, [identity]);
  const resolve = useCallback(async () => {
    if (!serializedScope || !record || !readable) return;
    const scope = JSON.parse(serializedScope) as WorkerScope;
    const stamp = workerAuthority();
    try {
      const receipt = record.mutation.kind === "answer" || record.mutation.kind === "retry-publication" ? await workersApi.executionReceipt(record.mutation.body.request_id) : await workersApi.receipt(record.mutation.body.request_id);
      if (!authorityCurrent(stamp) || mounted.current !== identity) return;
      validateWorkerReceipt(receipt, record.mutation.body.request_id, scope.project, scope.worker, "run" in record.mutation ? record.mutation.run : undefined);
      const next = { ...record, receipt };
      await writeJobRecord(workerStorageScope(scope), key, next);
      if (!authorityCurrent(stamp) || mounted.current !== identity) return;
      save(next); setCanRetry(false); setMessage(`Durable request ${receipt.status}; acceptance is not execution, stopping, or provider confirmation.`);
    } catch (error) {
      if (!authorityCurrent(stamp) || mounted.current !== identity) return;
      setCanRetry(error instanceof ApiError && error.status === 404);
      setMessage(`Receipt unresolved. No mutation was replayed. ${String(error)}`);
      if (error instanceof ApiError && [401, 403].includes(error.status)) await handleWorkerError(error, scope.project);
    }
  }, [serializedScope, identity, key, readable, record, save]);
  useEffect(() => {
    if (!readable) { recovery.current = ""; return; }
    if (!saved.loaded || !record) return;
    const request = JSON.stringify([identity, record.mutation.body.request_id]);
    if (recovery.current === request) return;
    recovery.current = request;
    void resolve();
  }, [saved.loaded, record, readable, identity, resolve]);
  const send = async (mutation: WorkerMutation, retry = false) => {
    if (!scope || !saved.loaded || sending.current || (saved.value && !retry)) throw new Error("Resolve the original request before creating another identity");
    const state = useWorkers.getState();
    if (state.account !== scope.account || !state.authorized || !state.online || !state.foreground || !state.mutations || (scope.worker && !workerControlsReady(scope.worker))) throw new Error("Synchronize current worker authority before mutation");
    const stamp = workerAuthority(); sending.current = true; setBusy(true); setCanRetry(false);
    try {
      const record = { mutation, receipt: null };
      await writeJobRecord(workerStorageScope(scope), key, record);
      if (!authorityCurrent(stamp) || mounted.current !== identity) return;
      saved.save(record);
      const receipt = await dispatchWorkerMutation(scope, mutation);
      if (!authorityCurrent(stamp) || mounted.current !== identity) return;
      validateWorkerReceipt(receipt, mutation.body.request_id, scope.project, scope.worker, "run" in mutation ? mutation.run : undefined);
      await writeJobRecord(workerStorageScope(scope), key, { mutation, receipt });
      if (!authorityCurrent(stamp) || mounted.current !== identity) return;
      saved.save({ mutation, receipt }); setMessage(`Durable request ${receipt.status}. Current execution state is shown separately.`);
      if (scope.worker) await synchronizeWorker(scope.worker, scope.project);
      await queryClient.invalidateQueries({ queryKey: ["workers", scope.account, scope.project, scope.worker] });
    } catch (error) {
      if (authorityCurrent(stamp) && mounted.current === identity) { setMessage(`Unconfirmed request; resolve its receipt before retrying. ${String(error)}`); if (error instanceof ApiError && [401, 403].includes(error.status)) await handleWorkerError(error, scope.project); else if (error instanceof ApiError && error.status === 409 && scope.worker) await synchronizeWorker(scope.worker, scope.project); }
    } finally { sending.current = false; if (mounted.current === identity) setBusy(false); }
  };
  return { record: saved.value, loaded: saved.loaded, busy, message: message ?? saved.error, canRetry, resolve, requestId: randomUUID,
    submit: async (mutation: WorkerMutation) => { try { await send(mutation); } catch (error) { setMessage(String(error)); } },
    retry: async () => {
      if (!canRetry || !saved.value) { setMessage("Resolve the original receipt before explicit same-ID retry"); return; }
      try { await send(saved.value.mutation, true); } catch (error) { setMessage(String(error)); }
    },
    clear: async () => {
      try {
        if (scope && saved.value?.receipt && saved.value.receipt.status !== "pending") {
          await writeJobRecord(workerStorageScope(scope), key, null); saved.save(null); setMessage(null); setCanRetry(false);
        }
      } catch (error) { setMessage(`Receipt could not be retired: ${String(error)}`); }
    },
  };
}
