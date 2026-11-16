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
  type PaneMachineFields,
  type SessionInstance,
} from "./projectInstances";
import type { MachineChoice } from "./machineChoice";

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
 * read the same. The server words it by whether that machine's daemon is
 * connected: a connected machine is started by opening the project again,
 * and an offline one by nothing a viewer can do. A machine this viewer
 * cannot see reads as connected, which is what the server assumes until it
 * knows better.
 */
export function stoppedOnHostReason(hostname: string | undefined, offline = false): string {
  if (!hostname) return "This tab's machine is not running this project.";
  return offline
    ? `${hostname} is not running this project because it is offline.`
    : `${hostname} is not running this project. Open the project again to start it there.`;
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
 * Why a tab cannot be used while its machine's instance is reconnecting: the
 * machine's daemon still runs the project, but the instance is not attached.
 * Not the server's wording on purpose. The server refuses such a tab as not
 * running, but telling the user to start a project that is running would send
 * them to a Start that can only answer "already running"; the tab works again
 * by itself once the instance reattaches.
 */
export function reconnectingOnHostReason(hostname: string): string {
  return `${hostname} is reconnecting to this project. Its tabs work again once it reconnects.`;
}

/**
 * `running`: the tab's machine runs the project and this viewer may use it.
 * `no_access`: it runs, but the viewer has no runtime access there.
 * `disconnected`: the machine's daemon reports the project running, but its
 * instance is not attached: a transport loss or a restart it is coming back
 * from. `stopped`: the machine is not running the project. `offline`:
 * stopped, and its daemon is not connected either, so it cannot be started
 * from here.
 */
export type TabHostState = "running" | "disconnected" | "stopped" | "offline" | "no_access";

/**
 * What can be done about the tabs of a machine that is not running the
 * project here, or whose instance is reconnecting: forget them. Starting one
 * machine alone is not offered. A machine that owns tabs of an open project
 * is started by opening the project again, which the project's own controls
 * and its not-following notice offer (`lib/projectOpenState.ts`).
 */
export interface StoppedMachineActions {
  machineId: string;
  hostname: string;
  /** Only the project's owner may forget a machine's tabs. */
  canForget: boolean;
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
  /** Present for a tab whose machine is known and stopped, offline or
   *  disconnected. */
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
    } else if (instancesSupported && entry && !entry.daemonConnected) {
      state = "offline";
    } else if (
      instancesSupported
      && entry?.daemonConnected
      && entry.projects.some((project) => project.isRunning && projectIds.includes(project.projectId))
    ) {
      // The daemon still runs the project, so its instance lost only its own
      // connection (or has not reattached after a server restart) and comes
      // back by itself. Calling that stopped would tell the user to open a
      // project that is running there. The daemon's report lags a real stop
      // by up to one heartbeat, so a stopped machine reads this way for that
      // long.
      state = "disconnected";
    } else {
      state = "stopped";
    }
    const refusal = state === "running"
      ? undefined
      : state === "no_access"
        ? noRuntimeAccessReason(hostname ?? place.instance?.hostname ?? "that machine")
        : state === "disconnected"
          ? reconnectingOnHostReason(hostname ?? entry?.machine.hostname ?? "This tab's machine")
          : stoppedOnHostReason(hostname, state === "offline");

    let stopped: StoppedMachineActions | undefined;
    if ((state === "stopped" || state === "offline" || state === "disconnected") && place.machineId && hostname) {
      // A machine whose instance is reconnecting can be forgotten all the
      // same: the server sees no instance there, and a daemon that dropped
      // silently (a laptop gone to sleep) keeps reading as connected and
      // running until its socket errors, which can take hours. Forgetting
      // touches only the roster, and its tabs come back when that machine
      // reports them again.
      stopped = { machineId: place.machineId, hostname, canForget: isOwner };
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
    case "disconnected":
      return "disconnected";
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
    case "disconnected":
      return `${where} is reconnecting to this project`;
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
 * machines running the project, machines where it is registered, owned
 * machines to clone onto, then unavailable machines with their reasons.
 * Empty groups are left out.
 *
 * A tab on a registered machine that is not running the project opens the
 * project: the server starts every machine that owns its tabs, then adds the
 * tab. Against a server that records no open state it starts that machine
 * alone, as it always did, and the group keeps its old name. The choices say
 * which (`MachineChoice.alsoStarts`).
 */
export function groupMachineChoices(choices: MachineChoice[]): MachineChoiceGroup[] {
  const opensProject = choices.some((choice) => choice.kind === "registered" && choice.alsoStarts !== undefined);
  const groups: MachineChoiceGroup[] = [
    { key: "instance", label: "Running", choices: [] },
    { key: "registered", label: opensProject ? "Opens the project" : "Starts the project first", choices: [] },
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
      if (choice.alsoStarts === undefined) return "Registered · the project starts there, then the tab opens";
      return choice.alsoStarts.length > 0
        ? `Registered · opens the project, which also starts its tabs on ${hostList(choice.alsoStarts)}, then the tab opens`
        : "Registered · opens the project, then the tab opens";
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

/** What the clone-here confirmation says. The folder is only the one asked
 *  for: the daemon never reuses an existing folder, so when another checkout
 *  (often a separate project of the same repository) already has that name it
 *  clones into a suffixed one instead. The acknowledgement does not carry the
 *  path; the Machines page lists where the checkout went. */
export function cloneHereCopy(hostname: string, gitRemote: string): ConfirmationCopy {
  const name = cloneInstanceName(gitRemote);
  return {
    title: `Clone this project onto ${hostname}?`,
    body: [
      `${hostname} clones ${gitRemote} into ~/apas_projects/${name} (auto-suffixed, as ${name}-2 and so on, if that folder exists) as another checkout of this project, with no new branch. The project then starts there and the new tab opens on ${hostname}.`,
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
