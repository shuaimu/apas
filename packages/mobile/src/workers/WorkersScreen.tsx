import { useState } from "react";
import { View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { ErrorNotice, SectionTitle } from "@/components/ui";
import { Copy, JobAction } from "@/jobs/ui";
import { workersApi } from "./api";
import { useWorkerEvidence } from "./hooks";
import { useWorkers } from "./runtime";
import { WorkerPage } from "./ui";
import { workerAttentionDestination } from "./links";

export default function WorkersScreen() {
  const params = useLocalSearchParams<{ project?: string; attention?: string }>();
  const state = useWorkers();
  const [project, setProject] = useState(params.project ?? "");
  const [status, setStatus] = useState(params.attention === "1" ? "attention" : "");
  const [cursor, setCursor] = useState<string | null>(null);
  const [previous, setPrevious] = useState<(string | null)[]>([]);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const page = useWorkerEvidence(state.account ? { account: state.account, project, worker: "" } : null, `list:${status}:${cursor ?? "first"}`, (signal) => workersApi.list(project || undefined, cursor, status || undefined, signal), state.authorized);
  const select = (nextProject: string, nextStatus: string) => { setProject(nextProject); setStatus(nextStatus); setCursor(null); setPrevious([]); };
  const visibleWorkers = page.data?.workers.filter((worker) => !state.revoked[worker.project_id]) ?? [];
  return <WorkerPage title={status === "attention" ? "Workers · Attention" : "Autonomous workers"}>
    <JobAction onPress={() => router.push({ pathname: "/(code)/workers/new", params: project ? { project } : {} })}>Approve a new responsibility</JobAction>
    <JobAction onPress={() => router.push("/(code)/jobs")}>Earlier finite-job results (read-only)</JobAction>
    <SectionTitle>Project-authorized inventory</SectionTitle>
    <JobAction selected={!project} onPress={() => select("", status)}>All accessible projects</JobAction>
    {state.projects.map((value) => <JobAction key={value.id} selected={project === value.id} onPress={() => select(value.id, status)}>{value.name}</JobAction>)}
    <Copy>Project filtering happens before server pagination, independently of the global page.</Copy>
    {["", "attention", "working", "idle", "needs_input", "waiting_review", "waiting_budget", "pausing", "paused", "stopping", "blocked", "recovery_required"].map((value) => <JobAction key={value} selected={status === value} onPress={() => select(project, value)}>{value ? value.replaceAll("_", " ") : "All states"}</JobAction>)}
    {page.error && <ErrorNotice message={String(page.error)} />}
    {state.revoked[project] && <ErrorNotice message="This project's protected inventory was purged after access loss." />}
    <JobAction onPress={() => { void page.refetch(); }}>Refresh authorized inventory</JobAction>
    <Copy>{visibleWorkers.length} workers on this loaded page; not a total.</Copy>
    {visibleWorkers.map((worker) => <View key={worker.id} style={{ gap: 10 }}>
      <SectionTitle>{worker.title}</SectionTitle>
      <Copy>{worker.state.replaceAll("_", " ")} · {worker.reason ?? "No blocking reason"}{"\n"}Project {worker.project_id} · host {worker.hostname} / {worker.machine_id}{"\n"}Last confirmed {worker.updated_at}; next check {worker.next_check_at ?? "not scheduled"}{"\n"}Current item {worker.current_item_id ?? "none"}</Copy>
      <Copy>{expanded[worker.id] || worker.description.length <= 220 ? worker.description : `${worker.description.slice(0, 220)}…`}</Copy>
      {worker.description.length > 220 && <JobAction onPress={() => setExpanded({ ...expanded, [worker.id]: !expanded[worker.id] })}>{expanded[worker.id] ? "Collapse description" : "Expand full description in place"}</JobAction>}
      {worker.attention && <JobAction onPress={() => router.push(workerAttentionDestination(worker))}>Open this exact question, review or recovery reason</JobAction>}
      <JobAction onPress={() => router.push({ pathname: "/(code)/workers/[workerId]", params: { workerId: worker.id, project: worker.project_id } })}>Open responsibility, checklist and exact attention</JobAction>
    </View>)}
    {previous.length > 0 && <JobAction onPress={() => { setCursor(previous[previous.length - 1]); setPrevious(previous.slice(0, -1)); }}>Previous worker page</JobAction>}
    {page.data?.next_cursor && <JobAction onPress={() => { setPrevious([...previous, cursor]); setCursor(page.data!.next_cursor!); }}>Next worker page</JobAction>}
  </WorkerPage>;
}
