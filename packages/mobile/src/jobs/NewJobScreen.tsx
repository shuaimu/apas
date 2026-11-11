import { useEffect, useState } from "react";
import { View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { useQuery } from "@tanstack/react-query";
import type { JobLimits, JobPreview, JobPreviewRequest } from "@apas/protocol";
import { ErrorNotice, FormField, SectionTitle } from "@/components/ui";
import { useJobs, purgeJobAccess } from "./runtime";
import { jobsApi } from "./api";
import { ApiError } from "@/api/client";
import { useJobDraft, useJobMutation } from "./hooks";
import { Allowance, Copy, JobAction, JobPage } from "./ui";
import { authorizeJobStorageProject, writeJobRecord } from "./storage";

interface ComposerDraft { instruction: string; criteria: string; source: string; base: string; machine: string; worker: string; model: string; verification: string; limits: JobLimits | null; preview: JobPreview | null }
const initial: ComposerDraft = { instruction: "", criteria: "", source: "", base: "HEAD", machine: "", worker: "", model: "", verification: "", limits: null, preview: null };

export default function NewJobScreen() {
  const { project: linkedProject } = useLocalSearchParams<{ project?: string }>();
  const state = useJobs();
  const selection = useJobDraft(state.account ? { account: state.account, project: "", job: "" } : null, "composer-project", { project: linkedProject ?? "" });
  return <JobPage title="New autonomous job"><Copy>Select a project. Targets come from autonomous readiness, never terminal profiles.</Copy>{selection.error && <ErrorNotice message={selection.error} />}{state.projects.map((project) => <JobAction key={project.id} disabled={!selection.loaded} selected={selection.value.project === project.id} onPress={() => selection.save({ project: project.id })}>{project.name}</JobAction>)}{!state.projects.length && <Copy>No project inventory is currently available. Reconnect to refresh project access.</Copy>}{selection.loaded && state.projects.some((project) => project.id === selection.value.project) && state.account && <Composer key={`${state.account}:${selection.value.project}`} account={state.account} project={selection.value.project} />}</JobPage>;
}

function Composer({ account, project }: { account: string; project: string }) {
  const state = useJobs();
  const online = state.online && state.foreground && state.authorized;
  const scope = { account, project, job: "" };
  const draft = useJobDraft(scope, "composer", initial);
  const request = useJobMutation(scope, "admission-request");
  const [error, setError] = useState<string | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const targets = useQuery({ queryKey: ["jobs", account, project, "targets"], enabled: online, retry: false,
    queryFn: async ({ signal }) => {
      try {
        const response = await jobsApi.targets(project, signal);
        if (signal.aborted || useJobs.getState().account !== account) throw new Error("Project authority changed");
        authorizeJobStorageProject(account, project);
        return response;
      } catch (cause) {
        if (cause instanceof ApiError && [401, 403, 404, 410].includes(cause.status)) await purgeJobAccess(cause.status === 401 ? undefined : project);
        throw cause;
      }
    },
  });
  const value = draft.value;
  const target = targets.data?.targets.find((item) => item.machine_id === value.machine);
  const profile = target?.profiles.find((item) => item.key === value.worker);
  const eligible = Boolean(target?.available && target.allowed_actions.admit && profile?.ready && profile.model_profiles.some((item) => item.key === value.model) && profile.verification_profiles.some((item) => item.key === value.verification));
  const preview = value.preview;
  const change = (fields: Partial<ComposerDraft>) => draft.save({ ...value, ...fields, preview: null });
  useEffect(() => {
    const receipt = request.record?.receipt;
    if (!receipt?.job_id || !["accepted", "completed"].includes(receipt.status)) return;
    const jobId = receipt.job_id;
    let live = true;
    void (async () => {
      const owner = { account, project, job: "" };
      await writeJobRecord(owner, `receipt:${receipt.request_id}`, receipt);
      // Clear the reviewed inputs before retiring the pending identity, so a
      // process interruption cannot offer the old preview with a new request ID.
      await writeJobRecord(owner, "composer", initial);
      await writeJobRecord(owner, "admission-request", null);
      if (live && useJobs.getState().account === account) router.replace({ pathname: "/(code)/jobs/[jobId]", params: { jobId, project } });
    })().catch((cause) => { if (live) setError(`Receipt recovered, but protected storage could not finalize it: ${String(cause)}`); });
    return () => { live = false; };
  }, [request.record?.receipt, account, project]);
  const review = async () => {
    setError(null); setReviewing(true);
    try {
      await draft.flush();
      if (!eligible || !value.limits || !value.instruction.trim() || !value.criteria.trim()) throw new Error("Choose a ready target and all profiles, instruction and acceptance criteria");
      if (Object.values(value.limits).some((limit) => !Number.isSafeInteger(limit) || limit <= 0)) throw new Error("Every limit must be a positive integer");
      const body: JobPreviewRequest = { instruction: value.instruction.trim(), acceptance_criteria: value.criteria.split("\n").map((line) => line.trim()).filter(Boolean), source_url: value.source.trim() || null, machine_id: value.machine, worker_profile: value.worker, model_profile: value.model, verification_profile: value.verification, base_ref: value.base, limits: value.limits };
      const immutable = await jobsApi.preview(project, body);
      if (useJobs.getState().account === account) draft.save({ ...value, preview: immutable });
    } catch (cause) { setError(String(cause)); } finally { setReviewing(false); }
  };
  const start = async () => {
    setError(null);
    try {
      await draft.flush();
      if (!preview || Date.parse(preview.expires_at) <= Date.now() || !eligible) throw new Error("Preview or eligibility changed. Request a fresh review; no inputs were silently changed.");
      await request.submit({ kind: "admit", body: { request_id: request.requestId(), preview_id: preview.id, fingerprint: preview.fingerprint } });
    } catch (cause) { setError(String(cause)); }
  };
  const locked = !draft.loaded || Boolean(request.record) || request.busy;
  return <View style={{ gap: 14 }}>
    <SectionTitle>Task and acceptance</SectionTitle>
    <Copy>{draft.saving ? "Saving encrypted draft…" : draft.loaded ? "Draft stored in encrypted account/project storage" : "Restoring protected draft…"}</Copy>
    {(error || draft.error || targets.error) && <ErrorNotice message={error ?? draft.error ?? String(targets.error)} />}
    <FormField label="Instruction" multiline value={value.instruction} editable={!locked} onChangeText={(instruction) => change({ instruction })} />
    <FormField label="Acceptance criteria (one per line)" multiline value={value.criteria} editable={!locked} onChangeText={(criteria) => change({ criteria })} />
    <FormField label="Source URL (optional)" autoCapitalize="none" value={value.source} editable={!locked} onChangeText={(source) => change({ source })} />
    <FormField label="Committed base reference" autoCapitalize="none" value={value.base} editable={!locked} onChangeText={(base) => change({ base })} />
    <SectionTitle>Target and isolated profiles</SectionTitle>
    {targets.data?.unavailable_reasons.map((reason) => <ErrorNotice key={reason} message={reason} />)}
    {targets.data?.targets.map((item) => <View key={item.machine_id} style={{ gap: 8 }}><JobAction disabled={locked || !item.available || !item.allowed_actions.admit} selected={item.machine_id === value.machine} onPress={() => change({ machine: item.machine_id, worker: "", model: "", verification: "", limits: null })}>{item.hostname} · {item.available && item.allowed_actions.admit ? "Eligible" : "Unavailable"}</JobAction>{item.unavailable_reasons.map((reason) => <Copy key={reason}>{reason}</Copy>)}</View>)}
    {target?.profiles.map((item) => <View key={item.key}><JobAction disabled={locked || !item.ready} selected={value.worker === item.key} onPress={() => change({ worker: item.key, model: "", verification: "", limits: Object.fromEntries(Object.entries(item.limits).map(([key, limit]) => [key, Math.min(limit, target.policy.policy.limits[key as keyof JobLimits])])) as unknown as JobLimits })}>{item.label} · {item.ready ? "Ready" : "Not ready"}</JobAction>{item.unavailable_reasons.map((reason) => <Copy key={reason}>{reason}</Copy>)}</View>)}
    {profile && <><Copy>Model profile</Copy>{profile.model_profiles.map((item) => <JobAction key={item.key} disabled={locked} selected={value.model === item.key} onPress={() => change({ model: item.key })}>{item.label} · {item.model}</JobAction>)}<Copy>Independent verification profile</Copy>{profile.verification_profiles.map((item) => <JobAction key={item.key} disabled={locked} selected={value.verification === item.key} onPress={() => change({ verification: item.key })}>{item.label}</JobAction>)}</>}
    {value.limits && <><SectionTitle>Finite allowances</SectionTitle>{(Object.keys(value.limits) as (keyof JobLimits)[]).map((key) => <FormField key={key} label={key === "spend_micro_usd" ? "Reported spend threshold (micro USD; 1 USD = 1000000)" : key.replaceAll("_", " ")} keyboardType="number-pad" value={String(value.limits![key])} editable={!locked} onChangeText={(input) => change({ limits: { ...value.limits!, [key]: Number(input) } })} />)}</>}
    <Copy>Isolated worker access is limited to the reviewed repository/profile. Candidate code and repository CI remain untrusted. Publication creates a draft PR, not an approval or merge.</Copy>
    <JobAction disabled={locked || !online || !eligible || reviewing || Boolean(draft.error)} onPress={() => { void review(); }}>Resolve immutable admission preview</JobAction>
    {preview && <View style={{ gap: 12 }}><SectionTitle>Explicit admission review</SectionTitle><Copy>{preview.task.instruction}</Copy>{preview.task.acceptance_criteria.map((criterion, index) => <Copy key={index}>{criterion}</Copy>)}<Copy>Repository {preview.task.repository}{"\n"}Base {preview.task.base_ref} @ {preview.task.base_sha}{"\n"}Host {preview.hostname}{"\n"}Worker {preview.task.worker_profile} @ {preview.task.worker_profile_digest}{"\n"}Model {preview.task.model_profile}{"\n"}Verifier {preview.task.verification_profile} @ {preview.task.verification_profile_digest}{"\n"}Expires {preview.expires_at}</Copy><Allowance limits={preview.limits} /><JobAction disabled={!online || !state.mutations || !eligible || !request.loaded || Boolean(request.record) || draft.saving || request.busy} onPress={() => { void start(); }}>Start this reviewed job</JobAction></View>}
    {request.message && <ErrorNotice message={request.message} />}
    {request.record && <><Copy>Saved request: {request.record.mutation.body.request_id}</Copy><JobAction disabled={!online || request.busy} onPress={() => { void request.resolve(); }}>Resolve saved receipt (does not resend)</JobAction>{!request.record.receipt && <JobAction disabled={!online || !state.mutations || !eligible || request.busy || !request.canRetry} onPress={() => { void request.retry().catch((cause) => setError(String(cause))); }}>Explicitly retry original request ID</JobAction>}{request.record.receipt?.status === "rejected" && <JobAction onPress={() => { void request.clear().then(() => change({ preview: null })).catch((cause) => setError(String(cause))); }}>Keep draft and request a new review</JobAction>}</>}
  </View>;
}
