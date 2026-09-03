import type { CliClient, MachineWithProjects, SessionInfo } from "@/lib/store";

/**
 * The project list the sidebar shows, derived once so the expanded list and
 * the collapsed icon rail can never disagree about which projects exist, which
 * are running, or what order they come in.
 */

export type ProjectRole = "owner" | "user";

export interface ProjectEntry {
  /** Representative session id — what `attachSession` takes. */
  id: string;
  /** Stable `.apas` identity; falls back to the session id for legacy rows. */
  projectId: string;
  name: string;
  workingDir: string;
  hostname?: string;
  gitRemote?: string;
  gitRemoteUrl?: string;
  isActive: boolean;
  createdAt?: string;
  isShared?: boolean;
  ownerEmail?: string;
  shareRole?: ProjectRole;
  cliClientId?: string;
}

export interface RepoGroup {
  key: string;
  label: string;
  isNoRemote: boolean;
  cloneUrl?: string;
  projects: ProjectEntry[];
}

/** Group key for projects with no git remote. */
export const NO_REMOTE_KEY = "__no_remote__";

/// The same name the project list shows, so an agent row and its project agree.
export function projectNameFor(session: { workingDir?: string; projectId?: string; id: string }): string {
  return session.workingDir?.split("/").pop()
    || `Project ${(session.projectId || session.id).slice(0, 8)}`;
}

// Turn a canonical `host/owner/repo` remote into a compact header label.
// GitHub repos drop the host (`github.com/shuaimu/apas` -> `shuaimu/apas`);
// other hosts keep it so self-hosted/GitLab repos stay distinguishable.
export function repoDisplayLabel(remote: string): string {
  return remote.startsWith("github.com/")
    ? remote.slice("github.com/".length)
    : remote;
}

// Merge CLI clients (active) and sessions (historical) into a unified project
// list. Deduplicate by project_id (the stable .apas id) so moving a project
// directory doesn't show up as a second project. Falls back to id for legacy
// rows. Sorted active first, then by creation date (newest first).
export function buildProjectList(
  sessions: SessionInfo[],
  cliClients: CliClient[],
  machines: MachineWithProjects[],
): ProjectEntry[] {
  const projectMap = new Map<string, ProjectEntry>();

  // Sort sessions by date (newest first) so we keep the most recent per project
  const sortedSessions = [...sessions].sort((a, b) => {
    if (a.createdAt && b.createdAt) {
      return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
    }
    return 0;
  });

  // Add sessions, deduplicating by project_id
  // Active sessions take precedence over inactive ones for the same project
  for (const session of sortedSessions) {
    const projectKey = session.projectId || session.id;
    const workingDir = session.workingDir || session.id;
    const name = projectNameFor(session);

    const existing = projectMap.get(projectKey);
    if (!existing || (session.isActive && !existing.isActive)) {
      projectMap.set(projectKey, {
        id: session.id,
        projectId: projectKey,
        name,
        workingDir,
        hostname: session.hostname,
        gitRemote: session.gitRemote,
        gitRemoteUrl: session.gitRemoteUrl,
        isActive: session.isActive || false,
        createdAt: session.createdAt,
        isShared: session.isShared,
        ownerEmail: session.ownerEmail,
        shareRole: session.shareRole,
        cliClientId: session.cliClientId,
      });
    }
  }

  // Also mark projects as active if current user has a connected CLI client
  // (this handles the case where server hasn't refreshed yet)
  for (const client of cliClients) {
    if (client.activeSession) {
      for (const project of projectMap.values()) {
        if (project.id === client.activeSession) {
          project.isActive = true;
          project.cliClientId = client.id;
          break;
        }
      }
    }
  }

  // Also mark projects as active if daemon reports them as running.
  // Daemon's project_id is the .apas id, so match against the project key.
  for (const machine of machines) {
    for (const mp of machine.projects) {
      if (mp.isRunning) {
        const project = projectMap.get(mp.projectId);
        if (project) {
          project.isActive = true;
        }
      }
    }
  }

  // Sort: active first, then by creation date (newest first)
  return Array.from(projectMap.values()).sort((a, b) => {
    if (a.isActive !== b.isActive) return a.isActive ? -1 : 1;
    if (a.createdAt && b.createdAt) {
      return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
    }
    return 0;
  });
}

// Group the deduped projects by the repo they belong to. Always emit a header
// per group (including the "(no remote)" bucket). Named-repo groups keep the
// activity/recency order inherited from `projects` (Array#sort is stable, so
// returning 0 preserves first-seen order); the no-remote bucket sinks last.
export function groupProjectsByRepo(projects: ProjectEntry[]): RepoGroup[] {
  const byKey = new Map<string, RepoGroup>();
  for (const project of projects) {
    const key = project.gitRemote ?? NO_REMOTE_KEY;
    let group = byKey.get(key);
    if (!group) {
      group = {
        key,
        label: project.gitRemote ? repoDisplayLabel(project.gitRemote) : "(no remote)",
        isNoRemote: !project.gitRemote,
        projects: [],
      };
      byKey.set(key, group);
    }
    // Remember a representative clone URL from any project in the group.
    if (!group.cloneUrl && project.gitRemoteUrl) {
      group.cloneUrl = project.gitRemoteUrl;
    }
    group.projects.push(project);
  }
  return Array.from(byKey.values()).sort((a, b) => {
    if (a.isNoRemote !== b.isNoRemote) return a.isNoRemote ? 1 : -1;
    return 0;
  });
}

/**
 * Up to two characters that stand in for a project when there is no room for
 * its name: initials of the first two words (`my-project` -> `MP`), or the
 * first two letters of a one-word name (`apas` -> `AP`). Word boundaries are
 * any non-alphanumeric run, so `foo_bar`, `foo.bar` and `foo bar` all agree.
 */
export function projectInitials(name: string): string {
  const tokens = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (tokens.length === 0) return "?";
  if (tokens.length >= 2) {
    return (Array.from(tokens[0])[0] + Array.from(tokens[1])[0]).toUpperCase();
  }
  return Array.from(tokens[0]).slice(0, 2).join("").toUpperCase();
}

/**
 * A stable hue (0..359) for a project, hashed from its `.apas` id so the same
 * project keeps its colour across sessions, reloads and machines.
 */
export function projectHue(key: string): number {
  let hash = 5381;
  for (let i = 0; i < key.length; i++) {
    hash = ((hash * 33) ^ key.charCodeAt(i)) >>> 0;
  }
  return hash % 360;
}
