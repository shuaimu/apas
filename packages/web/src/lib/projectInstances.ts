import type { MachineWithProjects, PaneConfig, SessionInfo } from "./store";

/**
 * A project can run one instance per machine, every instance attached to the
 * project's single session. Each pane belongs to exactly one machine, and the
 * server routes a pane's controls to that machine's instance. These helpers
 * answer the questions the tab bar, the project list and the new-tab picker
 * ask about that arrangement, from the same rules the server routes by.
 */

/**
 * Mirrors `shared::PROJECT_INSTANCES_CAPABILITY`. Advertised in
 * `Authenticate.capabilities` so the server sends `session_instances`, and
 * echoed in `negotiated_capabilities` by a server that implements instances.
 * Only such a server answers `add_pane_result` or honours a target machine.
 */
export const PROJECT_INSTANCES_CAPABILITY = "project_instances_v1";

/**
 * Key for state reported by an instance with no machine id: everything an
 * older server sends, and a sole CLI that reported no machine. Such an
 * instance is always the session's primary, so requests that name no machine
 * reach it.
 */
export const LEGACY_MACHINE_KEY = "__legacy__";

/** One attached (running) instance; mirrors `shared::SessionInstanceInfo`. */
export interface SessionInstance {
  /** Absent for an attached CLI that reported no machine. It is the
   *  session's only instance, and no machine-addressed request can reach it. */
  machineId?: string;
  /** Display only; routing always uses `machineId`. */
  hostname: string;
  workingDir?: string;
  /** The earliest attached instance. It receives every control that names no
   *  machine, and controls for untagged panes with no attributed host. */
  primary: boolean;
  /** Whether this viewer may use the instance's runtime. `false` disables its
   *  tabs and its machine choice; absent means not evaluated, so the
   *  instance is treated as usable and the server decides. */
  runtimeAccess?: boolean;
}

/** The pane fields placement depends on. */
export type PaneMachineFields = Pick<PaneConfig, "machine_id" | "hostname">;

/**
 * Parse a wire instance list. `undefined` when the field is absent, which an
 * older server and a session with nothing attached both produce: either way
 * the session's own `hostname` is the description to show.
 */
export function parseSessionInstances(raw: unknown): SessionInstance[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  return raw.flatMap((item): SessionInstance[] => {
    if (!item || typeof item !== "object") return [];
    const value = item as Record<string, unknown>;
    if (typeof value.hostname !== "string") return [];
    return [{
      machineId: typeof value.machine_id === "string" && value.machine_id
        ? value.machine_id
        : undefined,
      hostname: value.hostname,
      workingDir: typeof value.working_dir === "string" ? value.working_dir : undefined,
      primary: value.primary === true,
      runtimeAccess: typeof value.runtime_access === "boolean" ? value.runtime_access : undefined,
    }];
  });
}

/** Whether the server implements instances: it answers `add_pane_result`,
 *  honours a target machine and sends `session_instances`. */
export function instancesNegotiated(state: { negotiatedCapabilities: Set<string> }): boolean {
  return state.negotiatedCapabilities.has(PROJECT_INSTANCES_CAPABILITY);
}

// Shared so a store selector returning "none" returns the same value every
// time; a fresh array would re-render its subscribers on every store change.
const NO_INSTANCES = Object.freeze([]) as unknown as SessionInstance[];

/**
 * The attached instances of a session. The list entry holds both the polled
 * `SessionInfo.instances` and every pushed `session_instances` frame, applied
 * in the order this socket received them, so whichever arrived last wins; the
 * pushed record only covers a session the list does not contain yet. Safe to
 * use directly as a store selector.
 */
export function selectSessionInstances(
  state: {
    sessions: Pick<SessionInfo, "id" | "instances">[];
    sessionInstances: Record<string, SessionInstance[]>;
  },
  sessionId: string | null | undefined,
): SessionInstance[] {
  if (!sessionId) return NO_INSTANCES;
  const session = state.sessions.find((item) => item.id === sessionId);
  if (session) return session.instances ?? NO_INSTANCES;
  return state.sessionInstances[sessionId] ?? NO_INSTANCES;
}

/** The primary instance; the first listed one if none is flagged. */
export function primaryInstance(instances: SessionInstance[]): SessionInstance | undefined {
  return instances.find((instance) => instance.primary) ?? instances[0];
}

/**
 * The live CLI connection a session's view state is keyed by (`cliClientId`:
 * the remembered tab and sidebar layout, the usage limits shown), or
 * `undefined` when no listed client reports the session, in which case each
 * caller keeps its own fallback.
 *
 * Every attached instance reports the project's one session as its active
 * session, and `cli_clients` lists them in no fixed order. Taking the first
 * would follow whichever instance happened to be listed first, so another
 * machine starting or stopping the project would re-key that state and send
 * the view back to its first tab. So: the session row's client while it is
 * live, which is the primary's (only the primary writes the row); else the
 * one already in use while its connection is still attached; only then the
 * first listed. A server without instances lists at most one client per
 * session, so there this picks what the first match always did.
 */
export function liveSessionCliClientId(
  clients: readonly { id: string; activeSession?: string }[],
  sessionId: string,
  sessionRowClientId: string | null | undefined,
  currentClientId: string | null | undefined,
): string | undefined {
  const live = clients.filter((client) => client.activeSession === sessionId);
  for (const preferred of [sessionRowClientId, currentClientId]) {
    if (preferred && live.some((client) => client.id === preferred)) return preferred;
  }
  return live[0]?.id;
}

/**
 * What a project row shows for its hosts: the primary host plus how many more
 * instances are attached ("zoo-005 +1"). `fallbackHostname` is the session's
 * recorded host, which is all an older server reports.
 */
export function instanceHostSummary(
  instances: SessionInstance[],
  fallbackHostname?: string,
): { primaryHost?: string; additionalCount: number } {
  return {
    primaryHost: primaryInstance(instances)?.hostname ?? fallbackHostname,
    additionalCount: Math.max(0, instances.length - 1),
  };
}

function hostKey(hostname: string | null | undefined): string | null {
  const trimmed = hostname?.trim().toLowerCase();
  return trimmed ? trimmed : null;
}

/** Hostnames are display names; compare them the way people read them. */
export function sameHost(a: string | null | undefined, b: string | null | undefined): boolean {
  const left = hostKey(a);
  return left !== null && left === hostKey(b);
}

function machineEntry(
  machines: MachineWithProjects[] | undefined,
  machineId: string | undefined,
): MachineWithProjects | undefined {
  if (!machineId || !machines) return undefined;
  return machines.find((entry) => entry.machine.machineId === machineId);
}

/**
 * Whether an attached instance runs an APAS without multi-machine support.
 * One that reported no machine id cannot have it. One whose daemon is
 * connected but does not advertise it is older too, since a project runs
 * inside its machine's daemon. Anything else is assumed capable: being wrong
 * that way leaves a control enabled for the server to refuse, rather than
 * disabling one that would have worked.
 */
export function instanceIsLegacy(
  instance: SessionInstance,
  machines?: MachineWithProjects[],
): boolean {
  if (!instance.machineId) return true;
  const entry = machineEntry(machines, instance.machineId);
  return Boolean(entry?.daemonConnected && !entry.projectInstancesAvailable);
}

/**
 * The attached instance that receives a pane's controls, or `null` when its
 * machine is not running the project. Mirrors the server's routing: a tagged
 * pane goes only to its machine. An untagged pane (written before instances)
 * goes to the instance on its attributed host; else to a sole instance
 * without multi-machine support, which owns the whole roster; else, with no
 * attributed host at all, to the primary. A pane attributed to a host that is
 * not attached is stopped there.
 */
export function resolvePaneInstance(
  pane: PaneMachineFields,
  instances: SessionInstance[],
  machines?: MachineWithProjects[],
): SessionInstance | null {
  if (pane.machine_id) {
    return instances.find((instance) => instance.machineId === pane.machine_id) ?? null;
  }
  const attributedHost = hostKey(pane.hostname);
  if (attributedHost) {
    const onHost = instances.find((instance) => hostKey(instance.hostname) === attributedHost);
    if (onHost) return onHost;
  }
  if (instances.length === 1 && instanceIsLegacy(instances[0], machines)) return instances[0];
  if (!attributedHost) return primaryInstance(instances) ?? null;
  return null;
}

/**
 * Whether a pane is known not to be running: its project has attached
 * instances, but none of them receives the pane's controls, because the
 * pane's machine is stopped, offline or reconnecting. Such a pane is not an
 * idle agent waiting for someone, even though the server reports it as not
 * working. With no instance list (an older server, or nothing attached)
 * nothing is known per pane and this is false, so callers keep judging by
 * the session alone.
 */
export function paneNotRunning(
  pane: PaneMachineFields,
  instances: SessionInstance[] | undefined,
  machines?: MachineWithProjects[],
): boolean {
  if (!instances || instances.length === 0) return false;
  return resolvePaneInstance(pane, instances, machines) === null;
}

/**
 * Machine ids by hostname and hostnames by machine id, gathered from every
 * source that pairs them. A hostname claimed by two different machines is
 * ambiguous and maps to neither.
 */
function machineDirectory(
  panes: PaneMachineFields[],
  instances: SessionInstance[],
  machines: MachineWithProjects[] | undefined,
): { idByHost: Map<string, string>; hostById: Map<string, string> } {
  const idByHost = new Map<string, string>();
  const ambiguousHosts = new Set<string>();
  const hostById = new Map<string, string>();
  const record = (machineId: string | null | undefined, hostname: string | null | undefined) => {
    const key = hostKey(hostname);
    if (!machineId || !key || !hostname) return;
    if (!hostById.has(machineId)) hostById.set(machineId, hostname.trim());
    if (ambiguousHosts.has(key)) return;
    const known = idByHost.get(key);
    if (known && known !== machineId) {
      idByHost.delete(key);
      ambiguousHosts.add(key);
    } else {
      idByHost.set(key, machineId);
    }
  };
  // Live attachments first, so a renamed host shows its current name.
  for (const instance of instances) record(instance.machineId, instance.hostname);
  for (const pane of panes) record(pane.machine_id, pane.hostname);
  for (const entry of machines ?? []) record(entry.machine.machineId, entry.machine.hostname);
  return { idByHost, hostById };
}

/** Where one pane runs, as the tab bar shows it. */
export interface PanePlacement {
  /** The pane's machine: its tag, or for an untagged pane the machine of its
   *  attributed host or of the instance that receives its controls. */
  machineId?: string;
  /** Host for the tab's badge. Absent for an untagged pane with no attributed
   *  host, which shows no badge. */
  hostname?: string;
  /** The instance that receives the pane's controls; `null` = stopped. */
  instance: SessionInstance | null;
  /** The pane's machine is running the project. */
  connected: boolean;
  /** Connected, and this viewer may use that instance. */
  usable: boolean;
}

/** Per-machine state for a project's tabs. */
export interface ProjectMachineState {
  /** Stable key: the machine id, else `host:<hostname>`. */
  key: string;
  machineId?: string;
  hostname?: string;
  /** An instance is attached on this machine. */
  attached: boolean;
  primary: boolean;
  paneIds: number[];
}

export interface ProjectPlacement {
  panes: Map<number, PanePlacement>;
  /** Every machine with tabs or an attached instance, attached first. */
  machines: ProjectMachineState[];
  /** The roster's tabs run on more than one machine, which is when tabs
   *  show host badges. */
  spansMachines: boolean;
}

/**
 * Place every pane of a project on its machine. `machines` is the daemon
 * inventory, used to name machines and to recognise instances without
 * multi-machine support; it may be omitted.
 */
export function projectPlacement(
  panes: (PaneMachineFields & { pane_id: number })[],
  instances: SessionInstance[],
  machines?: MachineWithProjects[],
): ProjectPlacement {
  const directory = machineDirectory(panes, instances, machines);
  const states = new Map<string, ProjectMachineState>();
  const stateFor = (machineId: string | undefined, hostname: string | undefined) => {
    const key = machineId ?? `host:${hostKey(hostname)}`;
    let state = states.get(key);
    if (!state) {
      const instance = machineId
        ? instances.find((item) => item.machineId === machineId)
        : instances.find((item) => !item.machineId && sameHost(item.hostname, hostname));
      state = {
        key,
        machineId,
        hostname: (machineId ? directory.hostById.get(machineId) : undefined) ?? hostname,
        attached: Boolean(instance),
        primary: Boolean(instance?.primary),
        paneIds: [],
      };
      states.set(key, state);
    }
    return state;
  };
  for (const instance of instances) stateFor(instance.machineId, instance.hostname);

  const placements = new Map<number, PanePlacement>();
  for (const pane of panes) {
    const instance = resolvePaneInstance(pane, instances, machines);
    const attributedHost = pane.hostname?.trim() || undefined;
    let machineId: string | undefined;
    let hostname: string | undefined;
    if (pane.machine_id) {
      machineId = pane.machine_id;
      hostname = attributedHost ?? directory.hostById.get(pane.machine_id);
    } else if (attributedHost) {
      // Untagged: shown under the host the server last recorded it under.
      machineId = directory.idByHost.get(hostKey(attributedHost) ?? "");
      hostname = attributedHost;
    } else {
      // Untagged with no attributed host: no badge of its own, but its
      // controls reach the primary, so it counts as the primary's tab.
      machineId = instance?.machineId;
    }
    placements.set(pane.pane_id, {
      machineId,
      hostname,
      instance,
      connected: instance !== null,
      usable: instance !== null && instance.runtimeAccess !== false,
    });
    const groupHost = hostname ?? instance?.hostname;
    if (machineId || groupHost) stateFor(machineId, groupHost).paneIds.push(pane.pane_id);
  }

  const machineStates = Array.from(states.values()).sort((a, b) =>
    Number(b.primary) - Number(a.primary)
    || Number(b.attached) - Number(a.attached)
    || (a.hostname ?? "").localeCompare(b.hostname ?? ""),
  );
  return {
    panes: placements,
    machines: machineStates,
    spansMachines: machineStates.filter((state) => state.paneIds.length > 0).length > 1,
  };
}

/**
 * A pane's machine id, for "Start on <host>" and for defaulting a new tab to
 * the selected tab's machine. Undefined when the pane names no machine and
 * its attributed host is not a known machine.
 */
export function paneMachineId(
  pane: PaneMachineFields & { pane_id: number },
  instances: SessionInstance[],
  machines?: MachineWithProjects[],
): string | undefined {
  return projectPlacement([pane], instances, machines).panes.get(pane.pane_id)?.machineId;
}
