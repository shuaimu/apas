import { View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { ErrorNotice, SectionTitle } from "@/components/ui";
import { useJobs } from "./runtime";
import { useJob, useJobReceipt } from "./hooks";
import { Copy, JobAction, JobPage } from "./ui";

export default function JobQuestionScreen() {
  const { jobId, project, questionId, run } = useLocalSearchParams<{ jobId: string; project?: string; questionId: string; run?: string }>();
  const account = useJobs((state) => state.account);
  const selected = useJob(jobId, project); const detail = selected.entry?.detail;
  const question = detail?.questions.find((item) => item.id === questionId && (!run || item.run_id === run));
  const receipt = useJobReceipt(account && detail ? { account, project: detail.job.project_id, job: jobId } : null, `answer:${question?.run_id}:${question?.epoch}:${questionId}:request`);
  return <JobPage title="Earlier job question (read-only)"><JobAction onPress={() => router.push({ pathname: "/(code)/jobs/[jobId]", params: { jobId, project: detail?.job.project_id ?? project, run } })}>Earlier job evidence</JobAction>{selected.error && <ErrorNotice message={selected.error} />}<Copy>Retained finite-job questions cannot be answered or redirected into a worker execution. Earlier drafts remain protected but grant no authority.</Copy>{!question && <Copy>The exact question is unavailable, expired or not synchronized. No other dialog is substituted.</Copy>}{question && <><Copy>Dialog {question.id}; run {question.run_id}; epoch {question.epoch}; state {question.state.replaceAll("_", " ")}{"\n"}Provider confirmed {question.confirmed_at ?? "not confirmed"}</Copy>{question.questions.map((item, index) => {
    const answer = question.answers?.find((value) => value.id === item.id);
    return <View key={item.id} style={{ gap: 10 }}><SectionTitle>{item.header ?? `Question ${index + 1}`}</SectionTitle><Copy>{item.question}</Copy>{item.options.map((option) => <Copy key={option.id}>{answer?.selected_option_ids.includes(option.id) ? "Selected: " : ""}{option.label}{option.description ? ` — ${option.description}` : ""}</Copy>)}{answer?.custom_input && <Copy>{answer.custom_input}</Copy>}</View>;
  })}</>}{receipt.record && <><Copy>Earlier answer request {receipt.record.mutation.body.request_id} · {receipt.record.receipt?.status ?? "unconfirmed"}</Copy><JobAction disabled={!selected.entry?.ready} onPress={() => { void receipt.resolve(); }}>Resolve original answer receipt (never resend)</JobAction></>}{receipt.message && <ErrorNotice message={receipt.message} />}</JobPage>;
}
