"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import type { JobDetail, JobLimits, JobTarget, WorkerConversionSource, WorkerDetail, WorkerLimits, WorkerMandateSpec, WorkerPreview, WorkerPreviewRequest, WorkerTargetsResponse } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { buildProjectList } from "@/lib/projectList";
import { jobFetch, jobPath } from "@/lib/jobs";
import { readJobRecord } from "@/lib/jobCache";
import { workerFetch, workerHref, workerPath } from "@/lib/workers";
import { useWorkerDraft, useWorkerMutation, useWorkerOnline } from "@/lib/useWorkers";
import { onWorkerCachePurge, readWorkerRecord, workerCacheRevision, writeWorkerRecord } from "@/lib/workerCache";
import type { WorkerScope } from "@/lib/workerCache";
import { JobError, limitLabels } from "../jobs/JobElements";

export interface JobProjectContext { id: string; name: string; machineId?: string | null }
export interface WorkerComposerProps {
  project?: JobProjectContext;
  onApproved?: (workerId: string, projectId: string) => void;
  initialProject?: string | null;
  worker?: WorkerDetail;
  source?: WorkerConversionSource;
}
interface ComposerDraft {
  spec: Omit<WorkerMandateSpec, "limits"> & { limits?: WorkerLimits };
  preview?: WorkerPreview;
  pristine?: boolean;
}
const emptyDraft: ComposerDraft = { pristine: true, spec: {
  title: "", description: "", checklist: [], machine_id: "", worker_profile: "", model_profile: "",
  verification_profile: "", base_ref: "", interval_seconds: 0,
} };
const recurringLabels: Record<Exclude<keyof WorkerLimits, "per_run">, string> = {
  elapsed_seconds_24h: "Rolling 24-hour elapsed allowance (seconds)",
  spend_micro_usd_24h: "Rolling 24-hour reported spend allowance (micro-USD)",
  runs_24h: "Runs per rolling 24 hours", attempts_per_item: "Maximum attempts per work item",
};
export function eligibleWorkerTarget(target: JobTarget) {
  const policy = target.policy.policy;
  return target.available && target.allowed_actions.admit && policy.enabled && policy.recurring?.enabled === true
    && target.profiles.some((profile) => profile.ready && policy.worker_profiles.includes(profile.key)
      && profile.model_profiles.some((model) => policy.model_profiles.includes(model.key))
      && profile.verification_profiles.some((verification) => policy.verification_profiles.includes(verification.key)));
}

export function WorkerComposer({ project, initialProject, onApproved, worker, source }: WorkerComposerProps) {
  const account = useStore((state) => state.userId);
  const sessions = useStore((state) => state.sessions);
  const clients = useStore((state) => state.cliClients);
  const machines = useStore((state) => state.machines);
  const projects = useMemo(() => buildProjectList(sessions, clients, machines), [sessions, clients, machines]);
  const [selectedProject, setProject] = useState(initialProject ?? "");
  const projectId = worker?.worker.project_id ?? project?.id ?? selectedProject;
  const sourceKey = source?.kind === "job" ? `job:${source.job_id}` : "new";
  return <>
    <h1 tabIndex={-1}>{worker ? "Revise and reapprove worker" : "New autonomous worker"}</h1>
    <p>Approve an ongoing responsibility. The worker discovers and prioritizes concrete work within your description and standing checklist; editing this draft does not start execution.</p>
    {worker || project ? <p>Project: <strong>{project?.name ?? projectId}</strong></p> : <label>Project<select value={projectId} onChange={(event) => setProject(event.target.value)}>
      <option value="">Choose a project</option>
      {projectId && !projects.some((item) => item.projectId === projectId) && <option value={projectId}>{projectId}</option>}
      {projects.map((item) => <option key={item.projectId} value={item.projectId}>{item.name} · {item.hostname || item.projectId}</option>)}
    </select></label>}
    {!project && !worker && !projects.length && <p>Waiting for authorized project inventory. Workers do not require a running interactive session.</p>}
    {account && projectId && <ProjectComposer key={`${account}:${projectId}:${worker?.worker.id ?? sourceKey}`} account={account} projectId={projectId} preferredMachine={project?.machineId} worker={worker} source={source} sourceKey={sourceKey} onApproved={onApproved} />}
  </>;
}

function ProjectComposer({ account, projectId, preferredMachine, worker, source, sourceKey, onApproved }: {
  account: string; projectId: string; preferredMachine?: string | null; worker?: WorkerDetail;
  source?: WorkerConversionSource; sourceKey: string; onApproved?: WorkerComposerProps["onApproved"];
}) {
  const workerId = worker?.worker.id ?? "";
  const scope = useMemo<WorkerScope>(() => [account, projectId, workerId], [account, projectId, workerId]);
  const draftKey = worker ? `mandate:${worker.mandate.revision}` : `composer:${sourceKey}`;
  const initial = useMemo<ComposerDraft>(() => worker ? { spec: worker.mandate.spec, pristine: false } : emptyDraft, [worker]);
  const draft = useWorkerDraft(scope, draftKey, initial);
  const path = worker ? `${workerPath(projectId, worker.worker.id)}/mandate` : workerPath(projectId);
  const mutation = useWorkerMutation(scope, path, draft.value.preview?.id);
  const online = useWorkerOnline();
  const router = useRouter();
  const [targets, setTargets] = useState<WorkerTargetsResponse>();
  const [fresh, setFresh] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [purged, setPurged] = useState(false);
  const [legacyText, setLegacyText] = useState<{ description: string; checklist: string[]; origin: string }>();
  const lifetime = useRef({ active: true, revision: workerCacheRevision() });
  const handledReceipt = useRef("");
  const approving = useRef(false);
  const onOpen = useRef(onApproved);
  const preferMachine = useRef<boolean | undefined>(undefined);
  const draftRef = useRef(draft);
  useEffect(() => { draftRef.current = draft; onOpen.current = onApproved; }, [draft, onApproved]);
  useEffect(() => {
    const token = lifetime.current;
    token.active = true;
    const unsubscribe = onWorkerCachePurge(() => {
      token.active = false;
      setPurged(true); setTargets(undefined); setLegacyText(undefined); setFresh(false); setError("");
    });
    return () => { token.active = false; unsubscribe(); };
  }, []);
  const current = useCallback(() => lifetime.current.active && lifetime.current.revision === workerCacheRevision() && useStore.getState().userId === account, [account]);
  const value = draft.value;
  const spec = value.spec;
  useEffect(() => {
    let active = true;
    const revision = workerCacheRevision();
    setFresh(false);
    const load = async () => {
      const cached = await readWorkerRecord<WorkerTargetsResponse>([account, projectId, ""], "targets");
      if (!active || !current() || revision !== workerCacheRevision()) return;
      if (cached) setTargets(cached);
      if (!online) return;
      const response = await workerFetch<WorkerTargetsResponse>(`/projects/${encodeURIComponent(projectId)}/worker-targets`, "targets");
      if (!active || !current() || revision !== workerCacheRevision()) return;
      await writeWorkerRecord([account, projectId, ""], "targets", response, "", revision);
      if (active && current()) { setTargets(response); setFresh(true); setError(""); }
    };
    void load().catch((failure: unknown) => { if (active && current()) setError(failure instanceof Error ? failure.message : "Cannot load worker targets."); });
    return () => { active = false; };
    // Scope changes remount this component; cache purge permanently fences this instance.
  }, [account, projectId, online, refresh, current]);
  useEffect(() => {
    if (worker) return;
    let active = true;
    const load = async () => {
      if (source?.kind === "job") {
        if (!online) return;
        const previous = await jobFetch<JobDetail>(jobPath(projectId, source.job_id), "detail");
        if (active && current()) setLegacyText({ description: previous.task.instruction, checklist: previous.task.acceptance_criteria, origin: `Earlier job ${source.job_id}; original model profile ${previous.task.model_profile}` });
      } else {
        const previous = await readJobRecord<unknown>([account, projectId, ""], "draft", "composer");
        if (!active || !current() || !previous || typeof previous !== "object") return;
        const saved = previous as Record<string, unknown>;
        if (typeof saved.instruction === "string" && saved.instruction.trim()) setLegacyText({ description: saved.instruction, checklist: typeof saved.acceptance === "string" ? saved.acceptance.split("\n") : [], origin: "Saved finite-job draft (not approved)" });
      }
    };
    void load().catch((failure: unknown) => { if (active && current()) setError(failure instanceof Error ? failure.message : "Cannot inspect conversion source."); });
    return () => { active = false; };
  }, [account, projectId, source, online, worker, current]);
  const receipt = mutation.request?.receipt;
  const approved = receipt?.worker_id && (receipt.status === "accepted" || receipt.status === "completed") ? receipt.worker_id : undefined;
  useEffect(() => {
    const requestId = mutation.request?.id;
    if (!approved || !requestId || !draft.loaded || handledReceipt.current === requestId || !current()) return;
    handledReceipt.current = requestId;
    const finish = async () => {
      const revision = workerCacheRevision();
      await draftRef.current.flush();
      if (!current()) return;
      await writeWorkerRecord(scope, "draft", emptyDraft, draftKey, revision);
      if (!current()) return;
      if (onOpen.current) onOpen.current(approved, projectId);
      else router.replace(workerHref(projectId, approved));
    };
    void finish().catch(() => { if (current()) setError("Approval is durable, but the draft could not be retired. Open the acknowledged worker below; do not approve again."); });
  }, [approved, mutation.request?.id, draft.loaded, projectId, router, current, scope, draftKey]);
  const preferredTarget = targets?.targets.find((target) => target.machine_id === preferredMachine);
  useEffect(() => {
    if (!draft.loaded) return;
    preferMachine.current ??= value.pristine === true && !source && !worker;
    if (!preferMachine.current || spec.machine_id || !preferredTarget || !eligibleWorkerTarget(preferredTarget) || !fresh || !online || mutation.unresolved || approved || !current()) return;
    preferMachine.current = false;
    draft.update({ ...value, pristine: false, spec: { ...spec, machine_id: preferredTarget.machine_id } });
  }, [draft, value, spec, preferredTarget, fresh, online, mutation.unresolved, approved, source, worker, current]);
  const target = targets?.targets.find((item) => item.machine_id === spec.machine_id);
  const profile = target?.profiles.find((item) => item.key === spec.worker_profile);
  const policy = target?.policy.policy;
  const profileAllowed = Boolean(profile?.ready && policy?.worker_profiles.includes(spec.worker_profile));
  const modelAllowed = Boolean(policy?.model_profiles.includes(spec.model_profile) && profile?.model_profiles.some((item) => item.key === spec.model_profile));
  const verificationAllowed = Boolean(policy?.verification_profiles.includes(spec.verification_profile) && profile?.verification_profiles.some((item) => item.key === spec.verification_profile));
  const eligible = fresh && online && current() && Boolean(target && eligibleWorkerTarget(target) && profileAllowed && modelAllowed && verificationAllowed) && (!worker || worker.allowed_actions.edit_mandate);
  const update = (patch: Partial<ComposerDraft["spec"]>) => { if (current()) draft.update({ ...value, spec: { ...spec, ...patch }, pristine: false, preview: undefined }); };
  const review = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!eligible || !spec.limits || mutation.unresolved || approved || loading) return;
    setLoading(true); setError("");
    try {
      await draft.flush();
      if (!current()) return;
      const input: WorkerPreviewRequest = { worker_id: worker?.worker.id ?? null, expected_version: worker?.worker.state_version ?? null,
        spec: { ...spec, title: spec.title.trim(), description: spec.description.trim(), checklist: spec.checklist.map((line) => line.trim()).filter(Boolean), base_ref: spec.base_ref.trim(), limits: spec.limits }, source: source ?? null };
      const preview = await workerFetch<WorkerPreview>(`/projects/${encodeURIComponent(projectId)}/worker-previews`, "preview", input);
      if (!current()) return;
      draft.update({ ...value, preview, pristine: false });
    } catch (failure) { if (current()) setError(failure instanceof Error ? failure.message : "Mandate preview failed."); }
    finally { if (current()) setLoading(false); }
  };
  const preview = value.preview;
  const previewMatches = preview?.project_id === projectId && preview.worker_id === (worker?.worker.id ?? null)
    && preview.expected_version === (worker?.worker.state_version ?? null) && JSON.stringify(preview.source) === JSON.stringify(source ?? null);
  const previewRejected = receipt?.status === "rejected" && mutation.request && "preview_id" in mutation.request.body && mutation.request.body.preview_id === preview?.id;
  const locked = loading || mutation.busy || mutation.unresolved || !draft.loaded || Boolean(approved);
  const approve = async () => {
    if (approving.current || !preview || !previewMatches || !eligible || locked || previewRejected || Date.parse(preview.expires_at) <= Date.now()) return;
    approving.current = true;
    setLoading(true);
    try {
      await draft.flush();
      if (!current()) return;
      const id = crypto.randomUUID();
      await mutation.submit({ id, scope, path, createdAt: new Date().toISOString(), body: { request_id: id, preview_id: preview.id, fingerprint: preview.fingerprint } });
    } catch (failure) { if (current()) setError(failure instanceof Error ? failure.message : "Draft could not be saved; nothing was submitted."); }
    finally { approving.current = false; if (current()) setLoading(false); }
  };
  if (purged || !current()) return <p className="job-notice">Access changed. Reopen the project to authorize this draft again.</p>;
  return <>
    <JobError message={error || draft.error || mutation.error} />
    {!online && <p className="job-notice">Offline or reconnecting — drafts are editable; approval is disabled.</p>}
    {source && <p className="job-notice">This is an unapproved conversion from an earlier finite job. Original history remains unchanged. Choose the qualified OMP and model profiles explicitly; no provider, recurring permission or budget is inherited or substituted.</p>}
    {legacyText && <section className="job-card"><h2>Reusable earlier instructions</h2><p>{legacyText.origin}</p><p style={{ whiteSpace: "pre-wrap" }}>{legacyText.description}</p><button type="button" disabled={locked} onClick={() => update({ description: legacyText.description, checklist: legacyText.checklist })}>Copy text into unapproved mandate</button><p>No prior target, preview, approval, cadence or budget is imported. Review every recurring setting afresh.</p></section>}
    {targets?.unavailable_reasons.map((reason) => <p className="job-notice" key={reason}>{reason}</p>)}
    {preferredMachine && !spec.machine_id && <p className="job-notice">Project host: {preferredTarget?.hostname ?? preferredMachine}. {preferredTarget && eligibleWorkerTarget(preferredTarget) ? "Only a pristine new draft may use this eligible host as a hint." : "This host is not currently eligible. Ask its operator to check authority, recurring policy and runtime readiness."} No other machine is selected automatically.</p>}
    {fresh && !targets?.targets.some(eligibleWorkerTarget) && <p className="job-notice">No eligible recurring execution target. Ask the hosting operator to review permission, recurring policy and qualified OMP readiness, then refresh.</p>}
    <button type="button" disabled={!online || loading} onClick={() => setRefresh((count) => count + 1)}>Refresh target readiness</button>
    {mutation.request && <section className="job-card" aria-label="Approval receipt"><p>Request <code>{mutation.request.id}</code>: {receipt?.status ?? "unconfirmed"}{receipt?.error_code ? ` — ${receipt.error_code}` : ""}</p>
      {approved && <button type="button" onClick={() => onApproved ? onApproved(approved, projectId) : router.replace(workerHref(projectId, approved))}>Open acknowledged worker</button>}
      {!receipt || receipt.status === "pending" ? <button type="button" disabled={!online || mutation.busy} onClick={() => void mutation.resolve()}>Check original receipt</button> : null}
      {mutation.retryAllowed && <button type="button" disabled={!eligible || mutation.busy} onClick={() => { if (mutation.request) void mutation.submit(mutation.request); }}>Retry original approval with same request ID</button>}
    </section>}
    <form onSubmit={(event) => void review(event)}><fieldset disabled={locked}><legend>Responsibility and execution envelope</legend>
      <label>Worker title<input required maxLength={200} value={spec.title} onChange={(event) => update({ title: event.target.value })} /></label>
      <label>Job description<textarea required rows={6} maxLength={16384} value={spec.description} onChange={(event) => update({ description: event.target.value })} /></label>
      <label>Standing checklist (one item per line)<textarea required rows={5} value={spec.checklist.join("\n")} onChange={(event) => update({ checklist: event.target.value.split("\n") })} /></label>
      <label>Repository base branch or committed ref<input required maxLength={256} value={spec.base_ref} placeholder="For example: main" onChange={(event) => update({ base_ref: event.target.value })} /></label>
      <label>Execution machine<select required value={spec.machine_id} onChange={(event) => { preferMachine.current = false; update({ machine_id: event.target.value, worker_profile: "", model_profile: "", verification_profile: "", limits: undefined, interval_seconds: 0 }); }}><option value="">Choose a target</option>
        {spec.machine_id && !target && <option value={spec.machine_id}>{spec.machine_id} — unavailable</option>}{targets?.targets.map((item) => <option key={item.machine_id} value={item.machine_id}>{item.hostname}{!eligibleWorkerTarget(item) ? " — unavailable" : ""}</option>)}
      </select></label>
      {spec.machine_id && !target && <p className="job-notice">The saved execution machine is unavailable or not currently authorized. Its selection is preserved; no replacement is selected automatically.</p>}
      {target && <div className="job-notice"><p>Host owner: <code>{target.cluster_owner_user_id}</code></p>
        {!target.allowed_actions.admit && <p>Your account lacks unattended admission authority for this exact host.</p>}
        {!policy?.enabled && <p>Autonomous execution is disabled by effective policy.</p>}
        {!policy?.recurring?.enabled && <p>Recurring workers are disabled by effective policy independently of finite jobs. Ask the policy administrator to review recurring permission.</p>}
        {!target.available && <p>Runtime readiness is unavailable. Ask the hosting operator to qualify the exact OMP worker host; no unrestricted provider fallback exists.</p>}
        {target.unavailable_reasons.map((reason) => <p key={reason}>{reason}</p>)}
        <details><summary>Effective policy and inherited sources</summary><p>Digest: <code>{target.policy.digest}</code></p>{target.policy.sources.map((item) => <p key={`${item.scope}:${item.id}`}>{item.scope} {item.id} · revision {item.revision}</p>)}<pre>{JSON.stringify(policy, null, 2)}</pre></details>
      </div>}
      <label>OMP worker profile<select required value={spec.worker_profile} onChange={(event) => {
        const selected = target?.profiles.find((item) => item.key === event.target.value);
        let limits: WorkerLimits | undefined;
        if (selected && policy?.recurring) {
          const per_run = { ...selected.limits };
          for (const key of Object.keys(limitLabels) as (keyof JobLimits)[]) per_run[key] = Math.min(per_run[key], policy.limits[key]);
          per_run.attempts = 1;
          per_run.elapsed_seconds = Math.min(per_run.elapsed_seconds, policy.recurring.max_elapsed_seconds_24h);
          per_run.spend_micro_usd = Math.min(per_run.spend_micro_usd, policy.recurring.max_spend_micro_usd_24h);
          limits = { per_run, elapsed_seconds_24h: policy.recurring.max_elapsed_seconds_24h, spend_micro_usd_24h: policy.recurring.max_spend_micro_usd_24h, runs_24h: policy.recurring.max_runs_24h, attempts_per_item: 1 };
        }
        update({ worker_profile: event.target.value, model_profile: "", verification_profile: "", limits, interval_seconds: policy?.recurring?.min_interval_seconds ?? 0 });
      }}><option value="">Choose a qualified OMP worker</option>{spec.worker_profile && !profile && <option value={spec.worker_profile}>{spec.worker_profile} — unavailable</option>}{target?.profiles.map((item) => <option key={item.key} value={item.key}>{item.label}{!item.ready || !policy?.worker_profiles.includes(item.key) ? " — unavailable" : ""}</option>)}</select></label>
      {profile?.unavailable_reasons.map((reason) => <p className="job-notice" key={reason}>{reason}</p>)}
      {spec.worker_profile && !profileAllowed && <p className="job-notice">The saved worker profile is not ready or permitted. Its selection is preserved.</p>}
      <label>Model profile<select required value={spec.model_profile} onChange={(event) => update({ model_profile: event.target.value })}><option value="">Choose a model explicitly</option>{spec.model_profile && !modelAllowed && <option value={spec.model_profile}>{spec.model_profile} — unavailable</option>}{profile?.model_profiles.filter((item) => policy?.model_profiles.includes(item.key)).map((item) => <option key={item.key} value={item.key}>{item.label} · {item.model}</option>)}</select></label>
      <label>Independent verification profile<select required value={spec.verification_profile} onChange={(event) => update({ verification_profile: event.target.value })}><option value="">Choose independent checks</option>{spec.verification_profile && !verificationAllowed && <option value={spec.verification_profile}>{spec.verification_profile} — unavailable</option>}{profile?.verification_profiles.filter((item) => policy?.verification_profiles.includes(item.key)).map((item) => <option key={item.key} value={item.key}>{item.label}</option>)}</select></label>
      {(spec.model_profile && !modelAllowed || spec.verification_profile && !verificationAllowed) && <p className="job-notice">A saved model or verification profile is unavailable under current policy. Choose explicitly; no provider is substituted.</p>}
      <label>Inspection interval (seconds)<input required type="number" min={policy?.recurring?.min_interval_seconds ?? 1} step={1} value={spec.interval_seconds || ""} onChange={(event) => update({ interval_seconds: event.target.valueAsNumber })} /></label>
      {spec.limits && <><fieldset><legend>Per-run ceilings</legend><p>Every inspection and implementation is a separate bounded run with one attempt. All delegated work and question waiting count.</p>{(Object.keys(limitLabels) as (keyof JobLimits)[]).map((key) => <label key={key}>{limitLabels[key]}<input type="number" required min={1} step={1} readOnly={key === "attempts"} max={key === "attempts" ? 1 : profile && policy ? Math.min(profile.limits[key], policy.limits[key]) : undefined} value={spec.limits!.per_run[key]} onChange={(event) => update({ limits: { ...spec.limits!, per_run: { ...spec.limits!.per_run, [key]: event.target.valueAsNumber } } })} /></label>)}</fieldset>
        <fieldset><legend>Recurring allowance and item attempts</legend>{(Object.keys(recurringLabels) as (keyof typeof recurringLabels)[]).map((key) => <label key={key}>{recurringLabels[key]}<input type="number" required min={key === "elapsed_seconds_24h" ? spec.limits!.per_run.elapsed_seconds : key === "spend_micro_usd_24h" ? spec.limits!.per_run.spend_micro_usd : 1} max={key === "attempts_per_item" ? policy?.limits.attempts : key === "runs_24h" ? policy?.recurring?.max_runs_24h : key === "elapsed_seconds_24h" ? policy?.recurring?.max_elapsed_seconds_24h : policy?.recurring?.max_spend_micro_usd_24h} step={1} value={spec.limits![key]} onChange={(event) => update({ limits: { ...spec.limits!, [key]: event.target.valueAsNumber } })} /></label>)}<p>1 USD = 1,000,000 micro-USD. Reported-spend thresholds are not exact billing caps; in-flight requests may overrun them.</p></fieldset></>}
      <button type="submit" disabled={!eligible || !spec.limits || Boolean(draft.error)}>{loading ? "Resolving reviewed mandate…" : "Review exact recurring mandate"}</button>
    </fieldset></form>
    {preview && <section className="job-card" aria-label="Reviewed worker mandate"><h2>Confirm this exact recurring responsibility</h2><h3>{preview.mandate.spec.title}</h3><p style={{ whiteSpace: "pre-wrap" }}>{preview.mandate.spec.description}</p><ol>{preview.mandate.spec.checklist.map((item, index) => <li key={index}>{item}</li>)}</ol>
      <dl className="job-facts"><div><dt>Repository / approved base</dt><dd>{preview.mandate.repository} · {preview.mandate.spec.base_ref}</dd></div><div><dt>Current committed tip</dt><dd><code>{preview.base_sha}</code></dd></div><div><dt>Machine / owner</dt><dd>{preview.hostname} · {preview.mandate.spec.machine_id} · {preview.cluster_owner_user_id}</dd></div><div><dt>OMP / model / verification profiles</dt><dd>{preview.mandate.spec.worker_profile} / {preview.mandate.spec.model_profile} / {preview.mandate.spec.verification_profile}</dd></div><div><dt>Inspection interval</dt><dd>{preview.mandate.spec.interval_seconds} seconds</dd></div></dl>
      <h3>Every approved ceiling</h3>{(Object.keys(limitLabels) as (keyof JobLimits)[]).map((key) => <p key={key}>Per run — {limitLabels[key]}: {preview.mandate.spec.limits.per_run[key]}</p>)}{(Object.keys(recurringLabels) as (keyof typeof recurringLabels)[]).map((key) => <p key={key}>{recurringLabels[key]}: {preview.mandate.spec.limits[key]}</p>)}
      <details><summary>Pinned profile identities and policy</summary><p>Worker <code>{preview.mandate.worker_profile_digest}</code>; verifier <code>{preview.mandate.verification_profile_digest}</code>; policy <code>{preview.mandate.policy_digest}</code></p>{preview.policy.sources.map((item) => <p key={`${item.scope}:${item.id}`}>{item.scope} {item.id} · revision {item.revision}</p>)}<p>Draft publication: {preview.policy.policy.publish_enabled ? "permitted after independent verification" : "not permitted"}.</p></details>
      <p>Approval delegates autonomous discovery and work selection within this mandate. Future executions use the then-current committed tip of this same approved base branch, not uncommitted source changes; each run retains its exact SHA.</p>
      <p>The rolling 24-hour allowance renews automatically as earlier usage leaves the window. Eligible work can spend it again without another approval until paused or stopped. Reported spend is not an exact invoice cap.</p>
      <p>A delivered draft waits for human review; no automatic merge or stacked changes. Pause after this run fences future work while the current bounded run settles. Stop now also requests immediate termination and blocks publication; stopping remains unresolved until positive stop evidence. Neither resets consumption.</p>
      {worker && <p>Replacement approval uses worker version {preview.expected_version}. New work is fenced; this mandate takes effect only after earlier execution is positively stopped or finished. Existing backlog requires reassessment; history and usage are retained.</p>}
      <p>Expires <time dateTime={preview.expires_at}>{new Date(preview.expires_at).toLocaleString()}</time>.</p>
      {!previewMatches && <p className="job-notice">This preview is stale or belongs to different source/version inputs. Review again before approval.</p>}
      <button type="button" disabled={!eligible || locked || !previewMatches || Boolean(previewRejected) || Date.parse(preview.expires_at) <= Date.now()} onClick={() => void approve()}>{worker ? "Approve replacement mandate" : "Approve recurring worker"}</button>
    </section>}
  </>;
}
