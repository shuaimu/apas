import type { MachineWithProjects, MobileSessionSummary, SessionInstanceInfo } from "@apas/protocol";

/**
 * The machine fields of one roster entry. A live `pane_list` carries them on
 * `PaneConfig` and a bootstrap summary on `MobilePaneSummary`; the server sets
 * both from the instance that reported the pane, never from the pane data.
 */
export interface PaneMachineFields {
  machine_id?: string | null;
  hostname?: string | null;
}

/**
 * A machine as the last bootstrap listed it. `connected` is the server's
 * `daemon_connected`: that machine's daemon held a connection to the server
 * then. This app asks for no live machine list, so the answer is as old as
 * the last synchronization.
 */
export interface KnownMachine {
  machine_id: string;
  hostname: string;
  connected: boolean;
}

/**
 * Whether this viewer can act on a tab right now. `usable` includes "cannot
 * tell": the server has the final word on every control.
 */
export type PaneReachability = "usable" | "stopped" | "no_access";

/**
 * The instances describing a session: the live set once the server has pushed
 * one, else what the last bootstrap reported. Bootstrap omits the field when
 * nothing is attached and older servers never send it, so a missing set means
 * no detail rather than proof that nothing runs.
 */
export function instancesForSession(
  session: Pick<MobileSessionSummary, "instances"> | undefined,
  live: SessionInstanceInfo[] | undefined,
): SessionInstanceInfo[] {
  return live ?? session?.instances ?? [];
}

export function primaryInstance(instances: SessionInstanceInfo[]): SessionInstanceInfo | undefined {
  return instances.find((instance) => instance.primary) ?? instances[0];
}

/**
 * The host a tab is shown under. The server stamps `hostname` on every pane it
 * tags and, on an untagged pane written before multi-machine instances, the
 * host it last recorded the pane under. A pane with neither has no host: it is
 * never shown under whichever instance happens to be primary.
 */
export function paneHostname(pane: PaneMachineFields, instances: SessionInstanceInfo[]): string | null {
  const stamped = pane.hostname?.trim();
  if (stamped) return stamped;
  if (!pane.machine_id) return null;
  return instances.find((instance) => instance.machine_id === pane.machine_id)?.hostname.trim() || null;
}

/**
 * Host badges only earn their space once a project's tabs run on more than one
 * host. Counted by the name shown, so a tagged zoo-005 tab and an untagged tab
 * attributed to zoo-005 read as one host.
 */
export function rosterSpansHosts(hosts: (string | null)[]): boolean {
  return new Set(hosts.filter((host): host is string => Boolean(host))).size > 1;
}

/**
 * The host line for a project: the primary's host plus how many other
 * instances run it ("zoo-005 +1"), the way the web project list shows it.
 */
export function sessionHostSummary(
  session: Pick<MobileSessionSummary, "hostname" | "working_dir" | "is_active">,
  instances: SessionInstanceInfo[],
): string | null {
  const primary = primaryInstance(instances);
  if (session.is_active && primary && instances.length > 1) {
    return `${primary.hostname} +${instances.length - 1}`;
  }
  return session.hostname ?? session.working_dir ?? null;
}

/**
 * Whether a tab's machine can take controls, judged by the rule the server
 * routes pane controls with. It answers `stopped` or `no_access` only where
 * that rule makes the answer certain, so a partial or older view never locks
 * anyone out of a tab the server would still accept input for.
 *
 * Pass `live` only while the server keeps the instance set current: the
 * connection negotiated project instances and the session is active.
 */
export function paneReachability(
  pane: PaneMachineFields,
  instances: SessionInstanceInfo[],
  live: boolean,
): PaneReachability {
  if (!live || instances.length === 0) return "usable";
  const owner = paneOwner(pane, instances);
  if (owner === "none") return "stopped";
  if (owner === "unknown") return "usable";
  // Runtime access is evaluated per machine and per viewer: on a third-party
  // host it needs a cluster grant for this exact machine.
  return owner.runtime_access === false ? "no_access" : "usable";
}

function paneOwner(
  pane: PaneMachineFields,
  instances: SessionInstanceInfo[],
): SessionInstanceInfo | "none" | "unknown" {
  if (pane.machine_id) {
    return instances.find((instance) => instance.machine_id === pane.machine_id) ?? "none";
  }
  const host = pane.hostname?.trim();
  // An untagged pane with no attributed host falls back to the primary.
  if (!host) return primaryInstance(instances) ?? "unknown";
  const attributed = instances.find((instance) => instance.hostname.trim() === host);
  if (attributed) return attributed;
  // Instances only run side by side when all of them are capable, and a
  // capable instance never takes another host's untagged tabs.
  if (instances.length > 1) return "none";
  // A sole instance without the capability owns the whole roster. One that
  // reported no machine certainly lacks it; one that did may or may not.
  return instances[0].machine_id ? "unknown" : instances[0];
}

/** The machines of a bootstrap, reduced to what a tab's refusal needs. */
export function knownMachines(machines: MachineWithProjects[] | null | undefined): KnownMachine[] {
  return (machines ?? []).map((entry) => ({
    machine_id: entry.machine.machine_id,
    hostname: entry.machine.hostname,
    connected: entry.daemon_connected === true,
  }));
}

/**
 * Whether the machine a tab is recorded for is listed with a connected
 * daemon. A tagged tab is matched by its machine id alone, an untagged one by
 * the host it is shown under. A machine the list does not carry counts as not
 * connected: the server knows a machine only once its daemon has registered,
 * and lists it only for an account that may use it.
 */
export function paneMachineConnected(
  pane: PaneMachineFields,
  host: string | null,
  machines: KnownMachine[],
): boolean {
  if (pane.machine_id) {
    return machines.some((machine) => machine.connected && machine.machine_id === pane.machine_id);
  }
  return host !== null && machines.some((machine) => machine.connected && machine.hostname.trim() === host);
}

/**
 * Why a tab cannot take input here, or null when it can. A stopped machine is
 * described the way the server refuses its tabs' controls, word for word: one
 * sentence while that machine's daemon is connected, another while it is
 * offline. This app has no control that opens a project; the sentence says
 * what has to happen, as the server's does.
 */
export function paneUnavailableReason(
  reachability: PaneReachability,
  host: string | null,
  machineConnected: boolean,
): string | null {
  switch (reachability) {
    case "stopped":
      if (!host) return "This tab's machine is not running this project.";
      return machineConnected
        ? `${host} is not running this project. Open the project again to start it there.`
        : `${host} is not running this project because it is offline.`;
    case "no_access":
      return host
        ? `Your access does not cover ${host}, so this tab is read-only here.`
        : "Your access does not cover this tab's machine, so it is read-only here.";
    default:
      return null;
  }
}

/**
 * Why one tab cannot take input here, or null when it can or when nothing
 * says otherwise: a tab missing from the roster is left to the server.
 * `live` as for `paneReachability`; `machines` as the last bootstrap listed
 * them.
 */
export function tabUnavailableReason(
  pane: PaneMachineFields | undefined,
  instances: SessionInstanceInfo[],
  live: boolean,
  machines: KnownMachine[],
): string | null {
  if (!pane) return null;
  const host = paneHostname(pane, instances);
  return paneUnavailableReason(
    paneReachability(pane, instances, live),
    host,
    paneMachineConnected(pane, host, machines),
  );
}
