import type { WorkerSummary } from "@apas/protocol";

export interface WorkerDestination { worker: string; run?: string; question?: string; review: boolean }
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
export function parseWorkerLink(raw: string): WorkerDestination | null {
  let url: URL; try { url = new URL(raw); } catch { return null; }
  if (url.username || url.password || url.port || url.hash) return null;
  let path: string[];
  if (url.protocol === "apas:" && url.hostname === "code") path = url.pathname.split("/").filter(Boolean);
  else if (url.protocol === "https:" && url.hostname === "apas.mpaxos.com") { path = url.pathname.split("/").filter(Boolean); if (path.shift() !== "code") return null; }
  else return null;
  if (path[0] !== "workers" || !UUID.test(path[1] ?? "")) return null;
  if (path.length === 2) return { worker: path[1], review: false };
  if (path[2] !== "runs" || !UUID.test(path[3] ?? "")) return null;
  const destination = { worker: path[1], run: path[3], review: false };
  if (path.length === 4) return destination;
  if (path.length === 5 && path[4] === "review") return { ...destination, review: true };
  if (path.length === 6 && path[4] === "questions" && UUID.test(path[5])) return { ...destination, question: path[5] };
  return null;
}

export function workerAttentionDestination(worker: WorkerSummary) {
  const params = { workerId: worker.id, project: worker.project_id };
  if (!worker.attention_run_id) return { pathname: "/(code)/workers/[workerId]" as const, params };
  const exact = { ...params, runId: worker.attention_run_id };
  if (worker.attention_question_id) return { pathname: "/(code)/workers/[workerId]/runs/[runId]/questions/[questionId]" as const, params: { ...exact, questionId: worker.attention_question_id } };
  if (worker.state === "waiting_review") return { pathname: "/(code)/workers/[workerId]/runs/[runId]/review" as const, params: exact };
  return { pathname: "/(code)/workers/[workerId]/runs/[runId]" as const, params: exact };
}
