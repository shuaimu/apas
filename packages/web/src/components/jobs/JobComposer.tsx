"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import type { JobLimits, JobPreview, JobPreviewRequest, JobTarget, JobTargetsResponse } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { buildProjectList } from "@/lib/projectList";
import { jobFetch, jobHref, jobPath } from "@/lib/jobs";
import { useJobDraft, useJobMutation, useJobOnline } from "@/lib/useJobs";
import { jobCacheRevision, readJobRecord, writeJobRecord } from "@/lib/jobCache";
import type { JobScope } from "@/lib/jobCache";
import { JobError, limitLabels, RequestStatus } from "./JobElements";

interface ComposerDraft {
  instruction: string; acceptance: string; source: string; base: string;
  machine: string; worker: string; model: string; verification: string;
  limits?: JobLimits; preview?: JobPreview;
  pristine?: boolean;
}
const emptyDraft: ComposerDraft = { instruction: "", acceptance: "", source: "", base: "", machine: "", worker: "", model: "", verification: "", pristine: true };
export interface JobProjectContext {
  id: string;
  name: string;
  machineId?: string | null;
}
export interface JobComposerProps {
  initialProject?: string | null;
  projectContext?: JobProjectContext;
  onOpenJob?: (jobId: string) => void;
}
function canUseTarget(target: JobTarget) {
  const policy = target.policy.policy;
  return target.available && target.allowed_actions.admit && policy.enabled
    && target.profiles.some((profile) => profile.ready
      && profile.model_profiles.some((model) => policy.model_profiles.includes(model.key))
      && profile.verification_profiles.some((verification) => policy.verification_profiles.includes(verification.key)));
}
export function JobComposer({ initialProject, projectContext, onOpenJob }: JobComposerProps) {
  const account = useStore((state) => state.userId)!;
  const sessions = useStore((state) => state.sessions);
  const clients = useStore((state) => state.cliClients);
  const machines = useStore((state) => state.machines);
  const projects = useMemo(() => buildProjectList(sessions, clients, machines), [sessions, clients, machines]);
  const [selectedProject, setProject] = useState(initialProject ?? "");
  const project = projectContext?.id ?? selectedProject;
  const router = useRouter();
  return <>
    <h1 tabIndex={-1}>New autonomous job</h1>
    <p>A job runs independently of your interactive sessions. Review the exact base, target and finite allowance before starting.</p>
    {projectContext ? <p>Project: <strong>{projectContext.name}</strong></p> : <label>Project<select value={project} onChange={(event) => { setProject(event.target.value); router.replace(`/code/jobs/new?project=${encodeURIComponent(event.target.value)}`); }}>
      <option value="">Choose a project</option>
      {project && !projects.some((item) => item.projectId === project) && <option value={project}>{project}</option>}
      {projects.map((item) => <option key={item.projectId} value={item.projectId}>{item.name} · {item.hostname || item.projectId}</option>)}
    </select></label>}
    {!projectContext && !projects.length && <p>Waiting for authorized project inventory. Jobs do not require a running session; stopped projects appear when inventory arrives.</p>}
    {project && <ProjectComposer key={`${account}:${project}`} account={account} project={project} preferredMachine={projectContext?.machineId} onOpenJob={onOpenJob} />}
  </>;
}
function ProjectComposer({ account, project, preferredMachine, onOpenJob }: { account: string; project: string; preferredMachine?: string | null; onOpenJob?: (jobId: string) => void }) {
  const scope: JobScope = [account, project, ""];
  const draft = useJobDraft(scope, "composer", emptyDraft);
  const mutation = useJobMutation(scope, jobPath(project), draft.value.preview?.id);
  const online = useJobOnline();
  const router = useRouter();
  const [targets, setTargets] = useState<JobTargetsResponse>();
  const [fresh, setFresh] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [refresh, setRefresh] = useState(0);
  const openJob = useRef(onOpenJob);
  const handledReceipt = useRef("");
  const mounted = useRef(false);
  const preferContextMachine = useRef<boolean | undefined>(undefined);
  useEffect(() => { openJob.current = onOpenJob; }, [onOpenJob]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const value = draft.value;
  useEffect(() => {
    let active = true;
    setFresh(false);
    const load = async () => {
      const scope: JobScope = [account, project, ""];
      const revision = jobCacheRevision();
      const cached = await readJobRecord<JobTargetsResponse>(scope, "targets");
      if (active && cached) setTargets(cached);
      if (!online) return;
      const response = await jobFetch<JobTargetsResponse>(`/projects/${encodeURIComponent(project)}/job-targets`, "targets");
      await writeJobRecord(scope, "targets", response, "", revision);
      if (active) { setTargets(response); setFresh(true); setError(""); }
    };
    void load().catch((failure: unknown) => { if (active) setError(failure instanceof Error ? failure.message : "Targets could not be loaded."); });
    return () => { active = false; };
  }, [account, project, online, refresh]);
  const receipt = mutation.request?.receipt;
  const admittedJob = receipt?.job_id && (receipt.status === "accepted" || receipt.status === "completed") ? receipt.job_id : undefined;
  const requestId = mutation.request?.id;
  useEffect(() => {
    if (!admittedJob || !requestId || !draft.loaded || handledReceipt.current === requestId) return;
    handledReceipt.current = requestId;
    const revision = jobCacheRevision();
    const finish = async () => {
      await draft.flush();
      await writeJobRecord([account, project, ""], "draft", emptyDraft, "composer", revision);
      if (!mounted.current || revision !== jobCacheRevision()) return;
      if (openJob.current) openJob.current(admittedJob);
      else router.replace(jobHref(project, admittedJob));
    };
    void finish().catch(() => {
      if (mounted.current) setError("The job was accepted, but the composer draft could not be cleared. Open the original job below; do not start it again.");
    });
  }, [admittedJob, requestId, draft, account, project, router]);
  const preferredTarget = targets?.targets.find((item) => item.machine_id === preferredMachine);
  useEffect(() => {
    if (!draft.loaded) return;
    // Only a new or admission-cleared draft permits a contextual default.
    // Legacy saved drafts without this marker preserve all existing choices.
    preferContextMachine.current ??= value.pristine === true;
    if (!preferContextMachine.current || value.machine || !preferredTarget || !fresh || !online || mutation.unresolved || admittedJob) return;
    if (!canUseTarget(preferredTarget)) return;
    preferContextMachine.current = false;
    draft.update({ ...value, machine: preferredTarget.machine_id, pristine: false });
  }, [draft, value, preferredTarget, fresh, online, mutation.unresolved, admittedJob]);
  const target = targets?.targets.find((item) => item.machine_id === value.machine);
  const worker = target?.profiles.find((item) => item.key === value.worker);
  const modelAllowed = Boolean(target?.policy.policy.model_profiles.includes(value.model) && worker?.model_profiles.some((profile) => profile.key === value.model));
  const verificationAllowed = Boolean(target?.policy.policy.verification_profiles.includes(value.verification) && worker?.verification_profiles.some((profile) => profile.key === value.verification));
  const eligible = fresh && online && target?.available && target.allowed_actions.admit && worker?.ready && target.policy.policy.enabled && modelAllowed && verificationAllowed;
  const update = (patch: Partial<ComposerDraft>) => draft.update({ ...value, ...patch, preview: undefined, pristine: false });
  const review = async (event: React.FormEvent) => {
    event.preventDefault(); setLoading(true); setError("");
    try {
      await draft.flush();
      if (admittedJob || !value.limits || !eligible) throw new Error("Choose a currently eligible target and worker profile.");
      const input: JobPreviewRequest = {
        instruction: value.instruction.trim(), acceptance_criteria: value.acceptance.split("\n").map((line) => line.trim()).filter(Boolean),
        source_url: value.source.trim() || null, base_ref: value.base.trim(), machine_id: value.machine,
        worker_profile: value.worker, model_profile: value.model, verification_profile: value.verification, limits: value.limits,
      };
      const preview = await jobFetch<JobPreview>(`/projects/${encodeURIComponent(project)}/job-previews`, "preview", input);
      const next = { ...value, preview };
      await writeJobRecord(scope, "draft", next, "composer");
      draft.update(next);
    } catch (failure) { setError(failure instanceof Error ? failure.message : "Admission review failed."); }
    finally { setLoading(false); }
  };
  const previewRejected = mutation.request?.receipt?.status === "rejected" && "preview_id" in mutation.request.body && mutation.request.body.preview_id === value.preview?.id;
  const start = async () => {
    if (admittedJob || !value.preview || !eligible || mutation.unresolved || previewRejected) return;
    setError("");
    try {
      await draft.flush();
      const id = crypto.randomUUID();
      await mutation.submit({ id, scope, path: jobPath(project), createdAt: new Date().toISOString(), body: { request_id: id, preview_id: value.preview.id, fingerprint: value.preview.fingerprint } });
    } catch (failure) { setError(failure instanceof Error ? failure.message : "Draft could not be saved. Nothing was submitted."); }
  };
  const locked = loading || mutation.busy || mutation.unresolved || !draft.loaded || Boolean(admittedJob);
  return <>
    <JobError message={error || draft.error || mutation.error} />
    {!online && <p className="job-notice">Offline or reconnecting — drafts are editable; admission is disabled.</p>}
    {targets?.unavailable_reasons.map((reason) => <p className="job-notice" key={reason}>{reason}</p>)}
    {preferredMachine && !value.machine && <div className="job-notice">
      <p>Project host: <code>{preferredTarget?.hostname ?? preferredMachine}</code>. No other machine is selected automatically.</p>
      {!fresh || !online ? <p>Reconnect and refresh target readiness to check this host&apos;s current policy and your admission authority.</p>
        : !preferredTarget ? <p>This project host is not in the currently authorized execution targets. Ask the hosting operator to check project inventory and access, or explicitly choose another qualified target.</p>
        : !canUseTarget(preferredTarget) ? <>
          {!preferredTarget.available && <p>The project host is currently unavailable for autonomous execution.</p>}
          {!preferredTarget.allowed_actions.admit && <p>Your account cannot admit unattended work on the project host.</p>}
          {!preferredTarget.policy.policy.enabled && <p>Autonomous jobs are disabled by the project host&apos;s effective policy. Ask the hosting operator to review policy and readiness.</p>}
          <p>A ready worker with both a policy-approved model and independent verification profile is required. Refresh after the host is qualified, or explicitly choose another qualified target.</p>
          {preferredTarget.unavailable_reasons.map((reason) => <p key={reason}>{reason}</p>)}
          {preferredTarget.profiles.flatMap((profile) => profile.unavailable_reasons.map((reason) => <p key={`${profile.key}:${reason}`}>{profile.label}: {reason}</p>))}
        </> : <p>This host is eligible. Saved draft choices are preserved; choose the execution target explicitly if none is selected.</p>}
    </div>}
    {fresh && !targets?.targets.some(canUseTarget) && <p className="job-notice">No currently authorized target has a ready worker with policy-approved model and verification profiles. Ask the hosting operator to review readiness and effective policy, then refresh. Admission remains disabled.</p>}
    <button type="button" disabled={!online || loading} onClick={() => setRefresh((count) => count + 1)}>Refresh target readiness</button>
    <RequestStatus {...mutation} onOpenJob={onOpenJob} canResolve={online} canRetry={Boolean(eligible)} onResolve={mutation.resolve} onRetry={async () => { if (mutation.request) await mutation.submit(mutation.request); }} />
    <form onSubmit={(event) => void review(event)}>
      <fieldset disabled={locked}>
        <legend>Task and execution target</legend>
        <label>Instruction<textarea required rows={6} maxLength={65536} value={value.instruction} onChange={(event) => update({ instruction: event.target.value })} /></label>
        <label>Acceptance criteria (one per line)<textarea required rows={4} value={value.acceptance} onChange={(event) => update({ acceptance: event.target.value })} /></label>
        <label>Source issue URL (optional)<input type="url" value={value.source} onChange={(event) => update({ source: event.target.value })} /></label>
        <label>Committed base ref<input required placeholder="For example: main or a commit SHA" value={value.base} onChange={(event) => update({ base: event.target.value })} /></label>
        <label>Execution machine<select required value={value.machine} onChange={(event) => { preferContextMachine.current = false; update({ machine: event.target.value, worker: "", model: "", verification: "", limits: undefined }); }}>
          <option value="">Choose a target</option>
          {value.machine && !target && <option value={value.machine}>{value.machine} — unavailable</option>}
          {targets?.targets.map((item) => <option key={item.machine_id} value={item.machine_id}>{item.hostname}{!canUseTarget(item) ? " — unavailable" : ""}</option>)}
        </select></label>
        {value.machine && !target && <p className="job-notice">{fresh ? "The saved execution machine is not currently authorized or available for this project. Its selection has been preserved; refresh readiness or explicitly choose a qualified target." : "Checking the saved execution machine. Its selection will not be replaced automatically."}</p>}
        {target && <div className="job-notice"><p>Host owner: <code>{target.cluster_owner_user_id}</code></p>
          {!target.available && <p>This execution host is currently unavailable. Refresh readiness after the hosting operator resolves its reported reasons.</p>}
          {!target.allowed_actions.admit && <p>Your current account cannot admit unattended work on this target.</p>}
          {!target.policy.policy.enabled && <p>Autonomous jobs are disabled by effective policy. Ask the hosting operator to review policy and readiness.</p>}
          {!canUseTarget(target) && <p>This host needs a ready worker with both a policy-approved model and independent verification profile before admission is available.</p>}
          {target.unavailable_reasons.map((reason) => <p key={reason}>{reason}</p>)}
          <details><summary>Effective policy and inherited sources</summary><p>Policy revision digest: <code>{target.policy.digest}</code></p>{target.policy.sources.map((source) => <p key={`${source.scope}:${source.id}`}>{source.scope} {source.id} · revision {source.revision}</p>)}</details>
        </div>}
        <label>Worker profile<select required value={value.worker} onChange={(event) => {
          const selected = target?.profiles.find((profile) => profile.key === event.target.value);
          const limits = selected && target ? { ...selected.limits } : undefined;
          if (limits && target) {
            for (const key of Object.keys(limits) as (keyof JobLimits)[]) {
              limits[key] = Math.min(limits[key], target.policy.policy.limits[key]);
            }
          }
          update({ worker: event.target.value, model: "", verification: "", limits });
        }}><option value="">Choose a qualified worker</option>{value.worker && !worker && <option value={value.worker}>{value.worker} — unavailable</option>}{target?.profiles.map((item) => <option key={item.key} value={item.key}>{item.label}{!item.ready ? " — unavailable" : ""}</option>)}</select></label>
        {worker?.unavailable_reasons.map((reason) => <p className="job-notice" key={reason}>{reason}</p>)}
        {fresh && value.worker && (!worker || !worker.ready) && <p className="job-notice">The saved worker profile is not currently ready on this target. Its selection is preserved; ask the hosting operator to restore readiness or explicitly choose a qualified worker.</p>}
        <label>Model profile<select required value={value.model} onChange={(event) => update({ model: event.target.value })}><option value="">Choose a model</option>{value.model && !modelAllowed && <option value={value.model}>{value.model} — unavailable</option>}{worker?.model_profiles.filter((item) => target?.policy.policy.model_profiles.includes(item.key)).map((item) => <option key={item.key} value={item.key}>{item.label} · {item.model}</option>)}</select></label>
        <label>Independent verification profile<select required value={value.verification} onChange={(event) => update({ verification: event.target.value })}><option value="">Choose verification checks</option>{value.verification && !verificationAllowed && <option value={value.verification}>{value.verification} — unavailable</option>}{worker?.verification_profiles.filter((item) => target?.policy.policy.verification_profiles.includes(item.key)).map((item) => <option key={item.key} value={item.key}>{item.label}</option>)}</select></label>
        {fresh && (value.model && !modelAllowed || value.verification && !verificationAllowed) && <p className="job-notice">A saved model or verification profile is no longer available under this worker and effective policy. No replacement has been selected. Review policy and explicitly choose currently approved profiles before admission.</p>}
        {value.limits && <fieldset><legend>Finite cumulative allowances</legend><p>1 USD = 1,000,000 micro USD. Reported-spend stopping is not a billing cap: in-flight requests may overrun it.</p>
          {(Object.keys(limitLabels) as (keyof JobLimits)[]).map((key) => <label key={key}>{limitLabels[key]}<input type="number" inputMode="numeric" required min={1} step={1} max={worker && target ? Math.min(worker.limits[key], target.policy.policy.limits[key]) : undefined} value={value.limits![key]} onChange={(event) => update({ limits: { ...value.limits!, [key]: event.target.valueAsNumber } })} /></label>)}
        </fieldset>}
        <button type="submit" disabled={!eligible || Boolean(draft.error)}>{loading ? "Resolving immutable preview…" : "Review exact admission"}</button>
      </fieldset>
    </form>
    {value.preview && <section className="job-card" aria-label="Immutable admission preview">
      <h2>Confirm this exact job</h2>
      <p>{value.preview.task.instruction}</p><ul>{value.preview.task.acceptance_criteria.map((item, index) => <li key={index}>{item}</li>)}</ul>
      <dl className="job-facts"><div><dt>Repository</dt><dd>{value.preview.task.repository}</dd></div><div><dt>Committed base</dt><dd>{value.preview.task.base_ref} · <code>{value.preview.task.base_sha}</code></dd></div><div><dt>Machine / owner</dt><dd>{value.preview.hostname} · {value.preview.machine_id} · {value.preview.cluster_owner_user_id}</dd></div><div><dt>Worker / model / verification</dt><dd>{value.preview.task.worker_profile} / {value.preview.task.model_profile} / {value.preview.task.verification_profile}</dd></div></dl>
      <details><summary>Pinned profile identities and every approved limit</summary><p>Worker <code>{value.preview.task.worker_profile_digest}</code></p><p>Verifier <code>{value.preview.task.verification_profile_digest}</code></p><p>Policy <code>{value.preview.policy_digest}</code></p>{(Object.keys(limitLabels) as (keyof JobLimits)[]).map((key) => <p key={key}>{limitLabels[key]}: {value.preview!.limits[key]}</p>)}</details>
      <p>Expires <time dateTime={value.preview.expires_at}>{new Date(value.preview.expires_at).toLocaleString()}</time>. Approval grants unattended compute with isolated repository access; no merge authority. It will not include your uncommitted source changes.</p>
      <button type="button" disabled={!eligible || locked || previewRejected || Date.parse(value.preview.expires_at) <= Date.now()} onClick={() => void start()}>Start job with reviewed inputs</button>
    </section>}
  </>;
}
