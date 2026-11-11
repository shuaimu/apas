import { useEffect } from "react";
import { ActivityIndicator } from "react-native";
import { Redirect, useLocalSearchParams } from "expo-router";
import { Screen } from "@/components/ui";
import { useMobileStore } from "@/state/store";
import { retainJobLink } from "./links";

export function JobAlias({ kind }: { kind: "list" | "new" | "detail" | "question" | "review" }) {
  const params = useLocalSearchParams<{ jobId?: string; questionId?: string; project?: string; run?: string; attention?: string }>();
  const hydrated = useMobileStore((state) => state.hydrated);
  const signedIn = useMobileStore((state) => state.signedIn);
  useEffect(() => {
    if (signedIn || !params.jobId) return;
    const suffix = kind === "question" ? `/questions/${encodeURIComponent(params.questionId ?? "")}` : kind === "review" ? "/review" : "";
    retainJobLink(`apas://code/jobs/${encodeURIComponent(params.jobId)}${suffix}${params.run ? `?run=${encodeURIComponent(params.run)}` : ""}`);
  }, [signedIn, kind, params.jobId, params.questionId, params.run]);
  if (!hydrated) return <Screen><ActivityIndicator /></Screen>;
  if (!signedIn) return <Redirect href="/login" />;
  if (kind === "list") return <Redirect href={{ pathname: "/(code)/jobs", params: { attention: params.attention } }} />;
  if (kind === "new") return <Redirect href={{ pathname: "/(code)/jobs/new", params: { project: params.project } }} />;
  if (!params.jobId) return <Redirect href="/(code)/jobs" />;
  const target = { jobId: params.jobId, project: params.project, run: params.run };
  if (kind === "question") return <Redirect href={{ pathname: "/(code)/jobs/[jobId]/questions/[questionId]", params: { ...target, questionId: params.questionId ?? "" } }} />;
  if (kind === "review") return <Redirect href={{ pathname: "/(code)/jobs/[jobId]/review", params: target }} />;
  return <Redirect href={{ pathname: "/(code)/jobs/[jobId]", params: target }} />;
}
