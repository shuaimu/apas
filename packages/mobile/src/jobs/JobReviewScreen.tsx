import { useState } from "react";
import { Linking, ScrollView, Text, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { useQuery } from "@tanstack/react-query";
import type { JobArtifactContent, JobReview } from "@apas/protocol";
import { ApiError } from "@/api/client";
import { ErrorNotice, SectionTitle } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { jobsApi } from "./api";
import { useJob } from "./hooks";
import { useJobs, purgeJobAccess } from "./runtime";
import { readJobRecord, writeJobRecord } from "./storage";
import type { JobScope } from "./storage";
import { safePullRequest } from "./continuity";
import { Copy, JobAction, JobPage, styles } from "./ui";

function useEvidence<T>(scope: JobScope, key: string, load: (signal: AbortSignal) => Promise<T>, ready: boolean) {
  const online = useJobs((state) => state.online && state.foreground && state.authorized);
  return useQuery({ queryKey: ["jobs", scope.account, scope.project, scope.job, key, online, ready], enabled: true, retry: false,
    queryFn: async ({ signal }) => {
      if (!online || !ready) {
        const cached = await readJobRecord<T>(scope, key);
        if (!cached) throw new Error("Evidence is not cached. Synchronize this exact job to download it.");
        return cached;
      }
      try {
        const value = await load(signal);
        const state = useJobs.getState();
        if (signal.aborted || state.account !== scope.account || !state.authorized || !state.entries[scope.job]) throw new Error("Evidence authority changed");
        await writeJobRecord(scope, key, value);
        return value;
      } catch (error) {
        if (error instanceof ApiError && [401, 403].includes(error.status)) await purgeJobAccess(error.status === 401 ? undefined : scope.project);
        throw error;
      }
    },
  });
}

export default function JobReviewScreen() {
  const { jobId, project, run } = useLocalSearchParams<{ jobId: string; project?: string; run?: string }>();
  const account = useJobs((state) => state.account);
  const selected = useJob(jobId, project);
  const detail = selected.entry?.detail;
  return <JobPage title="Retained job review"><JobAction onPress={() => router.push({ pathname: "/(code)/jobs/[jobId]", params: { jobId, project: detail?.job.project_id ?? project } })}>Job detail</JobAction>{selected.error && <ErrorNotice message={selected.error} />}{detail && account ? <Review key={`${account}:${jobId}:${run ?? "current"}`} scope={{ account, project: detail.job.project_id, job: jobId }} run={run} ready={Boolean(selected.entry?.ready)} instruction={detail.task.instruction} /> : <Copy>Authorizing the exact job before showing retained evidence.</Copy>}</JobPage>;
}

function Review({ scope, run, ready, instruction }: { scope: JobScope; run?: string; ready: boolean; instruction: string }) {
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [artifact, setArtifact] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const review = useEvidence<JobReview>(scope, `review:${run ?? "current"}:${cursor ?? "first"}`, (signal) => jobsApi.review(scope.project, scope.job, cursor, run, signal), ready);
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
      {artifact && <Artifact key={artifact} scope={scope} artifact={artifact} ready={ready} />}
    </>}
  </>}{error && <ErrorNotice message={error} />}</View>;
}

function Artifact({ scope, artifact, ready }: { scope: JobScope; artifact: string; ready: boolean }) {
  const theme = useTheme();
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const content = useEvidence<JobArtifactContent>(scope, `artifact:${artifact}:${cursor ?? "first"}`, (signal) => jobsApi.artifact(scope.project, scope.job, artifact, cursor, signal), ready);
  return <View style={{ gap: 12 }}><SectionTitle>Evidence excerpt</SectionTitle>{content.error && <ErrorNotice message={String(content.error)} />}{content.data && <><Copy>{content.data.artifact.kind} · SHA256 {content.data.artifact.sha256}{"\n"}{content.data.expired ? "Expired: content is unavailable" : "Retained content"}{content.data.artifact.truncated ? " · explicitly truncated" : ""}{"\n"}Expires {content.data.artifact.expires_at ?? "according to retention policy"}</Copy>{!content.data.expired && <ScrollView horizontal nestedScrollEnabled style={{ maxHeight: 420 }} accessibilityLabel="Horizontally scrollable code or log excerpt"><Text selectable style={[styles.code, { color: theme.text }]}>{content.data.text}</Text></ScrollView>}{previous.length > 0 && <JobAction onPress={() => { setCursor(previous[previous.length - 1]); setPrevious(previous.slice(0, -1)); }}>Previous excerpt</JobAction>}{content.data.next_cursor && <JobAction onPress={() => { setPrevious([...previous, cursor]); setCursor(content.data!.next_cursor ?? null); }}>Next bounded excerpt</JobAction>}</>}</View>;
}
