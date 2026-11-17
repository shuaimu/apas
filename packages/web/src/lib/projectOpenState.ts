import type { MachinePart, OpenStateSetKind } from "@apas/protocol";
import type { MachineWithProjects } from "./store";
import {
  projectPlacement,
  sameHost,
  type PaneMachineFields,
  type SessionInstance,
} from "./projectInstances";
import { hostList, stoppedOnHostReason, type ConfirmationCopy } from "./tabHosts";

/**
 * A project is open or closed as a whole. The server records that, starts
 * every machine that owns tabs of an open project and stops every machine
 * that runs a closed one. This module turns what the server reports into the
 * one model the project view, the Machines page and the phone render: the
 * state to show, the actions to offer, what each confirmation says and which
 * machines are not following.
 *
 * It is pure. The store holds the inputs (`projectOpenStates`, the session
 * and machine lists); see `ProjectOpenModelInput`.
 */

/**
 * Mirrors `shared::PROJECT_OPEN_STATE_CAPABILITY`. Advertised in
 * `Authenticate.capabilities`; a server that records open states echoes it
 * in `negotiated_capabilities` and only then sends `project_open_state`.
 * Without the echo the server is older (or rolled back): a project is then
 * shown from what runs.
 */
export const PROJECT_OPEN_STATE_CAPABILITY = "project_open_state_v1";

/** A recorded state. "Not recorded" is the absence of a value, never one. */
export type OpenState = "open" | "closed";

export type { MachinePart, OpenStateSetKind };

/** One machine's part in a project; mirrors `shared::ProjectMachinePart`. */
export interface ProjectMachinePartInfo {
  /** Absent for a host the server knows only by name. */
  machineId?: string;
  hostname: string;
  part: MachinePart;
  /** The server's sentence for why the machine is not started or still
   *  runs. Shown as it is. */
  reason?: string;
  ownsTabs: boolean;
  connected: boolean;
  /**
   * Set here, never by the server: no frame is held and the machine has no
   * entry of its own, so its part was worked out from another entry's
   * `other_hosts`, which says only whether the machine runs the project. A
   * connected tab owner of an open project that is not running it may have
   * been asked to start, or may have failed to: `ProjectHostSummary` cannot
   * tell the two apart. Such a part reads "Not running yet", and the machine
   * is not counted among those an Open starts.
   */
  partUnconfirmed?: boolean;
}

/** Another machine of a project, as a machine-list entry names it; mirrors
 *  `shared::ProjectHostSummary`. */
export interface ProjectHostInfo {
  hostname: string;
  connected: boolean;
  running: boolean;
  ownsTabs: boolean;
}

/** The latest `project_open_state` frame of one project. */
export interface ProjectOpenReport {
  projectId: string;
  /** Absent while the project is not recorded. */
  state?: OpenState;
  /** The email of the account that set the state. */
  setBy?: string;
  setKind?: OpenStateSetKind;
  /** RFC3339, UTC. */
  setAt?: string;
  machines: ProjectMachinePartInfo[];
}

const MACHINE_PARTS: readonly MachinePart[] = [
  "running",
  "starting",
  "waiting_offline",
  "not_started",
  "still_running",
];

/** `undefined` for anything but the two recorded values. */
export function parseOpenState(value: unknown): OpenState | undefined {
  return value === "open" || value === "closed" ? value : undefined;
}

export function parseMachinePart(value: unknown): MachinePart | undefined {
  return MACHINE_PARTS.includes(value as MachinePart) ? (value as MachinePart) : undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value ? value : undefined;
}

/** Reads `MachineProjectInfo.other_hosts`; entries without a host name are
 *  dropped. `undefined` when the field is absent or empty. */
export function parseProjectHosts(value: unknown): ProjectHostInfo[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const hosts = value.flatMap((item): ProjectHostInfo[] => {
    if (!item || typeof item !== "object") return [];
    const record = item as Record<string, unknown>;
    const hostname = text(record.hostname);
    if (!hostname) return [];
    return [{
      hostname,
      connected: record.connected === true,
      running: record.running === true,
      ownsTabs: record.owns_tabs === true,
    }];
  });
  return hosts.length > 0 ? hosts : undefined;
}

/**
 * Reads one `project_open_state` frame. `null` for a frame without a project
 * id or with a state outside the vocabulary; a machine whose part is outside
 * it is left out. A frame without a state is a project nobody has recorded.
 */
export function parseProjectOpenStateFrame(data: Record<string, unknown>): ProjectOpenReport | null {
  const projectId = text(data.project_id);
  if (!projectId) return null;
  if (data.state != null && parseOpenState(data.state) === undefined) return null;
  const machines = Array.isArray(data.machines) ? data.machines : [];
  return {
    projectId,
    state: parseOpenState(data.state),
    setBy: text(data.set_by),
    setKind: data.set_kind === "request" || data.set_kind === "administrative" ? data.set_kind : undefined,
    setAt: text(data.set_at),
    machines: machines.flatMap((item): ProjectMachinePartInfo[] => {
      if (!item || typeof item !== "object") return [];
      const record = item as Record<string, unknown>;
      const hostname = text(record.hostname);
      const part = parseMachinePart(record.part);
      if (!hostname || !part) return [];
      return [{
        machineId: text(record.machine_id),
        hostname,
        part,
        reason: text(record.reason),
        ownsTabs: record.owns_tabs === true,
        connected: record.connected === true,
      }];
    }),
  };
}

/** Whether the server records open states (it echoed the capability). */
export function openStateNegotiated(state: { negotiatedCapabilities: Set<string> }): boolean {
  return state.negotiatedCapabilities.has(PROJECT_OPEN_STATE_CAPABILITY);
}

/**
 * The latest `project_open_state` frame held for a project, under the first
 * of `projectIds` that has one. Safe as a store selector: it returns the
 * stored object.
 */
export function selectProjectOpenReport(
  state: { projectOpenStates: Record<string, ProjectOpenReport> },
  projectIds: string | null | undefined | (string | null | undefined)[],
): ProjectOpenReport | undefined {
  for (const id of Array.isArray(projectIds) ? projectIds : [projectIds]) {
    if (id && state.projectOpenStates[id]) return state.projectOpenStates[id];
  }
  return undefined;
}

/** The last line of both Close confirmations (design Decision 12). */
export const CLOSE_DELETES_NOTHING = "Nothing is deleted. Its tabs come back when you open it again.";

/** In an operator's or administrator's "Stop runtime" confirmation: the
 *  server records the project closed (design Decision 9), so no machine
 *  starts it again by itself. */
export const STOP_RUNTIME_CLOSES_PROJECT =
  "This closes the project. Its machines do not start it again until someone opens it.";

/** In a suspension's confirmation: a suspended project is recorded closed,
 *  and reactivating it starts nothing. */
export const SUSPEND_CLOSES_PROJECT =
  "This also closes the project. It stays closed after it is reactivated, until someone opens it.";

/** What a control says when its request could not be sent. */
export const NOTHING_SENT_TEXT = "Not connected to the server. Nothing was sent.";

/**
 * The server's sentence, word for word, for a machine it does not start
 * because its daemon cannot run a project beside other machines (design
 * Decision 4). The server gives it once it has refused that machine; the
 * model says it before, from the machine list's own report of the daemon
 * (`ProjectOpenMachine.cannotStart`).
 */
export function olderApasReason(hostname: string): string {
  return `${hostname} runs an older APAS that cannot run this project beside other machines. Reboot ${hostname} to update it.`;
}

/** "Open" or "Closed": the project's state as every list shows it. */
export function openStateLabel(state: OpenState): "Open" | "Closed" {
  return state === "open" ? "Open" : "Closed";
}

/**
 * How one machine's part reads in a list.
 *
 * `not_started` is worded by the project's state: in an open project, or
 * with a reason, the machine failed or refused ("Not started: <reason>"). A
 * tab owner that does not run a closed project, or any project of a server
 * that records no state (`supported` false), has not failed at anything and
 * reads "Not running". Where the server did not state the part of an open
 * project's machine (`partUnconfirmed`), it may be starting or may have
 * failed, and reads "Not running yet".
 *
 * `part` absent means the machine has no part: it owns no tabs of the
 * project and does not run it.
 */
export function machinePartLabel(
  machine: { part?: MachinePart; hostname: string; reason?: string; partUnconfirmed?: boolean },
  project: { state: OpenState; supported: boolean },
): string {
  switch (machine.part) {
    case undefined:
      return "No tabs here";
    case "running":
      return "Running";
    case "starting":
      return "Starting…";
    case "waiting_offline":
      return `Waiting for ${machine.hostname} (offline)`;
    case "still_running":
      return "Still running";
    case "not_started":
      if (machine.reason) return `Not started: ${machine.reason}`;
      if (!(project.supported && project.state === "open")) return "Not running";
      return machine.partUnconfirmed ? "Not running yet" : "Not started";
  }
}

/** One machine in the model: its part, and how the part reads. */
export interface ProjectOpenMachine extends ProjectMachinePartInfo {
  /** Stable key: the machine id, else `host:<hostname>`. */
  key: string;
  /** `machinePartLabel` of this machine. */
  label: string;
  /**
   * Why an Open cannot start this machine although it is connected, owns
   * tabs and is not running the project: its daemon lacks
   * `project_instances_v1` (the machine list marks it "Older APAS") while
   * another machine owns tabs of the project or runs it. The server sends
   * such a machine nothing, every time, until it is rebooted to update
   * (design Decision 4). The sentence is the server's (`olderApasReason`).
   *
   * Taken from the viewer's machine list. A machine that list does not have
   * is assumed startable: the server decides.
   */
  cannotStart?: string;
}

export type ProjectOpenActionKind = "open" | "open_again";
export type ProjectCloseActionKind = "close" | "close_again";

export interface ProjectOpenAction {
  /**
   * `open`: the project is closed.
   * `open_again`: it is open and the request can start a connected machine.
   * Both send the same request (`openProject`).
   */
  kind: ProjectOpenActionKind;
  /** "Open" or "Open again": the button and the confirm button. */
  label: string;
  confirmation: ConfirmationCopy;
}

export interface ProjectCloseAction {
  /** `close_again`: the project is closed and a machine still runs it. Both
   *  send the same request (`closeProject`). */
  kind: ProjectCloseActionKind;
  /** "Close" or "Close again". */
  label: string;
  /** Its last body line is `CLOSE_DELETES_NOTHING`; put any extra line (the
   *  tabs on autopilot, for example) before it with `withLinesBeforeLast`. */
  confirmation: ConfirmationCopy;
}

/** One line of the notice about machines that are not in line with the
 *  project's state. */
export interface ProjectOpenNoticeLine {
  key: string;
  machineId?: string;
  hostname: string;
  /**
   * `not_started`: connected and not running an open (or half-running)
   * project; Open again can start it, unless its daemon is too old to run
   * the project beside the others (`reason` then says so, and no Open is
   * offered for it). `not_running_yet`: connected and not
   * running an open project, where the server did not say whether it is
   * starting or failed (`partUnconfirmed`); nothing is offered for it.
   * `offline`: not running it because it is offline, and nothing starts it
   * when it returns. `waiting_offline`: offline, and started when it
   * reconnects. `starting`: asked to start. `still_running`: runs a closed
   * project.
   */
  kind: "not_started" | "not_running_yet" | "offline" | "waiting_offline" | "starting" | "still_running";
  /** The sentence that names the machine. */
  text: string;
  /** The server's reason, as it gave it; else why an Open cannot start the
   *  machine (`ProjectOpenMachine.cannotStart`). Shown after `text`. */
  reason?: string;
}

/** The entry's own machine, for a row of the Machines page. */
export interface ProjectOpenThisMachine {
  machineId: string;
  hostname: string;
  /** Absent: this machine has no part in the project. */
  part?: MachinePart;
  /** "Running", "Starting…", "Not started: <reason>", "No tabs here", …
   *  From an older server, or for an entry the server filled nothing in,
   *  "Running" or "Not running". */
  label: string;
  reason?: string;
  ownsTabs: boolean;
  running: boolean;
}

export interface ProjectOpenModel {
  /** The project's state. Where the server has recorded none, and on a
   *  server without the capability: open while a machine runs it, else
   *  closed. */
  state: OpenState;
  /** The server records open states (`ProjectOpenModelInput.supported`).
   *  Only then is `state` the project's state rather than a reading of what
   *  runs; nothing shown tells the two apart beyond that. */
  supported: boolean;
  /** "Open" or "Closed". */
  stateLabel: "Open" | "Closed";
  /** Who set a recorded state, when the server said. */
  setBy?: string;
  setKind?: OpenStateSetKind;
  setAt?: string;
  /** Every machine with a part: it owns tabs of the project or runs it.
   *  Sorted by host name. */
  machines: ProjectOpenMachine[];
  /** Hosts that run the project now. */
  runningHosts: string[];
  /** Hosts that own tabs of it, online or not. */
  tabOwnerHosts: string[];
  /** Offline hosts that own tabs: a deletion cannot stop them. */
  offlineTabOwnerHosts: string[];
  /** Connected hosts that run it: a deletion stops their agents first. */
  stoppedFirstHosts: string[];
  /** At most one way to open; absent when there is nothing an Open would
   *  do, or no machine to name in the request (`request` is null). */
  openAction?: ProjectOpenAction;
  closeAction?: ProjectCloseAction;
  /** Machines that are not in line, each with its reason. Empty when every
   *  machine follows. `openAction` or `closeAction` (`close_again`) is the
   *  control that can change what it says, where one is offered. */
  notice: ProjectOpenNoticeLine[];
  /** What `openProject` / `closeProject` take: the input's `target`, or,
   *  when no machine-list entry lists the project, a machine the held frame
   *  names (see `reportTarget`). `null` when neither names a machine, and
   *  then no action is offered. */
  request: { machineId: string; projectId: string } | null;
  /** Present when `fromMachineId` was given and that machine is known. */
  thisMachine?: ProjectOpenThisMachine;
  /** Only the project's owner is offered "Delete project…". */
  canDelete: boolean;
}

export interface ProjectOpenModelInput {
  /** `openStateNegotiated(state)`. */
  supported: boolean;
  /** Ids the project may be listed under, the stable project id first
   *  (`[session.projectId, session.id]`, or the Machines row's id). */
  projectIds: (string | null | undefined)[];
  /** Used in confirmation titles; "this project" when absent. */
  projectName?: string;
  /** `selectProjectOpenReport(state, projectIds)`. */
  report?: ProjectOpenReport;
  /** The project's session-list entry, when it has one. */
  session?: { openState?: OpenState; isActive?: boolean } | null;
  /** The roster, where it is known (the attached project view). Used only
   *  when the server reported nothing about the machines. */
  panes?: (PaneMachineFields & { pane_id: number })[];
  /** The attached instances, where known. */
  instances?: SessionInstance[];
  machines: MachineWithProjects[];
  /** The machine and project id the requests name: the answer of
   *  `resolveMachineProjectTarget`, or the Machines row's own machine. When
   *  it is absent and no machine-list entry lists the project, the held
   *  frame's machines supply one (`reportTarget`). */
  target?: { machineId: string; projectId: string; hostname?: string } | null;
  /** The Machines page: the machine whose entry the controls sit in. */
  fromMachineId?: string;
  /** `sessionIsOwned(session)`, or the cluster panel's owner check. */
  isOwner?: boolean;
}

/** Inserts lines before a confirmation's last line, which stays last. */
export function withLinesBeforeLast(copy: ConfirmationCopy, lines: string[]): ConfirmationCopy {
  if (lines.length === 0) return copy;
  return { title: copy.title, body: [...copy.body.slice(0, -1), ...lines, ...copy.body.slice(-1)] };
}

function machineKey(machineId: string | undefined, hostname: string): string {
  return machineId ?? `host:${hostname.trim().toLowerCase()}`;
}

function runs(machine: { part: MachinePart }): boolean {
  return machine.part === "running" || machine.part === "still_running";
}

function verb(hosts: string[], singular: string, plural: string): string {
  return hosts.length === 1 ? singular : plural;
}

interface Sources {
  recordedState?: OpenState;
  setBy?: string;
  setKind?: OpenStateSetKind;
  setAt?: string;
  machines: ProjectMachinePartInfo[];
  /** The server filled in the machines' parts (a frame or the machine
   *  list). False when they were derived here from what runs. */
  serverParts: boolean;
}

/** Machine-list entries that list the project, the named entry first. */
function listingEntries(input: ProjectOpenModelInput, ids: string[]) {
  const entries = input.machines.flatMap((entry) => {
    const project = ids
      .map((id) => entry.projects.find((candidate) => candidate.projectId === id))
      .find(Boolean);
    return project ? [{ entry, project }] : [];
  });
  return entries.sort((a, b) =>
    Number(b.entry.machine.machineId === input.fromMachineId)
    - Number(a.entry.machine.machineId === input.fromMachineId));
}

function resolveSources(input: ProjectOpenModelInput, ids: string[]): Sources {
  const entries = listingEntries(input, ids);
  if (!input.supported) return { machines: derivedParts(input, entries), serverParts: false };

  // The lists are polled every few seconds and a frame is applied to them
  // when it arrives, so they are never older than the frame held. A frame
  // whose state differs from theirs was superseded while this viewer was
  // not sent the change; it is not used.
  const polledKnown = Boolean(input.session) || entries.length > 0;
  const polledState = input.session
    ? input.session.openState
    : entries.find(({ project }) => project.openState)?.project.openState;
  const report = input.report && (!polledKnown || input.report.state === polledState)
    ? input.report
    : undefined;
  if (report) {
    return {
      recordedState: report.state,
      setBy: report.setBy,
      setKind: report.setKind,
      setAt: report.setAt,
      machines: report.machines,
      serverParts: true,
    };
  }
  if (entries.length === 0) {
    return { recordedState: polledState, machines: derivedParts(input, entries), serverParts: false };
  }

  // No frame: the machine list's own entries carry each listed machine's
  // part and name the others.
  const machines: ProjectMachinePartInfo[] = [];
  const known = (hostname: string) => machines.some((machine) => sameHost(machine.hostname, hostname));
  for (const { entry, project } of entries) {
    // A machine whose inventory says it runs the project runs it, also where
    // the server filled in no part: an entry for a project this viewer holds
    // no role in carries none.
    const part: MachinePart | undefined = project.part
      ?? (project.isRunning ? (polledState === "closed" ? "still_running" : "running") : undefined);
    if (!part) continue;
    machines.push({
      machineId: entry.machine.machineId,
      hostname: entry.machine.hostname,
      part,
      reason: project.partReason,
      // A part that waits or did not start belongs to a tab owner. For a
      // machine that runs the project the entry does not say; another
      // entry's `otherHosts` may.
      ownsTabs: part === "waiting_offline" || part === "not_started"
        || (entries.flatMap((other) => other.project.otherHosts ?? [])
          .find((host) => sameHost(host.hostname, entry.machine.hostname))?.ownsTabs ?? true),
      connected: entry.daemonConnected === true,
    });
  }
  for (const { entry, project } of entries) {
    for (const host of project.otherHosts ?? []) {
      if (known(host.hostname) || sameHost(host.hostname, entry.machine.hostname)) continue;
      // A machine this viewer lists that has no part is not in `machines`,
      // and must not come back through another entry's list.
      if (entries.some((other) => sameHost(other.entry.machine.hostname, host.hostname))) continue;
      if (!host.running && !host.ownsTabs) continue;
      const part: MachinePart = host.running
        ? (polledState === "closed" ? "still_running" : "running")
        : polledState === "open" && !host.connected ? "waiting_offline" : "not_started";
      machines.push({
        hostname: host.hostname,
        part,
        ownsTabs: host.ownsTabs,
        connected: host.connected,
        // `other_hosts` says "running" only of a machine that runs the
        // project. A connected tab owner of an open project that is not
        // running it may be starting right now; the server did not say.
        ...(part === "not_started" && polledState === "open" ? { partUnconfirmed: true } : {}),
      });
    }
  }
  return { recordedState: polledState, machines, serverParts: true };
}

/** What the viewer can tell by itself, for a server that reports nothing:
 *  a machine runs the project when an instance is attached there or its
 *  inventory says so, and owns tabs when the roster places some on it. */
function derivedParts(
  input: ProjectOpenModelInput,
  entries: ReturnType<typeof listingEntries>,
): ProjectMachinePartInfo[] {
  const instances = input.instances ?? [];
  const placement = projectPlacement(input.panes ?? [], instances, input.machines);
  const machines: ProjectMachinePartInfo[] = [];
  const entryFor = (machineId: string | undefined, hostname: string | undefined) => entries.find(({ entry }) =>
    machineId ? entry.machine.machineId === machineId : sameHost(entry.machine.hostname, hostname));
  for (const state of placement.machines) {
    if (!state.hostname) continue;
    const listed = entryFor(state.machineId, state.hostname);
    const running = state.attached || listed?.project.isRunning === true;
    if (!running && state.paneIds.length === 0) continue;
    const known = state.machineId
      ? input.machines.find((entry) => entry.machine.machineId === state.machineId)
      : undefined;
    machines.push({
      machineId: state.machineId,
      hostname: state.hostname,
      part: running ? "running" : "not_started",
      ownsTabs: state.paneIds.length > 0,
      // A machine this viewer does not list may well be connected; the
      // server decides when asked.
      connected: state.attached || (known ? known.daemonConnected === true : true),
    });
  }
  for (const { entry, project } of entries) {
    if (!project.isRunning) continue;
    if (machines.some((machine) => machine.machineId
      ? machine.machineId === entry.machine.machineId
      : sameHost(machine.hostname, entry.machine.hostname))) continue;
    machines.push({
      machineId: entry.machine.machineId,
      hostname: entry.machine.hostname,
      part: "running",
      ownsTabs: false,
      connected: entry.daemonConnected === true,
    });
  }
  return machines;
}

/**
 * The machine a request names when the viewer's machine list has no entry
 * for the project. The server keeps its machine list in memory, so after a
 * server restart a machine that has not registered again is in no list,
 * while the frame still names the project's machines. An Open or a Close is
 * for the whole project and the server accepts it without that machine
 * being connected, so any machine the frame names by id will do: a
 * connected tab owner first, since an Open starts that one.
 *
 * `null` when the frame names no machine by id, as for a roster of untagged
 * tabs only.
 */
function reportTarget(
  report: ProjectOpenReport,
  projectId: string | undefined,
): { machineId: string; projectId: string; hostname: string } | null {
  if (!projectId) return null;
  const named = report.machines.filter((machine) => machine.machineId);
  const machine = named.find((candidate) => candidate.connected && candidate.ownsTabs) ?? named[0];
  return machine?.machineId
    ? { machineId: machine.machineId, projectId, hostname: machine.hostname }
    : null;
}

function offlineStartLines(hosts: string[], supported: boolean): string[] {
  if (hosts.length === 0) return [];
  if (!supported) {
    return [`${hostList(hosts)} ${verb(hosts, "is", "are")} offline and ${verb(hosts, "is", "are")} not started.`];
  }
  return hosts.length === 1
    ? [`${hosts[0]} is offline. Its tabs start when ${hosts[0]} reconnects.`]
    : [`${hostList(hosts)} are offline. Their tabs start when each reconnects.`];
}

function offlineStopLines(hosts: string[], supported: boolean): string[] {
  if (hosts.length === 0) return [];
  if (!supported) {
    return [`${hostList(hosts)} ${verb(hosts, "is", "are")} offline, so this does not reach ${verb(hosts, "it", "them")}.`];
  }
  return hosts.length === 1
    ? [`${hosts[0]} is offline. If it still runs the project when it reconnects, its agents are stopped then.`]
    : [`${hostList(hosts)} are offline. If one still runs the project when it reconnects, its agents are stopped then.`];
}

/**
 * The one model of a project's open state. See `ProjectOpenModel` for what
 * each part means and `ProjectOpenModelInput` for where the inputs come
 * from.
 */
export function buildProjectOpenModel(input: ProjectOpenModelInput): ProjectOpenModel {
  const ids = input.projectIds.filter((id): id is string => Boolean(id));
  const sources = resolveSources(input, ids);
  const name = input.projectName?.trim() || "this project";

  const anyActive = sources.machines.some((machine) => runs(machine) || machine.part === "starting")
    // With nothing reported per machine, an active session is all that says
    // the project runs.
    || (!sources.serverParts && input.session?.isActive === true);
  const state: OpenState = sources.recordedState ?? (anyActive ? "open" : "closed");
  const project = { state, supported: input.supported };

  const keyed: ProjectOpenMachine[] = sources.machines
    .map((machine) => ({
      ...machine,
      key: machineKey(machine.machineId, machine.hostname),
      label: machinePartLabel(machine, project),
    }))
    .sort((a, b) => a.hostname.localeCompare(b.hostname) || a.key.localeCompare(b.key));
  // A connected tab owner whose daemon cannot run a project beside other
  // machines is sent nothing by an Open while another machine owns tabs of
  // the project or runs it (design Decision 4). The viewer's machine list
  // says which daemons those are; a machine it does not list is left to the
  // server.
  const machines = keyed.map((machine): ProjectOpenMachine => {
    if (machine.part !== "not_started" || !machine.ownsTabs || !machine.connected) return machine;
    const listed = input.machines.filter((entry) => machine.machineId
      ? entry.machine.machineId === machine.machineId
      : sameHost(entry.machine.hostname, machine.hostname));
    const older = listed.length === 1
      && listed[0].daemonConnected === true
      && listed[0].projectInstancesAvailable === false;
    const beside = keyed.some((other) => other.key !== machine.key
      && (other.ownsTabs || runs(other) || other.part === "starting"));
    if (!older || !beside) return machine;
    const cannotStart = olderApasReason(machine.hostname);
    // In an open project the machine reads "Not started" already; say why
    // also where the server has not (it forgets its reasons at a restart).
    return {
      ...machine,
      cannotStart,
      label: machinePartLabel({
        ...machine,
        reason: machine.reason ?? (input.supported && state === "open" ? cannotStart : undefined),
      }, project),
    };
  });

  const hostsOf = (list: ProjectOpenMachine[]) => list.map((machine) => machine.hostname);
  const owners = machines.filter((machine) => machine.ownsTabs);
  const running = machines.filter(runs);
  const starting = machines.filter((machine) => machine.part === "starting");
  const idleOwners = owners.filter((machine) =>
    machine.part === "not_started" || machine.part === "waiting_offline");
  /** The tab owners an Open starts now. A machine whose part the server
   *  did not state may be starting already, and is not counted. Neither is
   *  one the server will not start (`cannotStart`). */
  const startable = idleOwners.filter((machine) =>
    machine.connected && !machine.partUnconfirmed && !machine.cannotStart);
  /** Connected tab owners an Open leaves as they are, each with its reason:
   *  "zoo-006 is not started. <reason>". */
  const notStartedLines = idleOwners
    .filter((machine) => machine.connected && machine.cannotStart)
    .map((machine) => `${machine.hostname} is not started. ${machine.reason ?? machine.cannotStart}`);
  const offlineOwners = idleOwners.filter((machine) => !machine.connected);

  const target = input.target
    ?? (input.supported && input.report && listingEntries(input, ids).length === 0
      ? reportTarget(input.report, ids[0])
      : null);
  const targetEntry = target
    ? input.machines.find((entry) => entry.machine.machineId === target.machineId)
    : undefined;
  const targetHost = target?.hostname ?? targetEntry?.machine.hostname;
  // With no tab owner an Open starts the machine the request names.
  const targetStartable = owners.length === 0 && running.length === 0 && starting.length === 0
    && (targetEntry ? targetEntry.daemonConnected === true : true);

  const fromEntry = input.fromMachineId
    ? input.machines.find((entry) => entry.machine.machineId === input.fromMachineId)
    : undefined;
  const fromMachine = fromEntry
    ? machines.find((machine) => machine.machineId
      ? machine.machineId === fromEntry.machine.machineId
      : sameHost(machine.hostname, fromEntry.machine.hostname))
    : undefined;
  // Who owns tabs is known from the server's parts or from the roster. An
  // older server asked from a machine list alone tells neither.
  const ownersKnown = sources.serverParts || input.panes !== undefined;
  /** The entry the controls sit in owns none of the project's tabs. */
  const fromHasNoTabs = Boolean(fromEntry) && !fromMachine?.ownsTabs && ownersKnown;

  // --- Open ---------------------------------------------------------------
  // One button opens or closes the project: Open while it is closed, Close
  // while it is open. "Open again" is offered beside Close only where the
  // project is shown open and a connected machine is not following it.
  let openKind: ProjectOpenActionKind | undefined;
  if (target) {
    if (state === "closed") {
      openKind = "open";
    } else if (input.supported) {
      // Also for a running project the server has not recorded yet: it is
      // shown open, its stopped tabs say "Open the project again", and an
      // Open of it is an ordinary open, which the server records.
      if (startable.length > 0 || targetStartable) openKind = "open_again";
    } else if (startable.length > 0) {
      // An older server records nothing: an Open is worth offering only
      // while it can start a machine.
      openKind = "open_again";
    }
  }

  let openAction: ProjectOpenAction | undefined;
  if (openKind) {
    const body: string[] = [];
    if (running.length > 0) {
      body.push(`${hostList(hostsOf(running))} already ${verb(hostsOf(running), "runs", "run")} it.`);
    }
    if (startable.length > 0) {
      body.push(fromHasNoTabs && fromEntry
        ? `It starts on ${hostList(hostsOf(startable))} and not on ${fromEntry.machine.hostname}, which has no tabs of it.`
        : `Its tabs start on ${hostList(hostsOf(startable))}.`);
    } else if (owners.length === 0 && running.length === 0 && !ownersKnown) {
      // What the request does, which is all that can be said without
      // knowing which machines own tabs.
      body.push(`It starts on each online machine that has its tabs${targetHost ? `, or on ${targetHost} when none has` : ""}.`);
    } else if (owners.length === 0 && running.length === 0) {
      if (targetHost && targetEntry && !targetEntry.daemonConnected) {
        body.push(input.supported
          ? `${targetHost} is offline. It starts there when ${targetHost} reconnects.`
          : `${targetHost} is offline and is not started.`);
      } else {
        body.push(targetHost
          ? `It starts on ${targetHost}.`
          : "It starts on the machine it is registered on.");
      }
    } else if (!input.supported && running.length === 0 && starting.length === 0
      && targetHost && targetEntry?.daemonConnected === true) {
      // A server that records no state starts the machine the request names
      // when no machine with the project's tabs is online.
      body.push(`It starts on ${targetHost}.`);
    } else if (fromHasNoTabs && fromEntry) {
      body.push(`Nothing starts on ${fromEntry.machine.hostname}, which has no tabs of it.`);
    }
    body.push(...notStartedLines);
    body.push(...offlineStartLines(hostsOf(offlineOwners), input.supported));
    if (body.length === 0) body.push("This records the project as open.");
    openAction = {
      kind: openKind,
      label: openKind === "open_again" ? "Open again" : "Open",
      confirmation: {
        title: openKind === "open_again" ? `Open ${name} again?` : `Open ${name}?`,
        body,
      },
    };
  }

  // --- Close --------------------------------------------------------------
  let closeKind: ProjectCloseActionKind | undefined;
  if (target) {
    if (state === "open") {
      closeKind = "close";
    } else if (running.length > 0 || starting.length > 0) {
      closeKind = "close_again";
    }
  }
  let closeAction: ProjectCloseAction | undefined;
  if (closeKind) {
    // A close stops the connected machines that run the project or were
    // asked to start it. A connected tab owner that is not running it has
    // nothing to stop and is not named.
    const reached = machines.filter((machine) =>
      machine.connected && (runs(machine) || machine.part === "starting"));
    const unreached = machines.filter((machine) => !machine.connected);
    const body: string[] = [];
    if (closeKind === "close_again" && running.length > 0) {
      body.push(`${hostList(hostsOf(running))} still ${verb(hostsOf(running), "runs", "run")} this project.`);
    }
    // Without the server's parts, the attached instances are what says
    // which machines run the project. A machine list alone shows only this
    // viewer's machines.
    const reachedKnown = sources.serverParts || input.instances !== undefined;
    body.push(reached.length > 0 && reachedKnown
      ? `This stops its agents on ${hostList(hostsOf(reached))}.`
      : "This stops its agents on every machine that runs it.");
    body.push(...offlineStopLines(hostsOf(unreached), input.supported));
    body.push(CLOSE_DELETES_NOTHING);
    closeAction = {
      kind: closeKind,
      label: closeKind === "close_again" ? "Close again" : "Close",
      confirmation: {
        title: closeKind === "close_again" ? `Close ${name} again?` : `Close ${name}?`,
        body,
      },
    };
  }

  // --- Notice -------------------------------------------------------------
  const notice: ProjectOpenNoticeLine[] = [];
  for (const machine of machines) {
    const line = (kind: ProjectOpenNoticeLine["kind"], sentence: string) => notice.push({
      key: machine.key,
      machineId: machine.machineId,
      hostname: machine.hostname,
      kind,
      text: sentence,
      reason: machine.reason ?? machine.cannotStart,
    });
    if (state === "closed") {
      if (machine.part === "still_running") {
        line("still_running", `${machine.hostname} still runs this project.`);
      }
      continue;
    }
    if (!machine.ownsTabs) continue;
    if (machine.part === "waiting_offline") {
      line("waiting_offline", `${machine.hostname} is offline. Its tabs start when ${machine.hostname} reconnects.`);
    } else if (machine.part === "starting") {
      line("starting", `${machine.hostname} is starting this project.`);
    } else if (machine.part === "not_started") {
      if (!machine.connected) line("offline", stoppedOnHostReason(machine.hostname, true));
      else if (machine.partUnconfirmed) line("not_running_yet", `${machine.hostname} is not running this project yet.`);
      else line("not_started", `${machine.hostname} is not running this project.`);
    }
  }

  // --- The entry's own machine --------------------------------------------
  let thisMachine: ProjectOpenThisMachine | undefined;
  if (fromEntry) {
    const listed = fromEntry.projects.find((candidate) => ids.includes(candidate.projectId));
    const isRunning = fromMachine ? runs(fromMachine) : listed?.isRunning === true;
    // "No tabs here" is a statement about the roster. Without a part, say
    // it only where the server filled the entry in; a shared machine's
    // entry for a project this viewer may not read carries nothing.
    const serverSaidNoPart = sources.serverParts && (
      fromEntry.clusterAccess !== "member"
      || listed?.openState !== undefined
      || (listed?.otherHosts?.length ?? 0) > 0);
    thisMachine = {
      machineId: fromEntry.machine.machineId,
      hostname: fromEntry.machine.hostname,
      part: fromMachine?.part,
      label: fromMachine
        ? fromMachine.label
        : serverSaidNoPart ? machinePartLabel({ hostname: fromEntry.machine.hostname }, project)
          : isRunning ? "Running" : "Not running",
      reason: fromMachine?.reason
        ?? (input.supported && state === "open" ? fromMachine?.cannotStart : undefined),
      ownsTabs: fromMachine?.ownsTabs === true,
      running: isRunning,
    };
  }

  return {
    state,
    supported: input.supported,
    stateLabel: openStateLabel(state),
    setBy: sources.setBy,
    setKind: sources.setKind,
    setAt: sources.setAt,
    machines,
    runningHosts: hostsOf(running),
    tabOwnerHosts: hostsOf(owners),
    offlineTabOwnerHosts: hostsOf(owners.filter((machine) => !machine.connected)),
    stoppedFirstHosts: hostsOf(machines.filter((machine) =>
      machine.connected && (runs(machine) || machine.part === "starting"))),
    openAction,
    closeAction,
    notice,
    request: target ? { machineId: target.machineId, projectId: target.projectId } : null,
    thisMachine,
    canDelete: input.isOwner === true,
  };
}
