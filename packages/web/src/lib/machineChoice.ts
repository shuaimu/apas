import type { MachineProviderStatus, MachineWithProjects, PaneConfig } from "./store";
import { PROVIDER_MODEL_GROUPS } from "./providerOptions";
import {
  instanceIsLegacy,
  paneMachineId,
  resolvePaneInstance,
  sameHost,
  type SessionInstance,
} from "./projectInstances";

/**
 * Which machine a new tab runs on. A project's tabs can run on any machine
 * with an instance of it: one already attached, one where it is registered
 * (the server opens the project, which starts that machine and every other
 * machine that owns its tabs, then adds the tab), or an owned machine that
 * does not have it yet (clone there and join). This model is pure so the
 * desktop `+` menu and the mobile pane sheet offer exactly the same choices.
 */

export type MachineChoiceKind = "instance" | "registered" | "clone";

export interface MachineChoice {
  /** Stable key for lists. */
  key: string;
  /** The machine to pass to `addPane` or `cloneAndAddPane`. Absent only for a
   *  sole instance that reported no machine: add without a machine then,
   *  which reaches the primary. */
  machineId?: string;
  hostname: string;
  kind: MachineChoiceKind;
  /** The session's primary instance. */
  primary: boolean;
  /** Why the machine cannot take the tab; absent when it can. */
  disabledReason?: string;
  /** The machine's provider report (`MachineInfo.providers`); absent when
   *  unknown. A machine can take a tab of one provider and not another, so
   *  this never disables the machine: see `providerOfferOn`. */
  providers?: MachineProviderStatus[];
  /** Only on a `registered` choice, and only against a server that records
   *  open states: a tab there opens the project, and these are the other
   *  hosts whose tabs start with it (empty when there are none). Absent
   *  means the request starts that machine alone, as an older server does. */
  alsoStarts?: string[];
}

export interface MachineChoiceInput {
  /** The project's stable id: `session.projectId ?? session.id`. */
  projectId: string;
  instances: SessionInstance[];
  machines: MachineWithProjects[];
  /** The server negotiated `project_instances_v1`. Without it a tab can only
   *  go to the primary, and naming a machine would be ignored. */
  instancesSupported: boolean;
  /** The project has a repository to clone and the viewer may join a clone
   *  of it to their own machines (see `sessionCanCloneJoin`). */
  canClone: boolean;
  /** The selected tab: a new tab defaults to its machine. */
  selectedPane?: (Pick<PaneConfig, "machine_id" | "hostname"> & { pane_id: number }) | null;
  /** Tried after the selected tab and the primary instance, for example the
   *  project view's resolved machine. */
  fallbackMachineId?: string;
  /** The server negotiated `project_open_state_v1`: a tab on a machine that
   *  is not running the project opens the project first. */
  openStateSupported?: boolean;
  /** The project's machines and their parts
   *  (`buildProjectOpenModel(...).machines`): which other tab owners an Open
   *  starts. May be omitted; no other machine is then named. */
  projectMachines?: {
    machineId?: string;
    hostname: string;
    part: string;
    ownsTabs: boolean;
    connected: boolean;
  }[];
}

export interface MachineChoiceModel {
  /** Attached instances, then machines where the project is registered, then
   *  owned machines to clone onto; disabled choices last, with reasons. */
  choices: MachineChoice[];
  /** The preselected choice. Never a clone, which always needs the user to
   *  pick it and confirm. */
  defaultChoice?: MachineChoice;
  /** Whether to show the machine step. False means the new-tab flow is
   *  unchanged: `defaultChoice` is then a running instance (or absent), and
   *  its machine is still passed to `addPane` so the tab cannot land on
   *  another machine. A default that would start the project somewhere is
   *  never hidden; see `buildMachineChoices`. */
  showChoice: boolean;
}

export const MACHINE_OFFLINE_REASON = "offline";
export const NO_RUNTIME_ACCESS_REASON = "no runtime access";

export function rebootToUpdateReason(hostname: string): string {
  return `Reboot ${hostname} to update`;
}

/**
 * Whether the viewer can clone the project onto one of their machines as
 * another instance of it. Joining needs a repository to clone, and the server
 * allows it only on the owner path, which a project shared with the viewer
 * is not.
 */
export function sessionCanCloneJoin(session: { gitRemote?: string; isShared?: boolean } | undefined): boolean {
  return Boolean(session?.gitRemote) && session?.isShared !== true;
}

function byHostname(a: MachineWithProjects, b: MachineWithProjects): number {
  return a.machine.hostname.localeCompare(b.machine.hostname)
    || a.machine.machineId.localeCompare(b.machine.machineId);
}

export function buildMachineChoices(input: MachineChoiceInput): MachineChoiceModel {
  if (!input.instancesSupported) return { choices: [], showChoice: false };
  const { instances, machines } = input;

  // A CLI without multi-machine support owns the whole project: the server
  // refuses to start another machine's instance beside it until it updates.
  const legacy = instances.find((instance) => instanceIsLegacy(instance, machines));
  const orderedInstances = [...instances].sort((a, b) => Number(b.primary) - Number(a.primary));
  const instanceChoices: MachineChoice[] = orderedInstances.map((instance) => ({
    key: instance.machineId ?? `host:${instance.hostname}`,
    machineId: instance.machineId,
    hostname: instance.hostname,
    kind: "instance",
    primary: instance.primary,
    disabledReason: instance.runtimeAccess === false ? NO_RUNTIME_ACCESS_REASON : undefined,
    providers: machines.find((entry) => instance.machineId
      ? entry.machine.machineId === instance.machineId
      : sameHost(entry.machine.hostname, instance.hostname))?.machine.providers,
  }));

  const registered: MachineChoice[] = [];
  const clones: MachineChoice[] = [];
  for (const entry of [...machines].sort(byHostname)) {
    // Shared cluster machines take tabs only through an instance already
    // attached there; starting or cloning onto them is the owner's call.
    if (entry.clusterAccess === "member") continue;
    const { machineId, hostname } = entry.machine;
    const attached = instances.some((instance) => instance.machineId
      ? instance.machineId === machineId
      : sameHost(instance.hostname, hostname));
    if (attached) continue;
    const hasProject = entry.projects.some((project) => project.projectId === input.projectId);
    if (!hasProject && !input.canClone) continue;
    const disabledReason = legacy
      ? rebootToUpdateReason(legacy.hostname)
      : !entry.daemonConnected
        ? MACHINE_OFFLINE_REASON
        : !entry.projectInstancesAvailable
          ? rebootToUpdateReason(hostname)
          : undefined;
    (hasProject ? registered : clones).push({
      key: machineId,
      machineId,
      hostname,
      kind: hasProject ? "registered" : "clone",
      primary: false,
      disabledReason,
      providers: entry.machine.providers,
      // Opening the project starts every connected tab owner that is not
      // running it, not only the machine the tab goes to.
      ...(hasProject && input.openStateSupported ? {
        alsoStarts: (input.projectMachines ?? [])
          .filter((other) => other.ownsTabs && other.connected && other.part === "not_started"
            && !(other.machineId ? other.machineId === machineId : sameHost(other.hostname, hostname)))
          .map((other) => other.hostname)
          .sort((a, b) => a.localeCompare(b)),
      } : {}),
    });
  }

  const ordered = [...instanceChoices, ...registered, ...clones];
  const eligible = ordered.filter((choice) => !choice.disabledReason);
  const choices = [...eligible, ...ordered.filter((choice) => choice.disabledReason)];
  const defaultable = eligible.filter((choice) => choice.kind !== "clone");

  const choiceForMachine = (machineId: string | undefined) =>
    machineId ? defaultable.find((choice) => choice.machineId === machineId) : undefined;
  let defaultChoice: MachineChoice | undefined;
  if (input.selectedPane) {
    const owner = resolvePaneInstance(input.selectedPane, instances, machines);
    defaultChoice = owner && !owner.machineId
      ? defaultable.find((choice) => choice.kind === "instance" && !choice.machineId)
      : choiceForMachine(paneMachineId(input.selectedPane, instances, machines));
  }
  defaultChoice ??= defaultable.find((choice) => choice.primary)
    ?? choiceForMachine(input.fallbackMachineId)
    ?? defaultable[0];

  return {
    choices,
    defaultChoice,
    // One eligible machine leaves the flow unchanged only when it already
    // runs the project. A registered machine as the only choice (typically a
    // stopped single-machine project) would be started first, relaunching
    // its saved tabs and unpaused bots: the step is shown so the menu says
    // "Opens the project" before the click, as Open asks before the same
    // start. A clone is never a default, so it is always picked here.
    showChoice: eligible.length > 1 || (eligible.length === 1 && defaultChoice?.kind !== "instance"),
  };
}

/** Reads `MachineInfo.providers` leniently: an entry without a provider or
 *  its two flags is dropped, as the server drops one it cannot parse, and
 *  anything but a list is unknown. */
export function parseMachineProviders(value: unknown): MachineProviderStatus[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.flatMap((entry): MachineProviderStatus[] => {
    if (!entry || typeof entry !== "object") return [];
    const record = entry as Record<string, unknown>;
    if (typeof record.provider !== "string" || typeof record.installed !== "boolean" || typeof record.can_install !== "boolean") {
      return [];
    }
    return [{
      provider: record.provider,
      installed: record.installed,
      version: typeof record.version === "string" && record.version ? record.version : undefined,
      canInstall: record.can_install,
    }];
  });
}

// Names the daemon's provider values the way the add-tab menu does.
const PROVIDER_NAMES: Record<string, string> = Object.fromEntries(
  PROVIDER_MODEL_GROUPS.map((group) => [group.iconProvider, group.label]),
);
// The order the Machines page lists installed providers in: the menu's.
const PROVIDER_ORDER = PROVIDER_MODEL_GROUPS.map((group) => group.iconProvider);

function providerName(provider: string): string {
  return PROVIDER_NAMES[provider] ?? provider;
}

/** The binary a tab of `provider` runs. DeepSeek is the Claude binary
 *  against another backend, so a machine reports it as Claude. */
function launchedBinary(provider: string): string {
  return provider === "deepseek" ? "claude" : provider;
}

/** What the add-tab menu says about one provider on the machine that will
 *  receive the tab. Empty when the provider is installed or its state is not
 *  reported: an older daemon reports nothing, and the menu is then what it
 *  always was. */
export interface ProviderOffer {
  /** A short line under the provider, when picking it does more than open a
   *  tab. */
  note?: string;
  /** A longer explanation of the note, for its tooltip. */
  noteTitle?: string;
  /** Why the provider cannot run on that machine; the pick is disabled. */
  disabledReason?: string;
}

export function providerOfferOn(
  machine: Pick<MachineChoice, "hostname" | "providers"> | undefined,
  provider: string,
): ProviderOffer {
  // Only an explicit "not installed" changes anything. A machine that lists
  // other providers but not this one has said nothing about it.
  const status = machine?.providers?.find((entry) => entry.provider === launchedBinary(provider));
  if (!machine || !status || status.installed) return {};
  const name = providerName(status.provider);
  if (status.canInstall) {
    return {
      note: `Installs ${name} first`,
      noteTitle: `The tab installs ${name} on ${machine.hostname}, then starts it. You may need to log in to ${name} in its terminal.`,
    };
  }
  return { disabledReason: `${name} is not installed on ${machine.hostname}.` };
}

/** The installed providers a machine reports, for its Machines page row:
 *  `text` names them, `title` adds their versions. Null when the machine
 *  reports nothing (an older daemon). */
export function installedProvidersSummary(
  providers: MachineProviderStatus[] | undefined,
): { text: string; title: string } | null {
  if (!providers) return null;
  const installed = providers
    .filter((entry) => entry.installed)
    .sort((a, b) => rank(a.provider) - rank(b.provider) || a.provider.localeCompare(b.provider));
  if (installed.length === 0) return { text: "No providers installed", title: "No providers installed" };
  const names = installed.map((entry) => providerName(entry.provider));
  return {
    text: `Providers: ${names.join(", ")}`,
    title: installed
      .map((entry) => entry.version ? `${providerName(entry.provider)} ${entry.version}` : providerName(entry.provider))
      .join(", "),
  };
}

function rank(provider: string): number {
  const index = PROVIDER_ORDER.indexOf(provider);
  return index === -1 ? PROVIDER_ORDER.length : index;
}
