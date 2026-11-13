"use client";

import { useEffect, useRef, useState } from "react";
import { validateJobContract } from "@apas/protocol";
import type { JobLimits, JobPolicy, JobPolicyManagement, JobPolicyOverride, RecurringWorkerPolicy } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { onWorkerCachePurge, workerCacheRevision } from "@/lib/workerCache";

type PolicyRequest = <T>(path: string, init?: RequestInit) => Promise<T>;
type Props = {
  title: string;
  endpoint: string;
  deployment?: boolean;
  request: PolicyRequest;
};

const limitFields: { key: keyof JobLimits; label: string }[] = [
  { key: "elapsed_seconds", label: "Elapsed time limit (seconds, including questions)" },
  { key: "spend_micro_usd", label: "Reported spend threshold (micro-USD; 1 USD = 1,000,000)" },
  { key: "attempts", label: "Maximum attempts" },
  { key: "cpu_millis", label: "CPU limit (millicores)" },
  { key: "memory_bytes", label: "Memory limit (bytes)" },
  { key: "workspace_bytes", label: "Workspace quota (bytes)" },
  { key: "output_bytes", label: "Retained output limit (bytes)" },
  { key: "pids", label: "Process limit" },
  { key: "model_concurrency", label: "Concurrent model requests" },
  { key: "model_output_tokens", label: "Maximum output tokens per request" },
];
const capacityFields = [
  { key: "max_active_project", label: "Concurrent runs per project" },
  { key: "max_active_machine", label: "Concurrent runs per machine" },
  { key: "max_active_cluster", label: "Concurrent runs per cluster" },
  { key: "max_queued_project", label: "Queued jobs per project" },
] as const;
const recurringFields: { key: Exclude<keyof RecurringWorkerPolicy, "enabled">; label: string }[] = [
  { key: "max_elapsed_seconds_24h", label: "Rolling 24-hour elapsed allowance (seconds)" },
  { key: "max_spend_micro_usd_24h", label: "Rolling 24-hour reported spend (micro-USD)" },
  { key: "max_runs_24h", label: "Maximum runs per rolling 24 hours" },
  { key: "min_interval_seconds", label: "Minimum inspection interval (seconds)" },
];
const profileFields = [
  { key: "worker_profiles", label: "Worker profile keys" },
  { key: "model_profiles", label: "Model profile keys" },
  { key: "verification_profiles", label: "Verification profile keys" },
] as const;
const inputClass = "min-h-11 w-full rounded border border-gray-300 bg-transparent px-3 py-2 text-sm dark:border-gray-700";

function policyDocument(value: unknown): JobPolicyManagement {
  if (!validateJobContract("policy_management", value).valid) {
    throw new Error("The server returned an incompatible autonomous job policy.");
  }
  return value as JobPolicyManagement;
}

export function AutonomousJobPolicyEditor({ title, endpoint, deployment = false, request }: Props) {
  const account = useStore((state) => state.userId);
  const generation = useRef(0);
  const [loaded, setLoaded] = useState<{ document: JobPolicyManagement; account: string | null; revision: number; endpoint: string } | null>(null);
  const document = loaded && loaded.account === account && loaded.endpoint === endpoint && loaded.revision === workerCacheRevision() ? loaded.document : null;
  const [error, setError] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [notice, setNotice] = useState("");

  useEffect(() => onWorkerCachePurge(() => {
    generation.current++;
    setLoaded(null);
    setNotice("");
    setError("Access changed. Reload to check current policy authority.");
  }), []);
  useEffect(() => {
    const controller = new AbortController();
    const current = ++generation.current;
    const revision = workerCacheRevision();
    const valid = () => !controller.signal.aborted && current === generation.current
      && revision === workerCacheRevision() && useStore.getState().userId === account;
    setLoaded(null);
    setError("");
    void request<unknown>(endpoint, { signal: controller.signal })
      .then(policyDocument)
      .then((value) => { if (valid()) setLoaded({ document: value, account, revision, endpoint }); })
      .catch((cause) => {
        if (valid()) setError(cause instanceof Error ? cause.message : "Cannot load job policy.");
      });
    return () => { controller.abort(); generation.current++; };
  }, [endpoint, request, refresh, account]);

  return (
    <section className="min-w-0 space-y-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
      <h2 className="font-semibold">{title}</h2>
      <p className="text-sm text-gray-600 dark:text-gray-300">
        Autonomous workers are separate from terminal panes. Recurring permission is independently default denied.
        Policies only narrow deployment and hosting-cluster bounds; permission never substitutes for a qualified, ready host.
      </p>
      {notice && <p role="status" className="text-sm">{notice}</p>}
      {error ? <div role="alert" className="space-y-2 text-sm text-red-700 dark:text-red-300">
        <p>{error}</p>
        <button type="button" className="min-h-11 rounded border px-3" onClick={() => setRefresh((value) => value + 1)}>Reload policy</button>
      </div> : !document ? <p role="status" className="text-sm">Loading job policy…</p> : <>
        {document.unavailable_reasons.length > 0 && <ul className="list-inside list-disc break-words text-sm text-amber-800 dark:text-amber-300">
          {document.unavailable_reasons.map((reason) => <li key={reason}>{reason}</li>)}
        </ul>}
        <details>
          <summary className="min-h-11 cursor-pointer py-3 text-sm">Requested, inherited and effective policy</summary>
          <pre className="max-h-80 overflow-y-auto whitespace-pre-wrap break-words rounded bg-gray-50 p-3 text-xs dark:bg-gray-950">
            {JSON.stringify({ requested: document.requested, inherited: document.inherited, effective: document.effective }, null, 2)}
          </pre>
        </details>
        {document.can_manage ? <PolicyForm
          key={`${endpoint}:${refresh}`}
          document={document}
          deployment={deployment}
          save={async (value) => {
            const current = generation.current;
            const revision = workerCacheRevision();
            if (!document || loaded?.revision !== revision || loaded.account !== useStore.getState().userId) throw new Error("Policy authority changed. Reload before saving.");
            policyDocument(await request<unknown>(endpoint, { method: "PUT", body: JSON.stringify(value) }));
            if (current !== generation.current || revision !== workerCacheRevision() || useStore.getState().userId !== account) return;
            setNotice("Autonomous worker policy saved. Existing and queued work is re-authorized by the server.");
            setRefresh((current) => current + 1);
          }}
        /> : <p className="text-sm text-gray-600 dark:text-gray-300">Your current authority permits viewing this policy, not changing it.</p>}
      </>}
    </section>
  );
}

function PolicyForm({ document, deployment, save }: {
  document: JobPolicyManagement;
  deployment: boolean;
  save: (value: JobPolicy | JobPolicyOverride | Record<string, never>) => Promise<void>;
}) {
  const seed = document.effective?.policy ?? document.inherited?.policy;
  const [enabled, setEnabled] = useState(seed?.enabled ?? false);
  const [publish, setPublish] = useState(seed?.publish_enabled ?? false);
  const [profiles, setProfiles] = useState<Record<string, string>>(() => Object.fromEntries(profileFields.map(({ key }) => [key, seed?.[key].join(", ") ?? ""])));
  const [numbers, setNumbers] = useState<Record<string, string>>(() => Object.fromEntries([
    ...limitFields.map(({ key }) => [key, String(seed?.limits[key] ?? "")]),
    ...capacityFields.map(({ key }) => [key, String(seed?.[key] ?? "")]),
  ]));
  const requestedRecurring = document.requested?.recurring;
  const recurringSeed = requestedRecurring ?? seed?.recurring;
  const [overrideRecurring, setOverrideRecurring] = useState(deployment || requestedRecurring != null);
  const [recurringEnabled, setRecurringEnabled] = useState(recurringSeed?.enabled ?? false);
  const [recurringNumbers, setRecurringNumbers] = useState<Record<string, string>>(() =>
    Object.fromEntries(recurringFields.map(({ key }) => [key, String(recurringSeed?.[key] ?? "")])));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function submit(inherit: boolean) {
    if (busy) return;
    setError("");
    setBusy(true);
    try {
      if (inherit) {
        await save({});
        return;
      }
      const number = (key: string, values = numbers) => {
        const text = values[key]?.trim() ?? "";
        const value = Number(text);
        const minimum = 1;
        if (!/^\d+$/.test(text) || !Number.isSafeInteger(value) || value < minimum) {
          throw new Error(`${key} must be a whole number of at least ${minimum}, within the supported integer range.`);
        }
        return value;
      };
      const profileKeys = (key: string) => [...new Set(profiles[key].split(/[\s,]+/).filter(Boolean))];
      const policy: JobPolicy | JobPolicyOverride = {
        enabled,
        publish_enabled: publish,
        worker_profiles: profileKeys("worker_profiles"),
        model_profiles: profileKeys("model_profiles"),
        verification_profiles: profileKeys("verification_profiles"),
        limits: Object.fromEntries(limitFields.map(({ key }) => [key, number(key)])) as unknown as JobLimits,
        max_active_project: number("max_active_project"),
        max_active_machine: number("max_active_machine"),
        max_active_cluster: number("max_active_cluster"),
        max_queued_project: number("max_queued_project"),
        ...(overrideRecurring ? { recurring: {
          enabled: recurringEnabled,
          max_elapsed_seconds_24h: number("max_elapsed_seconds_24h", recurringNumbers),
          max_spend_micro_usd_24h: number("max_spend_micro_usd_24h", recurringNumbers),
          max_runs_24h: number("max_runs_24h", recurringNumbers),
          min_interval_seconds: number("min_interval_seconds", recurringNumbers),
        } } : {}),
      };
      if (!validateJobContract(deployment ? "deployment_policy" : "policy_override", policy).valid) {
        throw new Error("These values do not satisfy the autonomous job policy contract.");
      }
      await save(policy);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Cannot save job policy.");
    } finally {
      setBusy(false);
    }
  }

  return <form className="space-y-4" onSubmit={(event) => { event.preventDefault(); void submit(false); }}>
    <fieldset disabled={busy} className="min-w-0 space-y-4">
      <legend className="sr-only">Autonomous job limits and permissions</legend>
      <label className="flex min-h-11 items-center gap-3 text-sm"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />Allow autonomous job admission</label>
      <label className="flex min-h-11 items-center gap-3 text-sm"><input type="checkbox" checked={publish} onChange={(event) => setPublish(event.target.checked)} />Allow verified draft pull-request publication</label>
      <p className="text-sm text-gray-600 dark:text-gray-300">Profile keys are comma-separated. Empty lists permit no profiles. The spend threshold uses reported usage, not a guaranteed invoice cap.</p>
      <div className="grid min-w-0 gap-3 sm:grid-cols-2">
        {profileFields.map(({ key, label }) => <label key={key} className="min-w-0 space-y-1 text-sm"><span>{label}</span><input className={inputClass} value={profiles[key]} onChange={(event) => setProfiles((current) => ({ ...current, [key]: event.target.value }))} /></label>)}
        {[...limitFields, ...capacityFields].map(({ key, label }) => <label key={key} className="min-w-0 space-y-1 text-sm"><span>{label}</span><input required inputMode="numeric" pattern="[0-9]+" className={inputClass} value={numbers[key]} onChange={(event) => setNumbers((current) => ({ ...current, [key]: event.target.value }))} /></label>)}
      </div>
      <fieldset className="min-w-0 space-y-3">
        <legend>Recurring autonomous workers</legend>
        {!deployment && <label className="flex min-h-11 items-center gap-3 text-sm"><input type="checkbox" checked={overrideRecurring} onChange={(event) => setOverrideRecurring(event.target.checked)} />Set explicit recurring override (otherwise inherit)</label>}
        <p className="text-sm">Requested recurring permission: {requestedRecurring == null ? "Inherited" : requestedRecurring.enabled ? "Enabled" : "Disabled"}. Effective recurring permission: {document.effective?.policy.recurring?.enabled ? "Enabled" : "Disabled"}.</p>
        <fieldset disabled={!overrideRecurring} className="space-y-3">
          <label className="flex min-h-11 items-center gap-3 text-sm"><input type="checkbox" checked={recurringEnabled} onChange={(event) => setRecurringEnabled(event.target.checked)} />Allow recurring autonomous workers</label>
          {recurringFields.map(({ key, label }) => <label key={key} className="block min-w-0 space-y-1 text-sm"><span>{label}</span><input required={overrideRecurring} inputMode="numeric" pattern="[0-9]+" className={inputClass} value={recurringNumbers[key]} onChange={(event) => setRecurringNumbers((current) => ({ ...current, [key]: event.target.value }))} /></label>)}
        </fieldset>
        <p className="text-sm">Eligible work can spend the allowance again as earlier usage leaves the rolling window until paused or stopped. Reported spend is not an exact invoice cap. Lower levels may reduce ceilings or increase the minimum interval, never widen authority.</p>
      </fieldset>
      {!deployment && <p className="text-sm text-gray-600 dark:text-gray-300">Saving sets explicit bounds. Inherit removes this override so future upstream changes apply directly.</p>}
      <div className="flex flex-wrap gap-3">
        <button type="submit" className="min-h-11 rounded bg-blue-600 px-4 py-2 text-sm text-white">{busy ? "Saving…" : "Save autonomous job policy"}</button>
        {!deployment && <button type="button" className="min-h-11 rounded border px-4 py-2 text-sm" onClick={() => void submit(true)}>Inherit autonomous job policy</button>}
      </div>
    </fieldset>
    {error && <p role="alert" className="break-words text-sm text-red-700 dark:text-red-300">{error}</p>}
  </form>;
}
