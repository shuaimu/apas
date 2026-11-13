import { useEffect, useState } from "react";
import { ActivityIndicator } from "react-native";
import { Redirect, useLocalSearchParams } from "expo-router";
import { ErrorNotice, Screen } from "@/components/ui";
import { useMobileStore } from "@/state/store";
import { retainJobLink } from "@/jobs/links";
import { handleAuthorizedLink } from "@/notifications";

export function WorkerAlias({ kind }: { kind: "list" | "new" | "detail" | "run" | "question" | "review" }) {
  const params = useLocalSearchParams<{ workerId?: string; runId?: string; questionId?: string; project?: string; attention?: string; sourceJob?: string }>();
  const { hydrated, signedIn } = useMobileStore(); const [error, setError] = useState<string | null>(null);
  const suffix = kind === "review" ? "/review" : kind === "question" ? `/questions/${encodeURIComponent(params.questionId ?? "")}` : "";
  const link = `apas://code/workers/${encodeURIComponent(params.workerId ?? "")}${params.runId ? `/runs/${encodeURIComponent(params.runId)}` : ""}${suffix}`;
  useEffect(() => {
    if (kind === "list" || kind === "new" || !params.workerId || !hydrated) return;
    if (!signedIn) { retainJobLink(link); return; }
    let live = true; void handleAuthorizedLink(link).then((accepted) => { if (live && !accepted) setError("This exact worker destination is unavailable or unauthorized."); }).catch((cause) => { if (live) setError(String(cause)); });
    return () => { live = false; };
  }, [hydrated, signedIn, link, params.workerId, kind]);
  if (!hydrated) return <Screen><ActivityIndicator /></Screen>;
  if (!signedIn) return <Redirect href="/login" />;
  if (kind === "list") return <Redirect href={{ pathname: "/(code)/workers", params: { project: params.project, attention: params.attention } }} />;
  if (kind === "new") return <Redirect href={{ pathname: "/(code)/workers/new", params: { project: params.project, workerId: params.workerId, sourceJob: params.sourceJob } }} />;
  return <Screen>{error ? <ErrorNotice message={error} /> : <ActivityIndicator />}</Screen>;
}
