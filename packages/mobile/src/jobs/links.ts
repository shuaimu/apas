export interface JobDestination { job: string; question?: string; review: boolean; run?: string }
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
let pendingLink: string | null = null;
export function retainJobLink(url: string): void { pendingLink = url; }
export function consumeJobLink(): string | null { const link = pendingLink; pendingLink = null; return link; }

export function parseJobLink(raw: string): JobDestination | null {
  let url: URL;
  try { url = new URL(raw); } catch { return null; }
  if (url.username || url.password || url.port || url.hash) return null;
  let path: string[];
  if (url.protocol === "apas:" && url.hostname === "code") path = url.pathname.split("/").filter(Boolean);
  else if (url.protocol === "https:" && url.hostname === "apas.mpaxos.com") {
    path = url.pathname.split("/").filter(Boolean);
    if (path.shift() !== "code") return null;
  } else return null;
  if (path[0] !== "jobs" || !UUID.test(path[1] ?? "")) return null;
  const run = url.searchParams.get("run") ?? undefined;
  if (run && !UUID.test(run)) return null;
  if (path.length === 2) return { job: path[1], review: false, run };
  if (path.length === 3 && path[2] === "review") return { job: path[1], review: true, run };
  if (path.length === 4 && path[2] === "questions" && UUID.test(path[3])) return { job: path[1], review: false, question: path[3], run };
  return null;
}
