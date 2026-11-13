import { useEffect, useMemo, useState } from "react";
import { View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { useQuery } from "@tanstack/react-query";
import type { JobLimits, WorkerMandateSpec, WorkerPreview, WorkerConversionSource } from "@apas/protocol";
import { ErrorNotice, FormField, SectionTitle } from "@/components/ui";
import { Allowance, Copy, JobAction } from "@/jobs/ui";
import { jobsApi } from "@/jobs/api";
import { readJobRecord, writeJobRecord, authorizeJobStorageProject } from "@/jobs/storage";
import type { JobSnapshot } from "@/jobs/storage";
import { useJobReceipt } from "@/jobs/hooks";
import { workersApi } from "./api";
import { eligibleWorkerTarget } from "./continuity";
import { authorityCurrent, handleWorkerError, useWorkers, workerAuthority, workerStorageScope } from "./runtime";
import { useWorker, useWorkerDraft, useWorkerMutation } from "./hooks";
import { RecurringAllowance, WorkerPage } from "./ui";

const initialSpec: WorkerMandateSpec = { title: "", description: "", checklist: [], machine_id: "", worker_profile: "", verification_profile: "", model_profile: "", base_ref: "HEAD", interval_seconds: 900, limits: { per_run: { elapsed_seconds: 1, spend_micro_usd: 1, attempts: 1, model_concurrency: 1, model_output_tokens: 1, cpu_millis: 1, memory_bytes: 1, workspace_bytes: 1, output_bytes: 1, pids: 1 }, elapsed_seconds_24h: 1, spend_micro_usd_24h: 1, runs_24h: 1, attempts_per_item: 1 } };
interface Draft { spec: WorkerMandateSpec; preview: WorkerPreview | null; source: WorkerConversionSource | null; sourceNote: string }
const initial: Draft = { spec: initialSpec, preview: null, source: null, sourceNote: "" };
export default function NewWorkerScreen() {
  const params = useLocalSearchParams<{ project?: string; workerId?: string; sourceJob?: string }>();
  const state = useWorkers(); const [chosen, setChosen] = useState(params.project ?? "");
  const project = params.project ?? chosen;
  const [accessError, setAccessError] = useState<string | null>(null);
  const reauthorize = async () => {
    const stamp = workerAuthority();
    if (!state.account) return;
    try {
      await workersApi.targets(project);
      if (!authorityCurrent(stamp)) return;
      authorizeJobStorageProject(state.account, project);
      useWorkers.setState((current) => ({ revoked: Object.fromEntries(Object.entries(current.revoked).filter(([id]) => id !== project)) }));
      setAccessError(null);
    } catch (cause) { setAccessError(String(cause)); }
  };
  return <WorkerPage title={params.workerId ? "Review replacement responsibility" : "New autonomous worker"}>
    <Copy>A worker discovers concrete work inside your approved responsibility. Editing this encrypted draft does not start execution.</Copy>
    {!params.project && state.projects.map((value) => <JobAction key={value.id} selected={project === value.id} onPress={() => setChosen(value.id)}>{value.name}</JobAction>)}
    {params.project && <Copy>Project {params.project} (fixed by this destination)</Copy>}
    {state.revoked[project] ? <><ErrorNotice message={accessError ?? "Project access was revoked. Protected drafts and receipts were purged."} /><JobAction disabled={!state.online || !state.foreground} onPress={() => { void reauthorize(); }}>Recheck current project access (does not resume work)</JobAction></> : state.account && project && <Composer key={`${state.account}:${project}:${params.workerId ?? "new"}:${params.sourceJob ?? ""}`} account={state.account} project={project} worker={params.workerId ?? ""} sourceJob={params.sourceJob} />}
  </WorkerPage>;
}
function Composer({ account, project, worker, sourceJob }: { account: string; project: string; worker: string; sourceJob?: string }) {
  const state = useWorkers(); const selected = useWorker(worker, project);
  const online = state.online && state.foreground && state.authorized;
  const scope = useMemo(() => ({ account, project, worker }), [account, project, worker]);
  const draftKey = sourceJob ? `composer:job:${sourceJob}` : "composer";
  const requestKey = sourceJob ? `approval-request:job:${sourceJob}` : "approval-request";
  const draft = useWorkerDraft(scope, draftKey, initial);
  const saveDraft = draft.save;
  const request = useWorkerMutation(scope, requestKey);
  const legacyReceipt = useJobReceipt(!worker && !state.revoked[project] ? { account, project, job: "" } : null, "admission-request");
  const [error, setError] = useState<string | null>(null); const [reviewing, setReviewing] = useState(false);
  const [restoredSource, setRestoredSource] = useState(false);
  const targets = useQuery({ queryKey: ["workers", account, project, "targets"], enabled: online, retry: false, queryFn: async ({ signal }) => {
    const stamp = workerAuthority();
    try {
      const value = await workersApi.targets(project, signal);
      if (signal.aborted || !authorityCurrent(stamp)) throw new Error("Project authority changed");
      authorizeJobStorageProject(account, project);
      useWorkers.setState((current) => ({ revoked: Object.fromEntries(Object.entries(current.revoked).filter(([id]) => id !== project)) }));
      return value;
    }
    catch (cause) { if (authorityCurrent(stamp)) await handleWorkerError(cause, project); throw cause; }
  } });
  const value = draft.value; const spec = value.spec;
  const target = targets.data?.targets.find((item) => item.machine_id === spec.machine_id);
  const profile = target?.profiles.find((item) => item.key === spec.worker_profile);
  const eligible = eligibleWorkerTarget(targets.data, spec);
  const change = (fields: Partial<WorkerMandateSpec>) => { setRestoredSource(true); saveDraft({ ...value, spec: { ...spec, ...fields }, preview: null }); };
  useEffect(() => {
    if (!draft.loaded || restoredSource || value.spec.description || request.record || (worker && !selected.entry)) return;
    let live = true; const stamp = workerAuthority();
    void (async () => {
      if (worker && selected.entry) {
        const replacement = { ...initial, spec: selected.entry.detail.mandate.spec };
        await writeJobRecord(workerStorageScope(scope), draftKey, replacement);
        if (!live || !authorityCurrent(stamp)) return;
        saveDraft(replacement);
      }
      else if (sourceJob) {
        const source = online ? await jobsApi.detail(project, sourceJob) : (await readJobRecord<JobSnapshot>({ account, project, job: sourceJob }, "snapshot"))?.detail;
        if (!source) throw new Error("The earlier source is not cached; reconnect to authorize its exact instructions");
        if (!live || !authorityCurrent(stamp)) return;
        if (source.job.id !== sourceJob || source.job.project_id !== project) throw new Error("Earlier conversion source identity mismatch");
        saveDraft({ ...initial, source: { kind: "job", job_id: sourceJob }, sourceNote: `Earlier job ${sourceJob}; original model ${source.task.model_profile}, worker ${source.task.worker_profile}. No provider was selected or approved.`, spec: { ...initialSpec, title: source.task.instruction.slice(0, 120), description: source.task.instruction, checklist: source.task.acceptance_criteria, base_ref: source.task.base_ref } });
      } else {
        const old = await readJobRecord<{ instruction?: string; criteria?: string; model?: string; worker?: string }>({ account, project, job: "" }, "composer");
        if (!live || !authorityCurrent(stamp)) return;
        if (old?.instruction) saveDraft({ ...initial, sourceNote: `Unapproved legacy draft; former model ${old.model || "unspecified"}, profile ${old.worker || "unspecified"}. Fresh target and recurring review required.`, spec: { ...initialSpec, title: old.instruction.slice(0, 120), description: old.instruction, checklist: old.criteria?.split("\n") ?? [] } });
      }
      if (live) { setRestoredSource(true); setError(null); }
    })().catch((cause) => { if (live) setError(String(cause)); });
    return () => { live = false; };
  }, [draft.loaded, saveDraft, online, selected.entry, sourceJob, restoredSource, request.record, value.spec.description, account, project, worker, scope, draftKey]);
  useEffect(() => {
    const receipt = request.record?.receipt;
    if (!receipt || typeof receipt.worker_id !== "string" || !receipt.worker_id || !["accepted", "completed"].includes(receipt.status)) return;
    const workerId = receipt.worker_id; const stamp = workerAuthority(); let live = true;
    void (async () => {
      await writeJobRecord(workerStorageScope(scope), `receipt:${receipt.request_id}`, receipt);
      if (!live || !authorityCurrent(stamp)) return;
      await writeJobRecord(workerStorageScope(scope), draftKey, initial);
      if (!live || !authorityCurrent(stamp)) return;
      await writeJobRecord(workerStorageScope(scope), requestKey, null);
      if (live && authorityCurrent(stamp)) router.replace({ pathname: "/(code)/workers/[workerId]", params: { workerId, project } });
    })().catch((cause) => { if (live) setError(`Durable approval recovered; draft retirement failed: ${String(cause)}`); });
    return () => { live = false; };
  }, [request.record?.receipt, scope, project, draftKey, requestKey]);
  const review = async () => {
    const stamp = workerAuthority(); setError(null); setReviewing(true);
    try {
      await draft.flush();
      if (!eligible || !spec.title.trim() || !spec.description.trim() || !spec.checklist.some((line) => line.trim())) throw new Error("Select eligible exact profiles and provide a description and checklist");
      if ([spec.interval_seconds, ...Object.values(spec.limits.per_run), spec.limits.elapsed_seconds_24h, spec.limits.spend_micro_usd_24h, spec.limits.runs_24h, spec.limits.attempts_per_item].some((limit) => !Number.isSafeInteger(limit) || limit <= 0)) throw new Error("Every allowance and cadence must be a finite positive integer");
      const preview = await workersApi.preview(project, { worker_id: worker || null, expected_version: worker ? selected.entry!.detail.worker.state_version : null, spec: { ...spec, checklist: spec.checklist.map((line) => line.trim()).filter(Boolean) }, source: value.source });
      if (authorityCurrent(stamp)) draft.save({ ...value, preview });
    } catch (cause) { if (authorityCurrent(stamp)) { setError(String(cause)); await handleWorkerError(cause, project); } } finally { setReviewing(false); }
  };
  const approve = async () => {
    try { await draft.flush(); if (!value.preview || Date.parse(value.preview.expires_at) <= Date.now() || !eligible) throw new Error("Review expired or exact target unavailable; request a new review without retargeting"); await request.submit({ kind: "approve", body: { request_id: request.requestId(), preview_id: value.preview.id, fingerprint: value.preview.fingerprint } }); }
    catch (cause) { setError(String(cause)); }
  };
  const locked = !draft.loaded || (!restoredSource && !value.spec.description) || reviewing || Boolean(request.record) || request.busy;
  return <View style={{ gap: 14 }}>{value.sourceNote && <Copy>{value.sourceNote}</Copy>}{(error || draft.error || targets.error || selected.error) && <ErrorNotice message={error ?? draft.error ?? selected.error ?? String(targets.error)} />}<Copy>{draft.saving ? "Saving encrypted draft…" : "Account/project-scoped draft. No recurring authority yet."}</Copy>
    {legacyReceipt.record && <><SectionTitle>Earlier finite approval receipt</SectionTitle><Copy>Original request {legacyReceipt.record.mutation.body.request_id}: {legacyReceipt.record.receipt?.status ?? "unconfirmed"}. It is retained as evidence, never replayed or promoted to recurring authority.</Copy><JobAction disabled={!online} onPress={() => { void legacyReceipt.resolve(); }}>Resolve original finite-job receipt without replay</JobAction>{legacyReceipt.record.receipt?.job_id && <JobAction onPress={() => router.push({ pathname: "/(code)/jobs/[jobId]", params: { project, jobId: legacyReceipt.record!.receipt!.job_id! } })}>Open original acknowledged earlier job</JobAction>}</>}
    {legacyReceipt.message && <Copy>{legacyReceipt.message}</Copy>}
    <FormField label="Responsibility title" value={spec.title} editable={!locked} onChangeText={(title) => change({ title })} />
    <FormField label="Full job description" multiline value={spec.description} editable={!locked} onChangeText={(description) => change({ description })} />
    <FormField label="Standing checklist (one observation per line)" multiline value={spec.checklist.join("\n")} editable={!locked} onChangeText={(text) => change({ checklist: text.split("\n") })} />
    <FormField label="Approved base branch" value={spec.base_ref} editable={!locked} onChangeText={(base_ref) => change({ base_ref })} />
    <Copy>Saved exact host: {spec.machine_id || "not selected"}; worker/model/verifier: {spec.worker_profile || "none"} / {spec.model_profile || "none"} / {spec.verification_profile || "none"}. Unavailable choices are retained, never silently replaced.</Copy>
    {!eligible && spec.machine_id && <Copy>The saved host/profile combination is not currently eligible under readiness and effective recurring policy. Choose explicitly or ask the hosting operator to resolve the reported restriction; no alternative is selected automatically.</Copy>}
    {targets.data?.unavailable_reasons.map((reason) => <ErrorNotice key={reason} message={reason} />)}
    {targets.data?.targets.map((item) => <View key={item.machine_id}><JobAction disabled={locked || !item.available || !item.allowed_actions.admit || !item.policy.policy.enabled || !item.policy.policy.recurring?.enabled} selected={spec.machine_id === item.machine_id} onPress={() => change({ machine_id: item.machine_id, worker_profile: "", model_profile: "", verification_profile: "" })}>{item.hostname} · {item.machine_id}</JobAction>{item.unavailable_reasons.map((reason) => <Copy key={reason}>{reason}</Copy>)}</View>)}
    {target?.profiles.map((item) => <JobAction key={item.key} disabled={locked || !item.ready || !target.policy.policy.recurring?.enabled || !target.policy.policy.worker_profiles.includes(item.key)} selected={spec.worker_profile === item.key} onPress={() => {
      const recurring = target.policy.policy.recurring;
      if (!recurring?.enabled) return;
      const per_run: JobLimits = { ...item.limits };
      for (const key of Object.keys(per_run) as (keyof JobLimits)[]) per_run[key] = key === "attempts" ? 1 : Math.min(per_run[key], target.policy.policy.limits[key]);
      change({ worker_profile: item.key, model_profile: "", verification_profile: "", interval_seconds: Math.max(spec.interval_seconds, recurring.min_interval_seconds), limits: { per_run, elapsed_seconds_24h: recurring.max_elapsed_seconds_24h, spend_micro_usd_24h: recurring.max_spend_micro_usd_24h, runs_24h: recurring.max_runs_24h, attempts_per_item: 1 } });
    }}>{item.label} · {item.ready ? "ready" : item.unavailable_reasons.join(", ")}</JobAction>)}
    {profile && target && <><SectionTitle>Explicit provider/model and verification choices</SectionTitle>{profile.model_profiles.map((item) => <JobAction key={item.key} disabled={locked || !target.policy.policy.model_profiles.includes(item.key)} selected={spec.model_profile === item.key} onPress={() => change({ model_profile: item.key })}>{item.label} · {item.model}</JobAction>)}{profile.verification_profiles.map((item) => <JobAction key={item.key} disabled={locked || !target.policy.policy.verification_profiles.includes(item.key)} selected={spec.verification_profile === item.key} onPress={() => change({ verification_profile: item.key })}>{item.label}</JobAction>)}</>}
    <FormField label="Minimum seconds between checks" keyboardType="number-pad" value={String(spec.interval_seconds)} editable={!locked} onChangeText={(text) => change({ interval_seconds: Number(text) })} />
    <SectionTitle>Per-run ceilings (each run has one attempt)</SectionTitle>{(Object.keys(spec.limits.per_run) as (keyof JobLimits)[]).filter((key) => key !== "attempts").map((key) => <FormField key={key} label={key.replaceAll("_", " ")} keyboardType="number-pad" editable={!locked} value={String(spec.limits.per_run[key])} onChangeText={(text) => change({ limits: { ...spec.limits, per_run: { ...spec.limits.per_run, [key]: Number(text) } } })} />)}
    <SectionTitle>Recurring allowance and cumulative item attempts</SectionTitle>{(["elapsed_seconds_24h", "spend_micro_usd_24h", "runs_24h", "attempts_per_item"] as const).map((key) => <FormField key={key} label={key.replaceAll("_", " ")} keyboardType="number-pad" editable={!locked} value={String(spec.limits[key])} onChangeText={(text) => change({ limits: { ...spec.limits, [key]: Number(text) } })} />)}
    <JobAction disabled={locked || !online || !eligible || reviewing || Boolean(draft.error) || (Boolean(worker) && !selected.ready)} onPress={() => { void review(); }}>Review recurring authority</JobAction>
    {value.preview && <><SectionTitle>Approve this immutable review</SectionTitle><Copy>{value.preview.mandate.spec.description}{"\n"}{value.preview.mandate.spec.checklist.join("\n")}</Copy><Copy>Repository {value.preview.mandate.repository}; branch {value.preview.mandate.spec.base_ref}; current tip {value.preview.base_sha}. Future runs select the then-current committed tip of this branch, not this preview SHA.</Copy><Copy>Exact host {value.preview.hostname} / {value.preview.mandate.spec.machine_id}; model {value.preview.mandate.spec.model_profile}; worker {value.preview.mandate.spec.worker_profile} @ {value.preview.mandate.worker_profile_digest}; verifier {value.preview.mandate.spec.verification_profile} @ {value.preview.mandate.verification_profile_digest}. Cadence {value.preview.mandate.spec.interval_seconds}s. Expires {value.preview.expires_at}.</Copy><Allowance limits={value.preview.mandate.spec.limits.per_run} /><RecurringAllowance limits={value.preview.mandate.spec.limits} /><Copy>The worker discovers and prioritizes work within this responsibility. One draft PR waits for human review; no automatic merge. Pause after this run fences future work but preserves its result. Stop now requests full termination and waits for positive stop evidence. Neither resets usage.</Copy><JobAction disabled={!online || !state.mutations || !eligible || locked || !request.loaded || (Boolean(worker) && !selected.ready)} onPress={() => { void approve(); }}>Approve recurring worker and renewing allowance</JobAction></>}
    {request.message && <ErrorNotice message={request.message} />}{request.record && <><Copy>Saved request {request.record.mutation.body.request_id}</Copy><JobAction disabled={!online || request.busy} onPress={() => { void request.resolve(); }}>Recover original receipt without resending</JobAction><JobAction disabled={!online || !eligible || !request.canRetry || request.busy} onPress={() => { void request.retry().catch((cause) => setError(String(cause))); }}>Explicitly retry original request identity</JobAction>{request.record.receipt?.status === "rejected" && <JobAction onPress={() => { void request.clear().then(() => draft.save({ ...value, preview: null })); }}>Acknowledge rejection and request fresh review</JobAction>}</>}
  </View>;
}
