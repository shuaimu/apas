import { useState } from "react";
import { Pressable, Text, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import type { JobQuestion, JobQuestionAnswer } from "@apas/protocol";
import { ErrorNotice, FormField, SectionTitle } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { validAnswers } from "@/jobs/continuity";
import { Copy, JobAction, styles } from "@/jobs/ui";
import { useWorkerDraft, useWorkerMutation } from "./hooks";
import { useExactWorkerRun } from "./WorkerRunScreen";
import { WorkerPage } from "./ui";

export default function WorkerQuestionScreen() {
  const { workerId, runId, questionId, project } = useLocalSearchParams<{ workerId: string; runId: string; questionId: string; project?: string }>();
  const { selected, scope, execution } = useExactWorkerRun(workerId, runId, project);
  const detail = execution.data;
  const question = detail?.questions.find((value) => value.id === questionId && value.run_id === runId);
  const current = selected.entry?.detail;
  return <WorkerPage title="Exact execution question"><JobAction onPress={() => router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]", params: { workerId, runId, project: scope?.project ?? project } })}>Exact execution</JobAction>{(selected.error || execution.error) && <ErrorNotice message={selected.error ?? String(execution.error)} />}{!question && <Copy>The requested historical dialog is unavailable or not synchronized. No other question is substituted.</Copy>}{scope && detail && question && <Dialog key={`${scope.account}:${workerId}:${runId}:${question.id}:${question.version}`} account={scope.account} project={scope.project} worker={workerId} question={question} stale={current?.worker.active_run_id !== runId || detail.job.current_run_id !== runId} ready={selected.ready && Boolean(current?.allowed_actions.answer) && detail.job.allowed_actions.answer} />}</WorkerPage>;
}

function Dialog({ account, project, worker, question, stale, ready }: { account: string; project: string; worker: string; question: JobQuestion; stale: boolean; ready: boolean }) {
  const theme = useTheme();
  const scope = { account, project, worker };
  const draftKey = `answer:${question.run_id}:${question.epoch}:${question.id}:${question.version}`;
  const draft = useWorkerDraft<JobQuestionAnswer[]>(scope, draftKey, question.questions.map((item) => ({ id: item.id, selected_option_ids: [], custom_input: null })));
  const request = useWorkerMutation(scope, `answer:${question.run_id}:${question.epoch}:${question.id}:request`);
  const [error, setError] = useState<string | null>(null);
  const editable = ready && draft.loaded && !stale && question.state === "pending" && !request.record;
  const send = async () => {
    try {
      setError(null); await draft.flush();
      if (!validAnswers(question, draft.value)) throw new Error("Answer every question using its options and/or custom text");
      await request.submit({ kind: "answer", run: question.run_id, question: question.id, body: { request_id: request.requestId(), run_id: question.run_id, epoch: question.epoch, expected_version: question.version, answers: draft.value } });
    } catch (cause) { setError(String(cause)); }
  };
  return <View style={{ gap: 14 }}><Copy>Dialog {question.id}{"\n"}Run {question.run_id} · epoch {question.epoch}{"\n"}State: {question.state.replaceAll("_", " ")}</Copy>
    {stale && <ErrorNotice message="Stale run or link: this retained dialog cannot receive an answer for a newer attempt." />}
    {question.state === "confirmed" && <Copy>Provider confirmed{!request.record ? " · answered elsewhere or on an earlier device session" : ""} at {question.confirmed_at ?? "the retained confirmation time"}. The confirmed answers below, not local drafts, are authoritative.</Copy>}
    {(question.state === "answer_accepted" || question.state === "answer_delivered") && <Copy>Answer intent received; awaiting provider confirmation. Do not send it again.</Copy>}
    {question.state === "uncertain" && <ErrorNotice message="Provider delivery is uncertain. This dialog is read-only pending exact runtime reconciliation; no blind retransmission." />}
    {question.state === "cancelled" && <Copy>This dialog was cancelled; no answer will be submitted.</Copy>}
    {question.questions.map((item, index) => {
      const answer = draft.value[index] ?? { id: item.id, selected_option_ids: [], custom_input: null };
      const confirmed = question.answers?.find((value) => value.id === item.id);
      const displayed = question.state !== "pending" && confirmed ? confirmed : answer;
      return <View key={item.id} style={{ gap: 10 }}><SectionTitle>{item.header ?? `Question ${index + 1}`}</SectionTitle><Copy>{item.question}</Copy><Copy>{item.multi ? "Choose multiple options and/or add custom text" : "Choose one option and/or add custom text"}. No answer is preselected.</Copy>
        {item.options.map((option) => {
          const checked = displayed.selected_option_ids.includes(option.id);
          return <Pressable key={option.id} accessibilityRole={item.multi ? "checkbox" : "radio"} accessibilityState={{ checked, disabled: !editable }} accessibilityLabel={`${option.label}${option.description ? `. ${option.description}` : ""}`} disabled={!editable} style={[styles.action, { borderColor: checked ? theme.accent : theme.border, backgroundColor: theme.surface }]} onPress={() => {
            const ids = item.multi ? (checked ? answer.selected_option_ids.filter((id) => id !== option.id) : [...answer.selected_option_ids, option.id]) : (checked ? [] : [option.id]);
            draft.save(draft.value.map((value, row) => row === index ? { ...value, selected_option_ids: ids } : value));
          }}><Text style={{ color: theme.text, fontSize: 16 }}>{checked ? "Selected: " : ""}{option.label}</Text>{option.description && <Copy>{option.description}</Copy>}{option.preview && <Copy>{option.preview}</Copy>}</Pressable>;
        })}
        <FormField label={`Custom text for question ${index + 1}`} multiline editable={editable} value={displayed.custom_input ?? ""} onChangeText={(text) => draft.save(draft.value.map((value, row) => row === index ? { ...value, custom_input: text || null } : value))} />
      </View>;
    })}
    <Copy>{draft.saving ? "Saving encrypted answer draft…" : "Answer drafts are scoped to this account, project, worker, run and exact dialog."}</Copy>
    {(error || draft.error || request.message) && <ErrorNotice message={error ?? draft.error ?? request.message!} />}
    <JobAction disabled={!editable || !ready || draft.saving || Boolean(draft.error) || !request.loaded || request.busy || !validAnswers(question, draft.value)} onPress={() => { void send(); }}>Submit all answers for provider confirmation</JobAction>
    {request.record && <><Copy>Request {request.record.mutation.body.request_id} · {request.record.receipt?.status ?? "unconfirmed"}</Copy><JobAction disabled={!ready || request.busy} onPress={() => { void request.resolve(); }}>Resolve receipt without resending</JobAction>{!request.record.receipt && question.state === "pending" && !stale && <JobAction disabled={!ready || request.busy || !request.canRetry} onPress={() => { void request.retry().catch((cause) => setError(String(cause))); }}>Explicitly retry the saved same-ID answer</JobAction>}</>}
    {request.record?.receipt?.status === "rejected" && question.state === "pending" && !stale && <JobAction disabled={!ready} onPress={() => { void request.clear().catch((cause) => setError(String(cause))); }}>Acknowledge rejection and edit this answer draft</JobAction>}
  </View>;
}
