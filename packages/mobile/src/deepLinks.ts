export type AuthorizedDeepLink =
  | { kind: "home" }
  | { kind: "session"; sessionId: string }
  | { kind: "new-task"; instruction?: string };

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export function resolveAuthorizedDeepLink(
  rawUrl: string,
  authorizedSessionIds: ReadonlySet<string>,
): AuthorizedDeepLink | null {
  let url: URL;
  try { url = new URL(rawUrl); } catch { return null; }
  let segments: string[];
  if (url.protocol === "apas:" && url.hostname === "code") {
    segments = url.pathname.split("/").filter(Boolean);
  } else if (url.protocol === "https:" && url.hostname === "apas.mpaxos.com") {
    const all = url.pathname.split("/").filter(Boolean);
    if (all.shift() !== "code") return null;
    segments = all;
  } else return null;

  if (segments.length === 0) return { kind: "home" };
  if (segments[0] === "session" && segments.length === 2 && UUID.test(segments[1])) {
    return authorizedSessionIds.has(segments[1]) ? { kind: "session", sessionId: segments[1] } : null;
  }
  if (segments[0] === "new" && segments.length === 1) {
    const instruction = url.searchParams.get("instruction")?.trim();
    return { kind: "new-task", instruction: instruction && instruction.length <= 4000 ? instruction : undefined };
  }
  return null;
}
