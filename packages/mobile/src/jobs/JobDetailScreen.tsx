import { useState } from "react";
import { FlatList, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { ErrorNotice, Screen, SectionTitle } from "@/components/ui";
import { useJobs } from "./runtime";
import { useJob, useJobMutation } from "./hooks";
import { Allowance, Copy, JobAction, styles } from "./ui";

export default function JobDetailScreen() {
  const { jobId, project, run } = useLocalSearchParams<{ jobId: string; project?: string; run?: string }>();
  const account = useJobs((state) => state.account);
  const selected = useJob(jobId, project);
  const detail = selected.entry?.detail;
  const mutation = useJobMutation(account && detail ? { account, project: detail.job.project_id, job: jobId } : null, "control-request");
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<"cancel" | "retry" | "retry-publication" | null>(null);
  const mutate = async () => {
    if (!detail || !confirm) return;
    setError(null);
    try { await mutation.submit({ kind: confirm, body: { request_id: mutation.requestId(), expected_version: detail.job.state_version } }); setConfirm(null); }
    catch (cause) { setError(String(cause)); }
  };
  const params = { jobId, project: detail?.job.project_id ?? project, ...(run ? { run } : {}) };
  return <Screen><FlatList data={selected.entry?.events ?? []} keyExtractor={(event) => event.id} contentContainerStyle={styles.page}
    ListHeaderComponent={<View style={{ gap: 14 }}><SectionTitle>{detail?.job.title ?? "Autonomous job"}</SectionTitle>{!selected.ready && <Copy>Read-only · authorizing and synchronizing this job before enabling controls.</Copy>}{(selected.error || error) && <ErrorNotice message={selected.error ?? error!} />}<JobAction onPress={() => { void selected.refresh(); }}>Refresh authoritative job</JobAction><JobAction onPress={() => router.push("/(code)/jobs")}>Jobs and Attention</JobAction>{detail && <>
      <Copy>{detail.job.project_name} · {detail.job.state.replaceAll("_", " ")}{"\n"}Job {detail.job.id}{"\n"}Current run {detail.job.current_run_id}{"\n"}Target {detail.job.hostname} · {detail.job.machine_id}{"\n"}Last confirmed {detail.job.updated_at}</Copy>
      {run && !detail.runs.some((item) => item.id === run) && <ErrorNotice message="The linked run is no longer retained. This is the same job's current authorized context." />}
      {detail.job.reason && <ErrorNotice message={detail.job.reason} />}
      {detail.job.state === "cancelling" && <Copy>Cancellation requested. The host has not yet confirmed all processes stopped.</Copy>}
      <Allowance limits={detail.job.limits} usage={detail.job.usage} />
      <Copy>{detail.task.instruction}</Copy>
      <SectionTitle>Questions</SectionTitle>
      {detail.questions.length === 0 && <Copy>No retained question dialogs.</Copy>}
      {detail.questions.map((question) => <JobAction key={question.id} onPress={() => router.push({ pathname: "/(code)/jobs/[jobId]/questions/[questionId]", params: { ...params, questionId: question.id, run: question.run_id } })}>Dialog {question.id} · {question.questions.length} questions · {question.state.replaceAll("_", " ")}</JobAction>)}
      <JobAction onPress={() => router.push({ pathname: "/(code)/jobs/[jobId]/review", params })}>Retained candidate, diff, checks and draft PR</JobAction>
      <SectionTitle>Control and recovery</SectionTitle>
      {!detail.job.allowed_actions.cancel && !detail.job.allowed_actions.retry && !detail.job.allowed_actions.retry_publication && <Copy>No control action is currently authorized. Existing evidence remains readable; see the phase/reason above.</Copy>}
      {(["cancel", "retry", "retry-publication"] as const).map((action) => {
        const allowed = action === "retry-publication" ? detail.job.allowed_actions.retry_publication : detail.job.allowed_actions[action];
        return allowed && <JobAction key={action} disabled={!selected.ready || !mutation.loaded || mutation.busy || Boolean(mutation.record)} onPress={() => setConfirm(action)}>{action === "cancel" ? "Request cancellation" : action === "retry" ? "Review explicit new coding attempt" : "Retry publication of this verified candidate"}</JobAction>;
      })}
      {confirm && <><Copy>{confirm === "cancel" ? "Request durable cancellation? This is not positive termination evidence." : confirm === "retry" ? "Start a new coding attempt from the approved base? Cumulative usage is not reset; unresolved old execution must be stopped first." : "Reconcile/retry the retained verified candidate and existing draft PR without rerunning the model?"}</Copy><JobAction disabled={!selected.ready || mutation.busy} onPress={() => { void mutate(); }}>Confirm {confirm}</JobAction><JobAction onPress={() => setConfirm(null)}>Keep current job unchanged</JobAction></>}
      {mutation.message && <ErrorNotice message={mutation.message} />}
      {mutation.record && <><Copy>Request {mutation.record.mutation.body.request_id}: {mutation.record.receipt?.status ?? "unconfirmed"}</Copy><JobAction disabled={!selected.ready || mutation.busy} onPress={() => { void mutation.resolve(); }}>Resolve original receipt</JobAction>{!mutation.record.receipt && <JobAction disabled={!selected.ready || mutation.busy || !mutation.canRetry} onPress={() => { void mutation.retry().catch((cause) => setError(String(cause))); }}>Explicitly retry same request ID</JobAction>}{mutation.record.receipt && mutation.record.receipt.status !== "pending" && <JobAction onPress={() => { void mutation.clear().catch((cause) => setError(String(cause))); }}>Acknowledge receipt and show current actions</JobAction>}</>}
      <SectionTitle>Attempts and stop evidence</SectionTitle>
      {detail.runs.map((attempt) => <View key={attempt.id} style={{ gap: 6 }}><Copy>Attempt {attempt.attempt} · {attempt.id} · {attempt.state} · epoch {attempt.epoch}{"\n"}{attempt.reason}</Copy><Copy>{attempt.stop_evidence?.descendants_stopped ? `All descendants stopped at ${attempt.stop_evidence.stopped_at}` : "No positive descendant termination evidence retained"}</Copy></View>)}
      <SectionTitle>Bounded progress and delegated work</SectionTitle>
      <Copy>Only confirmed events are shown; a root response alone does not imply completed delegated work.</Copy>
    </>}</View>}
    renderItem={({ item }) => <View style={styles.card}><JobAction selected={expanded === item.id} onPress={() => setExpanded(expanded === item.id ? null : item.id)}>{item.kind.replaceAll("_", " ")} · {item.created_at}{"\n"}{item.summary}</JobAction>{expanded === item.id && <Copy>Bounded detail excerpt (up to 16,000 characters):{"\n"}{item.detail ? JSON.stringify(item.detail, null, 2).slice(0, 16000) : "No additional event details"}</Copy>}</View>}
    ListEmptyComponent={<Copy>No events in the retained local window.</Copy>}
  /></Screen>;
}
