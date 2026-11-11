import type { MachineProject, MachineWithProjects, SessionInfo } from "./store";
import { primaryInstance, sameHost, type SessionInstance } from "./projectInstances";

export interface MachineProjectTarget {
  machineId: string;
  projectId: string;
  isRunning: boolean;
  hostname?: string;
}

/** Inspect one registered placement; visiting this URL never starts it. */
export function machineProjectHref(target: {
  machineId: string;
  projectId: string;
  clusterOwnerUserId?: string;
}): string {
  const owner = encodeURIComponent(target.clusterOwnerUserId || "owned");
  const machine = encodeURIComponent(target.machineId);
  const project = encodeURIComponent(target.projectId);
  return `/machines?cluster_owner=${owner}&machine=${machine}&project=${project}#project-${machine}-${project}`;
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

interface Placement {
  entry: MachineWithProjects;
  project: MachineProject;
}

function targetOf(placement: Placement, attached: boolean): MachineProjectTarget {
  return {
    machineId: placement.entry.machine.machineId,
    projectId: placement.project.projectId,
    // An attached instance is running even when the inventory has not
    // caught up with it yet.
    isRunning: placement.project.isRunning || attached,
    hostname: placement.entry.machine.hostname,
  };
}

/**
 * The machine the project view's Start, Close and boot controls address.
 *
 * A project may be registered on several machines and run an instance on each,
 * so this matches by stable project identity and then prefers, in order: the
 * primary attached instance's machine, any other attached instance's, a
 * running placement, the preferred registration machine, the session's
 * recorded host, and finally hostname order, so the answer is deterministic
 * rather than `null`. Start and Close then act on the project as a whole.
 *
 * Only legacy rows whose inventory knows the project under another id fall
 * back to matching the working directory, and that guess is made only when
 * it is unambiguous across machines.
 */
export function resolveMachineProjectTarget(
  session: (Pick<SessionInfo, "id" | "projectId" | "workingDir" | "hostname"> & {
    instances?: SessionInstance[];
  }) | undefined,
  machines: MachineWithProjects[],
  instances: SessionInstance[] = session?.instances ?? [],
): MachineProjectTarget | null {
  if (!session) return null;

  let matches: Placement[] = [];
  for (const projectId of [session.projectId, session.id]) {
    if (!projectId) continue;
    for (const entry of machines) {
      for (const project of entry.projects) {
        if (project.projectId === projectId) matches.push({ entry, project });
      }
    }
    if (matches.length > 0) break;
  }
  if (matches.length === 0) return resolveByWorkingDir(session, machines);

  // A daemon may transiently repeat a project; keep one placement per
  // machine, preferring the running one.
  const byMachine = new Map<string, Placement>();
  for (const placement of matches) {
    const machineId = placement.entry.machine.machineId;
    const existing = byMachine.get(machineId);
    if (!existing || (!existing.project.isRunning && placement.project.isRunning)) {
      byMachine.set(machineId, placement);
    }
  }
  matches = Array.from(byMachine.values()).sort((a, b) =>
    a.entry.machine.hostname.localeCompare(b.entry.machine.hostname)
    || a.entry.machine.machineId.localeCompare(b.entry.machine.machineId));

  const isAttached = (placement: Placement) => instances.some((instance) => instance.machineId
    ? instance.machineId === placement.entry.machine.machineId
    : sameHost(instance.hostname, placement.entry.machine.hostname));
  const onInstance = (instance: SessionInstance | undefined) => instance
    ? matches.find((placement) => instance.machineId
      ? placement.entry.machine.machineId === instance.machineId
      : sameHost(placement.entry.machine.hostname, instance.hostname))
    : undefined;

  const primary = primaryInstance(instances);
  const chosen = onInstance(primary)
    ?? instances.map(onInstance).find(Boolean)
    ?? matches.find((placement) => placement.project.isRunning)
    ?? matches.find((placement) =>
      placement.project.preferredMachineId === placement.entry.machine.machineId)
    ?? matches.find((placement) => sameHost(placement.entry.machine.hostname, session.hostname))
    ?? matches[0];
  return targetOf(chosen, isAttached(chosen));
}

/** Legacy path match: a session hostname wins when several hosts register the
 *  same path; otherwise only an unambiguous single machine is chosen. */
function resolveByWorkingDir(
  session: Pick<SessionInfo, "id" | "projectId" | "workingDir" | "hostname">,
  machines: MachineWithProjects[],
): MachineProjectTarget | null {
  const sessionPath = normalizeComparablePath(session.workingDir);
  if (!sessionPath) return null;

  const hostMatches: Placement[] = [];
  const allMatches: Placement[] = [];
  for (const entry of machines) {
    for (const project of entry.projects) {
      if (normalizeComparablePath(project.path) !== sessionPath) continue;
      allMatches.push({ entry, project });
      if (sameHost(entry.machine.hostname, session.hostname)) {
        hostMatches.push({ entry, project });
      }
    }
  }

  const unambiguous = (placements: Placement[]): Placement | null => {
    const machineIds = new Set(placements.map((placement) => placement.entry.machine.machineId));
    return machineIds.size === 1 ? placements[0] : null;
  };
  if (hostMatches.length > 0) {
    const onHost = unambiguous(hostMatches);
    return onHost ? targetOf(onHost, false) : null;
  }
  const only = unambiguous(allMatches);
  return only ? targetOf(only, false) : null;
}
