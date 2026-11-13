import { useState } from "react";
import { Linking, ScrollView, Text, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import type { JobArtifactContent, JobReview } from "@apas/protocol";
import { ErrorNotice, SectionTitle } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { safePullRequest } from "@/jobs/continuity";
import { Copy, JobAction, styles } from "@/jobs/ui";
import { workersApi } from "./api";
import { useWorkerEvidence } from "./hooks";
import type { WorkerScope } from "./runtime";
import { useExactWorkerRun } from "./WorkerRunScreen";
import { WorkerPage } from "./ui";

export default function WorkerReviewScreen() {
  const { workerId, runId, project } = useLocalSearchParams<{ workerId: string; runId: string; project?: string }>();
  const { selected, scope, execution } = useExactWorkerRun(workerId, runId, project);
  return <WorkerPage title="Exact historical execution review"><JobAction onPress={() => router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]", params: { workerId, runId, project: scope?.project ?? project } })}>Exact execution</JobAction>{(selected.error || execution.error) && <ErrorNotice message={selected.error ?? String(execution.error)} />}{scope && execution.data && <Review key={`${scope.account}:${workerId}:${runId}`} scope={scope} run={runId} ready={Boolean(selected.entry?.ready)} instruction={execution.data.task.instruction} />}</WorkerPage>;
}

function Review({ scope, run, ready, instruction }: { scope: WorkerScope; run: string; ready: boolean; instruction: string }) {
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [artifact, setArtifact] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const review = useWorkerEvidence<JobReview>(scope, `review:${run}:${cursor ?? "first"}`, async (signal) => {
    const response = await workersApi.review(scope.project, scope.worker, run, cursor, signal);
    if (response.run_id !== run) throw new Error("The server did not return the exact requested historical review");
    return response;
  }, ready);
  const value = review.data;
  return <View style={{ gap: 14 }}><Copy>Admitted instruction: {instruction}</Copy>{review.error && <ErrorNotice message={String(review.error)} />}{value && <>
    <Copy>Run {value.run_id}{"\n"}Base {value.base_sha}{"\n"}Candidate {value.candidate_sha ?? "not available"}</Copy>
    {run && run !== value.run_id && <ErrorNotice message="The linked run is not the returned review. No evidence from a different attempt is substituted." />}
    {(!run || run === value.run_id) && <>
      {value.reason && <ErrorNotice message={value.reason} />}
      <SectionTitle>Changed files</SectionTitle>
      {!value.files.length && <Copy>No file evidence retained on this page.</Copy>}
      {value.files.map((file) => <View key={file.path} style={{ gap: 8 }}><Copy>{file.path}{"\n"}+{file.additions} / −{file.deletions}{file.binary ? " · binary" : ""}</Copy>{file.unavailable_reason && <Copy>{file.unavailable_reason}</Copy>}{file.artifact_id && <JobAction onPress={() => setArtifact(file.artifact_id!)}>Read this file&apos;s bounded unified diff</JobAction>}</View>)}
      {previous.length > 0 && <JobAction onPress={() => { setCursor(previous[previous.length - 1]); setPrevious(previous.slice(0, -1)); setArtifact(null); }}>Previous file page</JobAction>}
      {value.next_cursor && <JobAction onPress={() => { setPrevious([...previous, cursor]); setCursor(value.next_cursor ?? null); setArtifact(null); }}>Next file page</JobAction>}
      <SectionTitle>Independent verification</SectionTitle>
      {value.verification ? <><Copy>Exact candidate {value.verification.candidate_sha}{"\n"}Base {value.verification.base_sha}{"\n"}Verifier image {value.verification.image_digest}{"\n"}Profile digest {value.verification.profile_digest}{"\n"}Completed {value.verification.completed_at}</Copy>{value.verification.checks.map((check, index) => <View key={`${check.name}:${index}`} style={{ gap: 8 }}><Copy>{check.name} · {check.status} · exit {check.exit_code ?? "unknown"} · {check.elapsed_seconds}s{"\n"}{check.argv.join(" ")}</Copy>{check.log_artifact_id && <JobAction onPress={() => setArtifact(check.log_artifact_id!)}>Read {check.name} check log</JobAction>}</View>)}</> : <Copy>No independent verification evidence is retained. This does not mean checks passed.</Copy>}
      <SectionTitle>Draft pull request handoff</SectionTitle>
      {value.publication ? <><Copy>{value.publication.repository} #{value.publication.number} · {value.publication.draft ? "draft" : "not draft"} · {value.publication.state}{"\n"}Head {value.publication.head_sha}{"\n"}Last externally observed {value.publication.observed_at}</Copy><JobAction disabled={!safePullRequest(value.publication.url, value.publication.repository)} onPress={() => { void Linking.openURL(value.publication!.url).catch((cause) => setError(String(cause))); }}>Open this repository&apos;s PR on GitHub (no approval or merge)</JobAction>{!safePullRequest(value.publication.url, value.publication.repository) && <ErrorNotice message="External PR URL is not an allowed exact repository destination." />}</> : <Copy>No PR was published. Candidate, failed checks and logs remain reviewable here.</Copy>}
      {artifact && <Artifact key={`${run}:${artifact}`} scope={scope} run={run} artifact={artifact} ready={ready} />}
    </>}
  </>}{error && <ErrorNotice message={error} />}</View>;
}

function Artifact({ scope, run, artifact, ready }: { scope: WorkerScope; run: string; artifact: string; ready: boolean }) {
  const theme = useTheme();
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const content = useWorkerEvidence<JobArtifactContent>(scope, `run:${run}:artifact:${artifact}:${cursor ?? "first"}`, (signal) => workersApi.artifact(scope.project, scope.worker, run, artifact, cursor, signal), ready);
  return <View style={{ gap: 12 }}><SectionTitle>Evidence excerpt</SectionTitle>{content.error && <ErrorNotice message={String(content.error)} />}{content.data && <><Copy>{content.data.artifact.kind} · SHA256 {content.data.artifact.sha256}{"\n"}{content.data.expired ? "Expired: content is unavailable" : "Retained content"}{content.data.artifact.truncated ? " · explicitly truncated" : ""}{"\n"}Expires {content.data.artifact.expires_at ?? "according to retention policy"}</Copy>{!content.data.expired && <ScrollView horizontal nestedScrollEnabled style={{ maxHeight: 420 }} accessibilityLabel="Horizontally scrollable code or log excerpt"><Text selectable style={[styles.code, { color: theme.text }]}>{content.data.text}</Text></ScrollView>}{previous.length > 0 && <JobAction onPress={() => { setCursor(previous[previous.length - 1]); setPrevious(previous.slice(0, -1)); }}>Previous excerpt</JobAction>}{content.data.next_cursor && <JobAction onPress={() => { setPrevious([...previous, cursor]); setCursor(content.data!.next_cursor ?? null); }}>Next bounded excerpt</JobAction>}</>}</View>;
}
