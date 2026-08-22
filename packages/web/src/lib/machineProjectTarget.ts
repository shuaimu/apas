import type { MachineWithProjects, SessionInfo } from "./store";

export interface MachineProjectTarget {
  machineId: string;
  projectId: string;
  isRunning: boolean;
}

function normalizeComparablePath(path: string | undefined): string | null {
  if (!path) return null;
  const trimmed = path.trim();
  if (!trimmed) return null;
  let normalized = trimmed;
  while (normalized.length > 1 && normalized.endsWith("/")) {
    normalized = normalized.slice(0, -1);
  }
  return normalized;
}

/**
 * Find the daemon project addressed by a session without guessing across
 * machines. A session hostname wins when the same shared project is registered
 * on several hosts; a path-only match is used only when it is unambiguous.
 */
export function resolveMachineProjectTarget(
  session: Pick<SessionInfo, "id" | "projectId" | "workingDir" | "hostname"> | undefined,
  machines: MachineWithProjects[],
): MachineProjectTarget | null {
  const sessionPath = normalizeComparablePath(session?.workingDir);
  if (!session || !sessionPath) return null;
  const sessionHostname = session.hostname?.trim().toLowerCase() || null;

  const hostMatches: MachineProjectTarget[] = [];
  const allMatches: MachineProjectTarget[] = [];

  for (const machineWithProjects of machines) {
    const machineHostname = machineWithProjects.machine.hostname.trim().toLowerCase();
    for (const project of machineWithProjects.projects) {
      if (normalizeComparablePath(project.path) !== sessionPath) continue;
      const target = {
        machineId: machineWithProjects.machine.machineId,
        projectId: project.projectId,
        isRunning: project.isRunning,
      };
      allMatches.push(target);
      if (sessionHostname && machineHostname === sessionHostname) {
        hostMatches.push(target);
      }
    }
  }

  const chooseTarget = (matches: MachineProjectTarget[]): MachineProjectTarget | null => {
    if (matches.length === 0) return null;

    // Prefer the stable project identity, then the legacy session-as-project
    // identity. More than one exact match is still ambiguous across hosts.
    for (const projectId of [session.projectId, session.id]) {
      if (!projectId) continue;
      const exactMatches = matches.filter((target) => target.projectId === projectId);
      if (exactMatches.length === 1) return exactMatches[0];
      if (exactMatches.length > 1) return null;
    }

    // A daemon may transiently repeat a project for one machine/path. Collapse
    // that harmless duplication, but never choose between different machines.
    const dedupedByMachine = new Map<string, MachineProjectTarget>();
    for (const target of matches) {
      if (!dedupedByMachine.has(target.machineId)) {
        dedupedByMachine.set(target.machineId, target);
      }
    }
    return dedupedByMachine.size === 1
      ? Array.from(dedupedByMachine.values())[0]
      : null;
  };

  if (sessionHostname) {
    const hostTarget = chooseTarget(hostMatches);
    if (hostTarget) return hostTarget;
    if (hostMatches.length > 1) return null;
  }

  return chooseTarget(allMatches);
}
