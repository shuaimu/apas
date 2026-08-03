import { useStore } from "@/lib/store";

export type ProjectRole = "owner" | "admin" | "user";

export function normalizeProjectRole(raw: string | undefined | null): ProjectRole {
  const normalized = (raw ?? "").trim().toLowerCase();
  if (normalized === "owner" || normalized === "admin") return normalized;
  return "user";
}

/**
 * Whether the viewer may change project-level *settings* — team mode on/off
 * and the Tech Lead autonomy flags.
 *
 * Mirrors the server's `share::ProjectRole::can_manage_access`, which is what
 * actually enforces this; the UI only decides what to render. Keep the two in
 * step — a control the server will reject is worse than no control at all.
 *
 * A session that was never shared has only its owner looking at it, so there is
 * no role row to consult.
 */
export function canManageProject(session: {
  isShared?: boolean;
  shareRole?: ProjectRole;
}): boolean {
  if (!session.isShared) return true;
  const role = normalizeProjectRole(session.shareRole);
  return role === "owner" || role === "admin";
}

/**
 * `canManageProject` for whichever session the Overview is currently showing.
 *
 * Defaults to **false** while the session list is still loading: briefly
 * hiding a control from its owner is recoverable, offering one to a plain user
 * is not.
 */
export function useCanManageCurrentProject(): boolean {
  const sessionId = useStore((s) => s.sessionId);
  const sessions = useStore((s) => s.sessions);
  if (!sessionId) return false;
  const session = sessions.find((s) => s.id === sessionId);
  if (!session) return false;
  return canManageProject(session);
}

/**
 * Whether managed team mode is switched on for the current project.
 *
 * Absent flags read as **off**, which is also what a CLI too old to send the
 * field produces — the UI must not offer a team the CLI will refuse to start.
 */
export function useTeamEnabled(): boolean {
  const sessionId = useStore((s) => s.sessionId);
  const projectFlags = useStore((s) => s.projectFlags);
  if (!sessionId) return false;
  return projectFlags[sessionId]?.teamEnabled === true;
}
