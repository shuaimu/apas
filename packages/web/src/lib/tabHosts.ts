import type {
  AddPaneRequest,
  CliLifecycleInventory,
  MachineWithProjects,
  SessionInfo,
} from "./store";
import {
  instanceHostSummary,
  LEGACY_MACHINE_KEY,
  projectPlacement,
  sameHost,
  type PaneMachineFields,
  type SessionInstance,
} from "./projectInstances";
import { rebootToUpdateReason, type MachineChoice } from "./machineChoice";

/**
 * What the tab bar and the mobile pane chips say about where each tab runs.
 * A project can run an instance on several machines; each tab belongs to one
 * of them and works only while that machine's instance is attached. The
 * desktop and mobile views share these rules so they disable the same
 * controls for the same reasons.
 */

/**
 * The server's refusal for a control on a tab whose machine is not running
 * the project, word for word, so a disabled control and a refused request
 * read the same.
 */
export function stoppedOnHostReason(hostname: string | undefined): string {
  return hostname
    ? `${hostname} is not running this project. Start it on ${hostname} first.`
    : "This tab's machine is not running this project.";
}

/**
 * The server's refusal, word for word, for a control on an instance this
 * viewer may not use: a pane control, a reboot of that instance, or a new
 * tab on it. The store refuses a new tab there with it too, so a disabled
 * control, a local refusal and a refused request all read the same.
 */
export function noRuntimeAccessReason(hostname: string): string {
  return `Access denied: you cannot use ${hostname}, which runs this project instance`;
}

/**
 * `running`: the tab's machine runs the project and this viewer may use it.
 * `no_access`: it runs, but the viewer has no runtime access there.
 * `stopped`: the machine is not running the project. `offline`: stopped, and
 * its daemon is not connected either, so it cannot be started from here.
 */
export type TabHostState = "running" | "stopped" | "offline" | "no_access";

/** Start and forget for the tabs of a machine that is not running the project. */
export interface StoppedMachineActions {
  machineId: string;
  hostname: string;
  /** The project's id in that machine's inventory: what Start sends. */
  projectId?: string;
  /** Why Start on <host> cannot be used now; absent when it can. Start is
   *  not offered at all for a machine missing from the viewer's machine
   *  list, since the viewer could not start anything there. */
  startDisabledReason?: string;
  /** Only the project's owner may forget a machine's tabs. */
  canForget: boolean;
  /** Whether the machine is in the viewer's machine list at all. */
  canStart: boolean;
}

export interface TabHost {
  /** Badge text; absent for an untagged tab with no attributed host. */
  hostname?: string;
  machineId?: string;
  state: TabHostState;
  /** Why the tab's controls are disabled; absent when they work. */
  refusal?: string;
  /** Show the host on the tab: once the roster spans machines, and on any
   *  tab that cannot be used, which is what explains it. */
  badge: boolean;
  /** Present for a tab whose machine is known and not running the project. */
  stopped?: StoppedMachineActions;
}

export interface TabHosts {
  /** Per pane. Empty when nothing says where tabs run: an older server, or a
   *  single-machine project with nothing attached. The tab bar is then
   *  exactly as it was before instances existed, and the project's Open
   *  button is the way to start it. */
  tabs: Map<number, TabHost>;
  /** The roster's tabs run on more than one machine. */
  spansMachines: boolean;
}

export interface TabHostsInput {
  panes: (PaneMachineFields & { pane_id: number })[];
  instances: SessionInstance[];
  machines: MachineWithProjects[];
  /** The server negotiated `project_instances_v1`. Its empty instance list
   *  then means nothing is attached, and its machine entries say whether a
   *  daemon is connected. */
  instancesSupported: boolean;
  /** Ids the project may be registered under, `session.projectId` first. */
  projectIds: string[];
  /** The viewer owns the project. */
  isOwner: boolean;
}

/** Whether the viewer owns the project, the way the sidebar decides it. */
export function sessionIsOwned(session: Pick<SessionInfo, "shareRole" | "isShared"> | undefined): boolean {
  if (!session) return false;
  return session.shareRole ? session.shareRole === "owner" : session.isShared !== true;
}

function startAvailability(
  entry: MachineWithProjects,
  hostname: string,
  projectIds: string[],
  othersAttached: boolean,
): { projectId?: string; reason?: string } {
  const projectId = projectIds
    .find((id) => entry.projects.some((project) => project.projectId === id));
  if (!entry.daemonConnected) return { projectId, reason: `${hostname} is offline.` };
  // An older daemon cannot start the project beside a running instance: the
  // claim guard keeps it out. With nothing attached it starts as it always
  // has.
  if (!entry.projectInstancesAvailable && othersAttached) {
    return { projectId, reason: rebootToUpdateReason(hostname) };
  }
  if (!projectId) return { reason: `This project is not registered on ${hostname}.` };
  return { projectId };
}

export function describeTabHosts(input: TabHostsInput): TabHosts {
  const { panes, instances, machines, instancesSupported, projectIds, isOwner } = input;
  const placement = projectPlacement(panes, instances, machines);
  const tabs = new Map<number, TabHost>();
  // With nothing attached, an older server and a stopped single-machine
  // project look the same, and neither has anything to say per tab.
  const known = instances.length > 0 || (instancesSupported && placement.spansMachines);
  if (!known) return { tabs, spansMachines: placement.spansMachines };

  for (const pane of panes) {
    const place = placement.panes.get(pane.pane_id);
    if (!place) continue;
    const entry = place.machineId
      ? machines.find((candidate) => candidate.machine.machineId === place.machineId)
      : undefined;
    // An untagged tab with no attributed host shows no host of its own, even
    // though its controls reach the primary.
    const hostname = place.hostname
      ?? (pane.machine_id ? place.instance?.hostname ?? entry?.machine.hostname : undefined);
    let state: TabHostState;
    if (place.instance) {
      state = place.usable ? "running" : "no_access";
    } else {
      state = instancesSupported && entry && !entry.daemonConnected ? "offline" : "stopped";
    }
    const refusal = state === "running"
      ? undefined
      : state === "no_access"
        ? noRuntimeAccessReason(hostname ?? place.instance?.hostname ?? "that machine")
        : stoppedOnHostReason(hostname);

    let stopped: StoppedMachineActions | undefined;
    if ((state === "stopped" || state === "offline") && place.machineId && hostname) {
      const start = entry
        ? startAvailability(entry, hostname, projectIds, instances.length > 0)
        : undefined;
      stopped = {
        machineId: place.machineId,
        hostname,
        projectId: start?.projectId,
        startDisabledReason: start?.reason,
        canStart: Boolean(entry),
        canForget: isOwner,
      };
    }

    tabs.set(pane.pane_id, {
      hostname,
      machineId: place.machineId,
      state,
      refusal,
      badge: Boolean(hostname) && (placement.spansMachines || state !== "running"),
      stopped,
    });
  }
  return { tabs, spansMachines: placement.spansMachines };
}

/**
 * The hosts the project view's Open starts, as the server picks them for a
 * project-wide Start: each machine that owns tabs of the project (an untagged
 * tab counts for the machine its attributed host names), lists `projectId`,
 * is online, and can run an instance beside others. Empty when none
 * qualifies; the server then starts only the machine the request names.
 * Tabs never move between machines, so this is not necessarily the host the
 * session last recorded: an instance with no tabs may have been the last
 * primary.
 */
export function projectStartHosts(input: {
  panes: PaneMachineFields[];
  machines: MachineWithProjects[];
  /** The id the Start request sends, as the machines' inventory knows it. */
  projectId: string;
}): string[] {
  const { panes, machines, projectId } = input;
  return machines
    .filter((entry) => entry.daemonConnected
      && entry.projectInstancesAvailable
      && entry.projects.some((project) => project.projectId === projectId)
      && panes.some((pane) => pane.machine_id
        ? pane.machine_id === entry.machine.machineId
        : sameHost(pane.hostname, entry.machine.hostname)))
    .map((entry) => entry.machine.hostname)
    .sort((a, b) => a.localeCompare(b));
}

/** "zoo-005", "zoo-005 and zoo-006", "zoo-005, zoo-006 and zoo-007". */
export function hostList(hosts: string[]): string {
  if (hosts.length <= 1) return hosts[0] ?? "";
  return `${hosts.slice(0, -1).join(", ")} and ${hosts[hosts.length - 1]}`;
}

/** How a tab's host state reads after its host name. */
export function tabHostStateLabel(state: TabHostState): string {
  switch (state) {
    case "running":
      return "running";
    case "no_access":
      return "no access";
    case "offline":
      return "offline";
    case "stopped":
      return "stopped";
  }
}

/** Accessible description of a tab's host badge. */
export function tabHostDescription(host: Pick<TabHost, "hostname" | "state">): string {
  const where = host.hostname ?? "its machine";
  switch (host.state) {
    case "running":
      return `Runs on ${where}`;
    case "no_access":
      return `Runs on ${where}; you do not have access to it`;
    case "offline":
      return `${where} is offline and not running this project`;
    case "stopped":
      return `${where} is not running this project`;
  }
}

/** A project row's host line. */
export interface ProjectHostLabel {
  /** The primary host, or the recorded host when nothing is attached. */
  primaryHost?: string;
  /** How many more instances are attached. */
  additionalCount: number;
  /** "zoo-005 +1". */
  text?: string;
  /** Every attached host, for a tooltip. */
  title?: string;
  /** What a screen reader hears instead of "+1". */
  spoken?: string;
}

/**
 * One project row per project, whatever number of machines it runs on: the
 * primary host plus how many more ("zoo-005 +1"). Older servers report no
 * instances, so the session's recorded host is the whole answer there.
 */
export function projectHostLabel(
  instances: SessionInstance[] | undefined,
  fallbackHostname?: string,
): ProjectHostLabel {
  const attached = instances ?? [];
  const { primaryHost, additionalCount } = instanceHostSummary(attached, fallbackHostname);
  if (!primaryHost) return { additionalCount };
  if (additionalCount === 0) {
    return { primaryHost, additionalCount, text: primaryHost, title: primaryHost, spoken: primaryHost };
  }
  const hosts = [...attached]
    .sort((a, b) => Number(b.primary) - Number(a.primary))
    .map((instance) => instance.hostname);
  return {
    primaryHost,
    additionalCount,
    text: `${primaryHost} +${additionalCount}`,
    title: `Running on ${hosts.join(", ")}`,
    spoken: `${primaryHost} and ${additionalCount} more machine${additionalCount === 1 ? "" : "s"}`,
  };
}

/** One group of the new-tab machine step. */
export interface MachineChoiceGroup {
  key: "instance" | "registered" | "clone" | "unavailable";
  label: string;
  choices: MachineChoice[];
}

/**
 * The machine step's groups, in the order `buildMachineChoices` ranks them:
 * machines running the project, machines where it is registered (started
 * first), owned machines to clone onto, then unavailable machines with their
 * reasons. Empty groups are left out.
 */
export function groupMachineChoices(choices: MachineChoice[]): MachineChoiceGroup[] {
  const groups: MachineChoiceGroup[] = [
    { key: "instance", label: "Running", choices: [] },
    { key: "registered", label: "Starts the project first", choices: [] },
    { key: "clone", label: "Clone here", choices: [] },
    { key: "unavailable", label: "Unavailable", choices: [] },
  ];
  for (const choice of choices) {
    const key = choice.disabledReason ? "unavailable" : choice.kind;
    groups.find((group) => group.key === key)?.choices.push(choice);
  }
  return groups.filter((group) => group.choices.length > 0);
}

/** The line under a machine choice: what picking it does, or why it cannot be
 *  picked. */
export function machineChoiceDetail(choice: MachineChoice): string {
  if (choice.disabledReason) return choice.disabledReason;
  switch (choice.kind) {
    case "instance":
      return choice.primary ? "Running this project · primary" : "Running this project";
    case "registered":
      return "Registered · the project starts there, then the tab opens";
    case "clone":
      return "Not there yet · clone the repository, then start it";
  }
}

/** The folder name a clone-to-join asks for: the repository's own name. */
export function cloneInstanceName(gitRemote: string): string {
  return gitRemote.replace(/\.git$/, "").split("/").filter(Boolean).at(-1) || "project";
}

/** A confirmation's wording: a question and its paragraphs. Desktop shows it
 *  in `confirm()`, mobile in a sheet, so both say the same thing. */
export interface ConfirmationCopy {
  title: string;
  body: string[];
}

export function confirmationText(copy: ConfirmationCopy): string {
  return [copy.title, ...copy.body].join("\n\n");
}

/** What the clone-here confirmation says. */
export function cloneHereCopy(hostname: string, gitRemote: string): ConfirmationCopy {
  const name = cloneInstanceName(gitRemote);
  return {
    title: `Clone this project onto ${hostname}?`,
    body: [
      `${hostname} clones ${gitRemote} into ~/apas_projects/${name} as another checkout of this project, with no new branch. The project then starts there and the new tab opens on ${hostname}.`,
      "Cloning a large repository can take a few minutes.",
    ],
  };
}

/** What the forget confirmation says. It must not promise the tabs are gone
 *  for good: an instance on that machine reports them again. */
export function forgetMachineTabsCopy(hostname: string): ConfirmationCopy {
  return {
    title: `Forget ${hostname}'s tabs?`,
    body: [
      `They are removed from this project's tab list. They come back if ${hostname} runs this project again.`,
    ],
  };
}

/** One correlated new-tab launch as the launch list shows it. */
export interface LaunchStatus {
  /** The tab and where it was requested. */
  title: string;
  detail: string;
  /** Still waiting for an answer; nothing to dismiss yet. */
  inFlight: boolean;
  tone: "progress" | "error" | "warning";
}

export function launchStatus(request: AddPaneRequest): LaunchStatus {
  const tab = request.label?.trim() || "New tab";
  const host = request.hostname;
  const title = host ? `${tab} on ${host}` : tab;
  switch (request.status) {
    case "cloning":
      return { title, detail: `Cloning the repository onto ${host ?? "the machine"}…`, inFlight: true, tone: "progress" };
    case "pending":
      return { title, detail: "Requesting the tab…", inFlight: true, tone: "progress" };
    case "starting":
      return { title, detail: `Starting the project on ${host ?? "the machine"}…`, inFlight: true, tone: "progress" };
    case "routed":
      return { title, detail: "Opening…", inFlight: true, tone: "progress" };
    case "failed":
      return { title, detail: request.error || "The tab was not created.", inFlight: false, tone: "error" };
    case "unconfirmed":
      return {
        title,
        detail: request.error || "The tab was not confirmed. It was not resent; check the tab list before trying again.",
        inFlight: false,
        tone: "warning",
      };
  }
}

/** One instance's entry in the CLI lifecycle menu. */
export interface RebootTarget {
  key: string;
  /** Absent for a sole instance that reported no machine: reboot it without
   *  naming one. */
  machineId?: string;
  hostname: string;
  primary: boolean;
  inventory?: CliLifecycleInventory;
  disabledReason?: string;
}

/**
 * The attached instances a viewer can reboot, primary first, each with its
 * own lifecycle inventory: a reboot confirmation counts only the panes of the
 * machine it restarts.
 */
export function instanceRebootTargets(
  instances: SessionInstance[],
  inventories: Record<string, CliLifecycleInventory> | undefined,
): RebootTarget[] {
  return [...instances]
    .sort((a, b) => Number(b.primary) - Number(a.primary) || a.hostname.localeCompare(b.hostname))
    .map((instance) => ({
      key: instance.machineId ?? `host:${instance.hostname}`,
      machineId: instance.machineId,
      hostname: instance.hostname,
      primary: instance.primary,
      inventory: inventories?.[instance.machineId ?? LEGACY_MACHINE_KEY],
      disabledReason: instance.runtimeAccess === false
        ? noRuntimeAccessReason(instance.hostname)
        : undefined,
    }));
}
