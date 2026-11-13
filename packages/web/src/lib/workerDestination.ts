export type WorkerRouteDestination =
  | { kind: "list" }
  | { kind: "new" }
  | { kind: "worker"; id: string; runId?: string; questionId?: string; review?: boolean }
  | { kind: "invalid" };

/** Parse identities without falling back to a worker's latest execution. */
export function parseWorkerDestination(path: readonly string[]): WorkerRouteDestination {
  if (!path.length) return { kind: "list" };
  if (path.length === 1) return path[0] === "new" ? { kind: "new" } : { kind: "worker", id: path[0] };
  if (path[0] === "new" || path[1] !== "runs" || !path[2]) return { kind: "invalid" };
  const identity = { kind: "worker" as const, id: path[0], runId: path[2] };
  if (path.length === 3) return identity;
  if (path.length === 4 && path[3] === "review") return { ...identity, review: true };
  if (path.length === 5 && path[3] === "questions" && path[4]) return { ...identity, questionId: path[4] };
  return { kind: "invalid" };
}
