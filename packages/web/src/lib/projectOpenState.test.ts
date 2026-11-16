import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { MachineProject, MachineWithProjects } from "./store";
import type { SessionInstance } from "./projectInstances";
import { stoppedOnHostReason } from "./tabHosts";
import {
  buildProjectOpenModel,
  CLOSE_DELETES_NOTHING,
  keepOpenNote,
  machinePartLabel,
  NOTHING_SENT_TEXT,
  olderApasReason,
  openStateNegotiated,
  parseMachinePart,
  parseOpenState,
  parseProjectHosts,
  parseProjectOpenStateFrame,
  PROJECT_OPEN_STATE_CAPABILITY,
  selectProjectOpenReport,
  withLinesBeforeLast,
  type ProjectMachinePartInfo,
  type ProjectOpenModelInput,
  type ProjectOpenReport,
} from "./projectOpenState";

const PROJECT = "30000000-0000-4000-8000-000000000001";
const SESSION = "20000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000001";
const ZOO6 = "50000000-0000-4000-8000-000000000002";
const LAPTOP = "50000000-0000-4000-8000-000000000007";

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(process.cwd(), "../protocol/fixtures", name), "utf8"));
}

function part(
  machineId: string | undefined,
  hostname: string,
  value: ProjectMachinePartInfo["part"],
  overrides: Partial<ProjectMachinePartInfo> = {},
): ProjectMachinePartInfo {
  return { machineId, hostname, part: value, ownsTabs: true, connected: true, ...overrides };
}

function report(state: ProjectOpenReport["state"], machines: ProjectMachinePartInfo[]): ProjectOpenReport {
  return { projectId: PROJECT, state, machines };
}

function machine(
  machineId: string,
  hostname: string,
  project: Partial<MachineProject> | null = {},
  overrides: Partial<MachineWithProjects> = {},
): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: project ? [{ projectId: PROJECT, path: "/work/project", isRunning: false, ...project }] : [],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
    ...overrides,
  };
}

function instance(machineId: string, hostname: string, primary = false): SessionInstance {
  return { machineId, hostname, primary };
}

function model(overrides: Partial<ProjectOpenModelInput>) {
  return buildProjectOpenModel({
    supported: true,
    projectIds: [PROJECT, SESSION],
    projectName: "mako",
    machines: [],
    target: { machineId: ZOO5, projectId: PROJECT, hostname: "zoo-005" },
    ...overrides,
  });
}

describe("the wire", () => {
  it("names the capability as the server does", () => {
    expect(PROJECT_OPEN_STATE_CAPABILITY).toBe("project_open_state_v1");
    expect(openStateNegotiated({ negotiatedCapabilities: new Set(["project_open_state_v1"]) })).toBe(true);
    expect(openStateNegotiated({ negotiatedCapabilities: new Set(["project_instances_v1"]) })).toBe(false);
  });

  it("reads a recorded frame with each machine's part", () => {
    expect(parseProjectOpenStateFrame(fixture("server-project-open-state.json"))).toEqual({
      projectId: PROJECT,
      state: "open",
      setBy: "owner@example.com",
      setKind: "request",
      setAt: "2026-11-16T05:00:00Z",
      machines: [
        { machineId: ZOO5, hostname: "zoo-005", part: "running", reason: undefined, ownsTabs: true, connected: true },
        { machineId: ZOO6, hostname: "zoo-006", part: "waiting_offline", reason: undefined, ownsTabs: true, connected: false },
      ],
    });
    const refused = parseProjectOpenStateFrame(fixture("server-project-open-state-refused.json"));
    expect(refused?.machines[1]).toMatchObject({
      part: "not_started",
      reason: "zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it.",
    });
  });

  it("reads a frame without a state as a project nobody has recorded", () => {
    const parsed = parseProjectOpenStateFrame(fixture("server-project-open-state-not-recorded.json"));
    expect(parsed?.state).toBeUndefined();
    expect(parsed?.setBy).toBeUndefined();
    // A host known only by name has no machine id.
    expect(parsed?.machines[1]).toMatchObject({ hostname: "laptop", part: "not_started", connected: false });
    expect(parsed?.machines[1].machineId).toBeUndefined();
  });

  it("drops what is outside the vocabulary instead of guessing", () => {
    expect(parseProjectOpenStateFrame({ type: "project_open_state", machines: [] })).toBeNull();
    expect(parseProjectOpenStateFrame({ project_id: PROJECT, state: "ajar", machines: [] })).toBeNull();
    expect(parseProjectOpenStateFrame({
      project_id: PROJECT,
      state: "open",
      machines: [{ hostname: "zoo-005", part: "paused", owns_tabs: true, connected: true }, null, { part: "running" }],
    })).toEqual({ projectId: PROJECT, state: "open", setBy: undefined, setKind: undefined, setAt: undefined, machines: [] });
    expect(parseOpenState("not_recorded")).toBeUndefined();
    expect(parseOpenState(null)).toBeUndefined();
    expect(parseMachinePart("still_running")).toBe("still_running");
    expect(parseMachinePart("stopped")).toBeUndefined();
    expect(parseProjectHosts(undefined)).toBeUndefined();
    expect(parseProjectHosts([])).toBeUndefined();
    expect(parseProjectHosts([{ hostname: "zoo-005", connected: true, running: true, owns_tabs: true }, { connected: true }]))
      .toEqual([{ hostname: "zoo-005", connected: true, running: true, ownsTabs: true }]);
  });

  it("selects the frame held under any of a project's ids", () => {
    const held = report("open", []);
    const state = { projectOpenStates: { [PROJECT]: held } };
    expect(selectProjectOpenReport(state, PROJECT)).toBe(held);
    expect(selectProjectOpenReport(state, [undefined, SESSION, PROJECT])).toBe(held);
    expect(selectProjectOpenReport(state, SESSION)).toBeUndefined();
    expect(selectProjectOpenReport(state, null)).toBeUndefined();
  });
});

describe("machinePartLabel", () => {
  const open = { state: "open" as const, recorded: true };
  const closed = { state: "closed" as const, recorded: true };
  const notRecorded = { state: "open" as const, recorded: false };

  it("words each part as the design does", () => {
    expect(machinePartLabel({ part: "running", hostname: "zoo-005" }, open)).toBe("Running");
    expect(machinePartLabel({ part: "starting", hostname: "zoo-005" }, open)).toBe("Starting…");
    expect(machinePartLabel({ part: "waiting_offline", hostname: "zoo-006" }, open)).toBe("Waiting for zoo-006 (offline)");
    expect(machinePartLabel({ part: "not_started", hostname: "zoo-006", reason: "zoo-006 refused." }, open))
      .toBe("Not started: zoo-006 refused.");
    expect(machinePartLabel({ part: "still_running", hostname: "zoo-006" }, closed)).toBe("Still running");
    expect(machinePartLabel({ hostname: "laptop" }, open)).toBe("No tabs here");
  });

  it("claims neither a start nor a failure for a part the server did not state", () => {
    const inferred = { part: "not_started" as const, hostname: "zoo-006", partUnconfirmed: true };
    expect(machinePartLabel(inferred, open)).toBe("Not running yet");
    // Outside an open project the machine was asked nothing.
    expect(machinePartLabel(inferred, closed)).toBe("Not running");
    expect(machinePartLabel(inferred, notRecorded)).toBe("Not running");
  });

  it("never words a tab owner of a closed or unrecorded project as a failure", () => {
    expect(machinePartLabel({ part: "not_started", hostname: "zoo-006" }, open)).toBe("Not started");
    expect(machinePartLabel({ part: "not_started", hostname: "zoo-006" }, closed)).toBe("Not running");
    expect(machinePartLabel({ part: "not_started", hostname: "zoo-006" }, notRecorded)).toBe("Not running");
  });
});

describe("buildProjectOpenModel", () => {
  it("offers only Open for a closed project and names the machines whose tabs start", () => {
    const result = model({
      report: report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]),
      session: { openState: "closed", isActive: false },
    });
    expect(result).toMatchObject({ state: "closed", recorded: true, stateLabel: "Closed" });
    expect(result.closeAction).toBeUndefined();
    expect(result.notice).toEqual([]);
    expect(result.openAction).toMatchObject({ kind: "open", label: "Open" });
    expect(result.openAction?.confirmation).toEqual({
      title: "Open mako?",
      body: ["Its tabs start on zoo-005 and zoo-006."],
    });
    // A tab owner of a closed project has not failed at anything.
    expect(result.machines.map((entry) => entry.label)).toEqual(["Not running", "Not running"]);
    expect(result.request).toEqual({ machineId: ZOO5, projectId: PROJECT });
  });

  it("offers only Close for an open project that both machines run", () => {
    const result = model({
      report: { ...report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]), setBy: "owner@example.com", setKind: "request", setAt: "2026-11-16T05:00:00Z" },
      session: { openState: "open", isActive: true },
    });
    expect(result).toMatchObject({
      state: "open",
      recorded: true,
      stateLabel: "Open",
      setBy: "owner@example.com",
      setKind: "request",
      setAt: "2026-11-16T05:00:00Z",
      runningHosts: ["zoo-005", "zoo-006"],
      tabOwnerHosts: ["zoo-005", "zoo-006"],
      stoppedFirstHosts: ["zoo-005", "zoo-006"],
      offlineTabOwnerHosts: [],
    });
    expect(result.openAction).toBeUndefined();
    expect(result.notice).toEqual([]);
    expect(result.closeAction).toMatchObject({ kind: "close", label: "Close" });
    expect(result.closeAction?.confirmation).toEqual({
      title: "Close mako?",
      body: ["This stops its agents on zoo-005 and zoo-006.", CLOSE_DELETES_NOTHING],
    });
  });

  it("says an offline machine's tabs start when it reconnects, and offers no Open again for it", () => {
    const result = model({
      report: report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "waiting_offline", { connected: false })]),
      session: { openState: "open", isActive: true },
    });
    expect(result.openAction).toBeUndefined();
    expect(result.notice).toEqual([{
      key: ZOO6,
      machineId: ZOO6,
      hostname: "zoo-006",
      kind: "waiting_offline",
      text: "zoo-006 is offline. Its tabs start when zoo-006 reconnects.",
      reason: undefined,
    }]);
    expect(result.machines.map((entry) => entry.label)).toEqual(["Running", "Waiting for zoo-006 (offline)"]);
    expect(result.offlineTabOwnerHosts).toEqual(["zoo-006"]);
    // Close names the offline tab owner too.
    expect(result.closeAction?.confirmation.body).toEqual([
      "This stops its agents on zoo-005.",
      "zoo-006 is offline. If it still runs the project when it reconnects, its agents are stopped then.",
      CLOSE_DELETES_NOTHING,
    ]);
  });

  it("names a refused machine with the server's reason and offers Open again", () => {
    const reason = "zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it.";
    const result = model({
      report: report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started", { reason })]),
      session: { openState: "open", isActive: true },
    });
    expect(result.notice).toEqual([{
      key: ZOO6,
      machineId: ZOO6,
      hostname: "zoo-006",
      kind: "not_started",
      text: "zoo-006 is not running this project.",
      reason,
    }]);
    expect(result.machines[1].label).toBe(`Not started: ${reason}`);
    expect(result.openAction).toMatchObject({ kind: "open_again", label: "Open again" });
    expect(result.openAction?.confirmation).toEqual({
      title: "Open mako again?",
      body: ["zoo-005 already runs it.", "Its tabs start on zoo-006."],
    });
    expect(result.closeAction?.kind).toBe("close");
  });

  it("says a machine that was asked to start is starting, without offering to open again", () => {
    const result = model({
      report: report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "starting")]),
      session: { openState: "open" },
    });
    expect(result.openAction).toBeUndefined();
    expect(result.notice.map((line) => [line.kind, line.text]))
      .toEqual([["starting", "zoo-006 is starting this project."]]);
    expect(result.stoppedFirstHosts).toEqual(["zoo-005", "zoo-006"]);
  });

  it("offers Keep open, saying what starts, and Close for a project nobody has recorded that runs on one of two machines", () => {
    const result = model({
      report: report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")]),
      session: { isActive: true },
    });
    expect(result).toMatchObject({ state: "open", recorded: false, stateLabel: "Open" });
    expect(result.notice.map((line) => [line.kind, line.text]))
      .toEqual([["not_started", "zoo-006 is not running this project."]]);
    expect(result.machines[1].label).toBe("Not running");
    // It runs somewhere, so the offer reads "Keep open" (the state beside it
    // already reads "Open"), as on every not-recorded project that runs.
    expect(result.openAction).toMatchObject({
      kind: "keep_open",
      label: "Keep open",
      note: keepOpenNote(["zoo-005", "zoo-006"]),
    });
    // What starts is said truthfully: confirming starts zoo-006.
    expect(result.openAction?.confirmation).toEqual({
      title: "Keep mako open?",
      body: [
        "zoo-005 already runs it.",
        "Its tabs start on zoo-006.",
        "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.",
      ],
    });
    expect(result.openAction?.confirmation.body.join(" ")).not.toContain("Nothing starts now");
    expect(result.closeAction?.kind).toBe("close");
  });

  describe("a machine whose daemon cannot run the project beside other machines", () => {
    const OLDER = "zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it.";
    // What the machine list says of zoo-006: connected, and "Older APAS".
    const older = (project: Partial<MachineProject> | null = {}) =>
      machine(ZOO6, "zoo-006", project, { projectInstancesAvailable: false });

    it("words the reason as the server does", () => {
      expect(olderApasReason("zoo-006")).toBe(OLDER);
    });

    it("is not offered Open again, and keeps its reason shown", () => {
      const result = model({
        report: report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started", { reason: OLDER })]),
        session: { openState: "open", isActive: true },
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }), older()],
      });
      // The server refuses zoo-006 every time until it is rebooted: an Open
      // would start nothing, so none is offered.
      expect(result.openAction).toBeUndefined();
      expect(result.closeAction?.kind).toBe("close");
      expect(result.machines[1]).toMatchObject({ cannotStart: OLDER, label: `Not started: ${OLDER}` });
      expect(result.notice).toEqual([{
        key: ZOO6,
        machineId: ZOO6,
        hostname: "zoo-006",
        kind: "not_started",
        text: "zoo-006 is not running this project.",
        reason: OLDER,
      }]);
    });

    it("says why before the server has, and names no start for it", () => {
      // After a server restart the reason is unknown until zoo-006 registers.
      const result = model({
        report: report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")]),
        session: { openState: "open", isActive: true },
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }), older()],
        fromMachineId: ZOO6,
      });
      expect(result.openAction).toBeUndefined();
      expect(result.notice[0]).toMatchObject({ kind: "not_started", reason: OLDER });
      expect(result.thisMachine).toMatchObject({ part: "not_started", label: `Not started: ${OLDER}`, reason: OLDER });
    });

    it("stays startable when the viewer's machine list does not have it: the server decides", () => {
      const result = model({
        report: report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started", { reason: OLDER })]),
        session: { openState: "open", isActive: true },
        machines: [machine(ZOO5, "zoo-005", { isRunning: true })],
      });
      expect(result.machines[1].cannotStart).toBeUndefined();
      expect(result.openAction).toMatchObject({ kind: "open_again", label: "Open again" });
      expect(result.openAction?.confirmation.body).toEqual(["zoo-005 already runs it.", "Its tabs start on zoo-006."]);
    });

    it("is started when it is the project's only machine, as the server runs it alone", () => {
      const result = model({
        report: report("open", [part(ZOO6, "zoo-006", "not_started")]),
        session: { openState: "open", isActive: false },
        machines: [older({ openState: "open" })],
        target: { machineId: ZOO6, projectId: PROJECT, hostname: "zoo-006" },
      });
      expect(result.machines[0].cannotStart).toBeUndefined();
      expect(result.openAction?.kind).toBe("open_again");
      expect(result.openAction?.confirmation.body).toEqual(["Its tabs start on zoo-006."]);
    });

    it("is named as not started, with the reason, when an Open is offered for other machines", () => {
      // Closed: Open starts zoo-005 and leaves zoo-006 as it is.
      const closed = model({
        report: report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]),
        session: { openState: "closed", isActive: false },
        machines: [machine(ZOO5, "zoo-005"), older()],
      });
      expect(closed.openAction).toMatchObject({ kind: "open", label: "Open" });
      expect(closed.openAction?.confirmation.body).toEqual([
        "Its tabs start on zoo-005.",
        `zoo-006 is not started. ${OLDER}`,
      ]);
      // A closed project's tab owner has not failed at anything.
      expect(closed.machines[1].label).toBe("Not running");
      expect(closed.notice).toEqual([]);

      // Open, with a third machine that can be started.
      const again = model({
        report: report("open", [
          part(LAPTOP, "laptop", "not_started", { reason: "The daemon did not answer the start." }),
          part(ZOO5, "zoo-005", "running"),
          part(ZOO6, "zoo-006", "not_started", { reason: OLDER }),
        ]),
        session: { openState: "open", isActive: true },
        machines: [machine(LAPTOP, "laptop"), machine(ZOO5, "zoo-005", { isRunning: true }), older()],
      });
      expect(again.openAction?.kind).toBe("open_again");
      expect(again.openAction?.confirmation.body).toEqual([
        "zoo-005 already runs it.",
        "Its tabs start on laptop.",
        `zoo-006 is not started. ${OLDER}`,
      ]);
    });

    it("is not started by keeping a not-recorded project open, and the confirmation says so", () => {
      const result = model({
        report: report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")]),
        session: { isActive: true },
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }), older()],
      });
      expect(result.openAction).toMatchObject({ kind: "keep_open", label: "Keep open" });
      expect(result.openAction?.confirmation).toEqual({
        title: "Keep mako open?",
        body: [
          "This records the project as open. Nothing starts now.",
          "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.",
          `zoo-006 is not started. ${OLDER}`,
        ],
      });
      expect(result.notice.map((line) => [line.text, line.reason]))
        .toEqual([["zoo-006 is not running this project.", OLDER]]);
    });
  });

  it("says an offline tab owner of an unrecorded project is not running it, and keeps the project open", () => {
    const result = model({
      report: report(undefined, [part(ZOO5, "zoo-005", "running"), part(undefined, "laptop", "not_started", { connected: false })]),
      session: { isActive: true },
    });
    expect(result.notice).toEqual([{
      key: "host:laptop",
      machineId: undefined,
      hostname: "laptop",
      kind: "offline",
      text: "laptop is not running this project because it is offline.",
      reason: undefined,
    }]);
    // Nothing can be started now, so recording is all an Open does.
    expect(result.openAction?.kind).toBe("keep_open");
    expect(result.openAction?.confirmation.body).toEqual([
      "This records the project as open. Nothing starts now.",
      "Its tabs start again on laptop and zoo-005 after those machines have been away.",
      "laptop is offline. Its tabs start when laptop reconnects.",
    ]);
  });

  it("offers Keep open for a project nobody has recorded that runs everywhere", () => {
    const result = model({
      report: report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]),
      session: { isActive: true },
    });
    expect(result).toMatchObject({ state: "open", recorded: false });
    expect(result.notice).toEqual([]);
    expect(result.openAction).toMatchObject({
      kind: "keep_open",
      label: "Keep open",
      note: "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.",
    });
    expect(result.openAction?.note).toBe(keepOpenNote(["zoo-005", "zoo-006"]));
    expect(result.openAction?.confirmation).toEqual({
      title: "Keep mako open?",
      body: [
        "This records the project as open. Nothing starts now.",
        "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.",
      ],
    });
    expect(result.closeAction?.kind).toBe("close");
  });

  it("offers Open for a project nobody has recorded that runs nowhere", () => {
    const result = model({ report: report(undefined, [part(ZOO5, "zoo-005", "not_started")]), session: { isActive: false } });
    expect(result).toMatchObject({ state: "closed", recorded: false, stateLabel: "Closed" });
    expect(result.openAction).toMatchObject({ kind: "open", label: "Open" });
    expect(result.openAction?.confirmation.body).toEqual(["Its tabs start on zoo-005."]);
    expect(result.closeAction).toBeUndefined();
    expect(result.notice).toEqual([]);
  });

  it("offers Open again and Close for an open project that runs nowhere", () => {
    // Its one tab owner is connected and did not start.
    const owned = model({
      report: report("open", [part(ZOO5, "zoo-005", "not_started")]),
      session: { openState: "open", isActive: false },
    });
    expect(owned.state).toBe("open");
    expect(owned.openAction).toMatchObject({ kind: "open_again", label: "Open again" });
    expect(owned.openAction?.confirmation.body).toEqual(["Its tabs start on zoo-005."]);
    expect(owned.closeAction?.kind).toBe("close");
    expect(owned.machines[0].label).toBe("Not started");

    // No machine has a part at all: the request names its machine.
    const bare = model({
      report: report("open", []),
      session: { openState: "open", isActive: false },
      machines: [machine(ZOO5, "zoo-005", { openState: "open" })],
    });
    expect(bare.openAction?.kind).toBe("open_again");
    expect(bare.openAction?.confirmation.body).toEqual(["It starts on zoo-005."]);
    expect(bare.closeAction?.confirmation.body)
      .toEqual(["This stops its agents on every machine that runs it.", CLOSE_DELETES_NOTHING]);

    // ...unless that machine is offline: nothing can be started.
    const offline = model({
      report: report("open", []),
      session: { openState: "open", isActive: false },
      machines: [machine(ZOO5, "zoo-005", { openState: "open" }, { daemonConnected: false })],
    });
    expect(offline.openAction).toBeUndefined();
    expect(offline.closeAction?.kind).toBe("close");
  });

  it("offers Close again while a machine still runs a closed project, with the reason", () => {
    const reason = "The account that closed this project may not use zoo-006.";
    const result = model({
      report: report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "still_running", { reason })]),
      session: { openState: "closed", isActive: true },
    });
    expect(result).toMatchObject({ state: "closed", recorded: true, runningHosts: ["zoo-006"] });
    expect(result.notice).toEqual([{
      key: ZOO6,
      machineId: ZOO6,
      hostname: "zoo-006",
      kind: "still_running",
      text: "zoo-006 still runs this project.",
      reason,
    }]);
    expect(result.machines.map((entry) => entry.label)).toEqual(["Not running", "Still running"]);
    expect(result.closeAction).toMatchObject({ kind: "close_again", label: "Close again" });
    expect(result.closeAction?.confirmation).toEqual({
      title: "Close mako again?",
      body: [
        "zoo-006 still runs this project.",
        // zoo-005 owns tabs and runs nothing: there is nothing to stop on it.
        "This stops its agents on zoo-006.",
        CLOSE_DELETES_NOTHING,
      ],
    });
    // Open stays the way out of a closed project.
    expect(result.openAction?.kind).toBe("open");
    expect(result.openAction?.confirmation.body).toEqual(["zoo-006 already runs it.", "Its tabs start on zoo-005."]);
  });

  describe("on a server without the capability", () => {
    const base = {
      supported: false,
      // A frame or a field from such a server would be a bug; neither is read.
      report: report("closed", [part(ZOO5, "zoo-005", "not_started")]),
      session: { openState: "closed" as const, isActive: true },
      panes: [
        { pane_id: 1, machine_id: ZOO5, hostname: "zoo-005" },
        { pane_id: 2, machine_id: ZOO6, hostname: "zoo-006" },
      ],
    };

    it("shows the project from what runs and never offers Keep open", () => {
      const result = model({
        ...base,
        instances: [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")],
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }), machine(ZOO6, "zoo-006", { isRunning: true })],
      });
      expect(result).toMatchObject({ state: "open", recorded: false, runningHosts: ["zoo-005", "zoo-006"] });
      expect(result.openAction).toBeUndefined();
      expect(result.closeAction?.kind).toBe("close");
      expect(result.closeAction?.confirmation.body)
        .toEqual(["This stops its agents on zoo-005 and zoo-006.", CLOSE_DELETES_NOTHING]);
    });

    it("offers Open for one that runs nowhere", () => {
      const result = model({
        ...base,
        session: { isActive: false },
        machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      });
      expect(result).toMatchObject({ state: "closed", recorded: false });
      expect(result.openAction).toMatchObject({ kind: "open", label: "Open" });
      expect(result.openAction?.confirmation.body).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
      expect(result.closeAction).toBeUndefined();
    });

    it("offers to open again only while that can start a connected machine", () => {
      const half = model({
        ...base,
        instances: [instance(ZOO5, "zoo-005", true)],
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }), machine(ZOO6, "zoo-006")],
      });
      expect(half.state).toBe("open");
      expect(half.openAction).toMatchObject({ kind: "open_again", label: "Open again" });
      // No promise that the project is kept open: this server records nothing.
      expect(half.openAction?.confirmation.body).toEqual(["zoo-005 already runs it.", "Its tabs start on zoo-006."]);
      expect(half.notice.map((line) => line.text)).toEqual(["zoo-006 is not running this project."]);

      const offline = model({
        ...base,
        instances: [instance(ZOO5, "zoo-005", true)],
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }), machine(ZOO6, "zoo-006", {}, { daemonConnected: false })],
      });
      expect(offline.openAction).toBeUndefined();
      expect(offline.notice.map((line) => line.text))
        .toEqual(["zoo-006 is not running this project because it is offline."]);
      expect(offline.closeAction?.confirmation.body).toEqual([
        "This stops its agents on zoo-005.",
        "zoo-006 is offline, so this does not reach it.",
        CLOSE_DELETES_NOTHING,
      ]);
    });

    it("names the machine the request names when no machine with the tabs is online", () => {
      // Every tab is zoo-005's, which is offline. Such a server then starts
      // the machine the request names; a server that records the state
      // waits for zoo-005 and starts nothing else.
      const input = {
        session: { isActive: false },
        panes: [{ pane_id: 1, machine_id: ZOO5, hostname: "zoo-005" }],
        instances: [],
        machines: [machine(ZOO5, "zoo-005", {}, { daemonConnected: false }), machine(LAPTOP, "laptop")],
        target: { machineId: LAPTOP, projectId: PROJECT, hostname: "laptop" },
      };
      const older = model({ ...input, supported: false });
      expect(older.openAction?.kind).toBe("open");
      expect(older.openAction?.confirmation.body)
        .toEqual(["It starts on laptop.", "zoo-005 is offline and is not started."]);

      const recording = model({
        ...input,
        supported: true,
        // Such a server fills in each listed machine's part.
        machines: [
          machine(ZOO5, "zoo-005", { part: "not_started" }, { daemonConnected: false }),
          machine(LAPTOP, "laptop"),
        ],
      });
      expect(recording.openAction?.confirmation.body)
        .toEqual(["zoo-005 is offline. Its tabs start when zoo-005 reconnects."]);
    });

    it("names no machine it cannot know of when asked from a machine list alone", () => {
      // The Machines page of an older server: no roster and no instances,
      // only this viewer's own machines.
      const stopped = model({
        supported: false,
        machines: [machine(ZOO5, "zoo-005")],
        fromMachineId: ZOO5,
      });
      expect(stopped.openAction?.confirmation.body)
        .toEqual(["It starts on each online machine that has its tabs, or on zoo-005 when none has."]);
      expect(stopped.thisMachine?.label).toBe("Not running");

      const running = model({
        supported: false,
        machines: [machine(ZOO5, "zoo-005", { isRunning: true })],
        fromMachineId: ZOO5,
      });
      expect(running.openAction).toBeUndefined();
      expect(running.closeAction?.confirmation.body)
        .toEqual(["This stops its agents on every machine that runs it.", CLOSE_DELETES_NOTHING]);
    });

    it("judges a project it knows only as an active session as open", () => {
      const result = model({ supported: false, session: { isActive: true }, target: null });
      expect(result).toMatchObject({ state: "open", recorded: false, request: null });
      // No machine to name: no action is offered.
      expect(result.openAction).toBeUndefined();
      expect(result.closeAction).toBeUndefined();
    });
  });

  describe("from the machine list alone", () => {
    it("shows an entry's own part and names the other machines an Open reaches", () => {
      const [entry] = (fixture("server-machines-open-state.json").machines as Array<Record<string, unknown>>);
      const projects = entry.projects as Array<Record<string, unknown>>;
      const zoo6 = machine(ZOO6, "zoo-006", {
        isRunning: false,
        openState: "open",
        part: "not_started",
        partReason: projects[0].part_reason as string,
        otherHosts: parseProjectHosts(projects[0].other_hosts),
      });
      const result = model({
        machines: [zoo6],
        target: { machineId: ZOO6, projectId: PROJECT },
        fromMachineId: ZOO6,
      });
      expect(result).toMatchObject({ state: "open", recorded: true });
      expect(result.thisMachine).toMatchObject({
        machineId: ZOO6,
        hostname: "zoo-006",
        part: "not_started",
        label: "Not started: zoo-006 could not start the project: the checkout is missing.",
        ownsTabs: true,
        running: false,
      });
      expect(result.machines.map((entry) => [entry.hostname, entry.part, entry.connected])).toEqual([
        ["laptop", "waiting_offline", false],
        ["zoo-005", "running", true],
        ["zoo-006", "not_started", true],
      ]);
      expect(result.openAction?.kind).toBe("open_again");
      expect(result.openAction?.confirmation.body).toEqual([
        "zoo-005 already runs it.",
        "Its tabs start on zoo-006.",
        "laptop is offline. Its tabs start when laptop reconnects.",
      ]);
      // Only a machine that runs the project is named as stopped.
      expect(result.closeAction?.confirmation.body).toEqual([
        "This stops its agents on zoo-005.",
        "laptop is offline. If it still runs the project when it reconnects, its agents are stopped then.",
        CLOSE_DELETES_NOTHING,
      ]);
    });

    // `other_hosts` says "running" only of a machine that runs the project:
    // one that was just asked to start arrives exactly like one that failed.
    it("does not say another account's machine failed to start, nor offer to open again for it", () => {
      const result = model({
        machines: [machine(ZOO5, "zoo-005", {
          isRunning: true,
          openState: "open",
          part: "running",
          otherHosts: [{ hostname: "zoo-006", connected: true, running: false, ownsTabs: true }],
        })],
        fromMachineId: ZOO5,
      });
      expect(result.machines.map((entry) => [entry.hostname, entry.part, entry.label, entry.partUnconfirmed === true])).toEqual([
        ["zoo-005", "running", "Running", false],
        ["zoo-006", "not_started", "Not running yet", true],
      ]);
      expect(result.openAction).toBeUndefined();
      expect(result.closeAction?.kind).toBe("close");
      expect(result.notice).toEqual([expect.objectContaining({
        hostname: "zoo-006",
        kind: "not_running_yet",
        text: "zoo-006 is not running this project yet.",
      })]);
    });

    it("keeps Open again for a machine whose part the server stated, without naming the unstated one", () => {
      const result = model({
        machines: [machine(ZOO5, "zoo-005", {
          openState: "open",
          part: "not_started",
          partReason: "zoo-005 could not start the project: the checkout is missing.",
          otherHosts: [{ hostname: "zoo-006", connected: true, running: false, ownsTabs: true }],
        })],
        fromMachineId: ZOO5,
      });
      expect(result.openAction).toMatchObject({ kind: "open_again", label: "Open again" });
      expect(result.openAction?.confirmation.body).toEqual(["Its tabs start on zoo-005."]);
      expect(result.machines.find((entry) => entry.hostname === "zoo-006")?.label).toBe("Not running yet");
    });

    it("still counts another machine of a closed project among those an Open starts", () => {
      const result = model({
        machines: [machine(ZOO5, "zoo-005", {
          openState: "closed",
          part: "not_started",
          otherHosts: [{ hostname: "zoo-006", connected: true, running: false, ownsTabs: true }],
        })],
        fromMachineId: ZOO5,
      });
      expect(result.machines.map((entry) => [entry.hostname, entry.label, entry.partUnconfirmed === true]))
        .toEqual([["zoo-005", "Not running", false], ["zoo-006", "Not running", false]]);
      expect(result.openAction?.confirmation.body).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    });

    it("says an Open from a machine with no tabs starts the others and not this one", () => {
      const laptop = machine(LAPTOP, "laptop", {
        openState: "closed",
        otherHosts: [{ hostname: "zoo-005", connected: true, running: false, ownsTabs: true }],
      });
      const result = model({
        machines: [laptop],
        target: { machineId: LAPTOP, projectId: PROJECT },
        fromMachineId: LAPTOP,
      });
      expect(result).toMatchObject({ state: "closed", recorded: true });
      expect(result.thisMachine).toMatchObject({ label: "No tabs here", ownsTabs: false, running: false });
      expect(result.thisMachine?.part).toBeUndefined();
      expect(result.openAction?.confirmation).toEqual({
        title: "Open mako?",
        body: ["It starts on zoo-005 and not on laptop, which has no tabs of it."],
      });
      expect(result.request).toEqual({ machineId: LAPTOP, projectId: PROJECT });
    });

    it("shows an open project with no tabs on this machine, and offers nothing that starts it here", () => {
      const result = model({
        machines: [
          machine(ZOO5, "zoo-005", { isRunning: true, openState: "open", part: "running" }),
          machine(ZOO6, "zoo-006", {
            openState: "open",
            otherHosts: [{ hostname: "zoo-005", connected: true, running: true, ownsTabs: true }],
          }),
        ],
        target: { machineId: ZOO6, projectId: PROJECT },
        fromMachineId: ZOO6,
      });
      expect(result.state).toBe("open");
      expect(result.thisMachine?.label).toBe("No tabs here");
      // zoo-006 has no part, so it is not among the project's machines.
      expect(result.machines.map((entry) => entry.hostname)).toEqual(["zoo-005"]);
      expect(result.openAction).toBeUndefined();
      expect(result.closeAction?.confirmation.body)
        .toEqual(["This stops its agents on zoo-005.", CLOSE_DELETES_NOTHING]);
    });

    it("starts a registered project that never ran on the machine whose entry opens it", () => {
      const result = model({
        machines: [machine(ZOO5, "zoo-005")],
        target: { machineId: ZOO5, projectId: PROJECT },
        fromMachineId: ZOO5,
      });
      expect(result).toMatchObject({ state: "closed", recorded: false });
      expect(result.openAction).toMatchObject({ kind: "open", label: "Open" });
      expect(result.openAction?.confirmation.body).toEqual(["It starts on zoo-005."]);
      expect(result.closeAction).toBeUndefined();
      expect(result.thisMachine?.label).toBe("No tabs here");
    });

    it("offers to keep open and to close an unrecorded project this machine runs", () => {
      const result = model({
        machines: [machine(ZOO5, "zoo-005", { isRunning: true, part: "running" })],
        fromMachineId: ZOO5,
      });
      expect(result).toMatchObject({ state: "open", recorded: false });
      expect(result.thisMachine).toMatchObject({ label: "Running", running: true });
      expect(result.openAction).toMatchObject({
        kind: "keep_open",
        note: "Its tabs start again on zoo-005 after those machines have been away.",
      });
      expect(result.closeAction?.kind).toBe("close");
    });

    it("says a closed project another machine still runs is still running there", () => {
      const result = model({
        machines: [machine(ZOO5, "zoo-005", {
          openState: "closed",
          part: "not_started",
          otherHosts: [{ hostname: "zoo-006", connected: true, running: true, ownsTabs: true }],
        })],
        fromMachineId: ZOO5,
      });
      expect(result.machines.map((entry) => [entry.hostname, entry.label]))
        .toEqual([["zoo-005", "Not running"], ["zoo-006", "Still running"]]);
      expect(result.closeAction?.kind).toBe("close_again");
    });

    it("takes a machine that runs the project as running where the server filled no part in", () => {
      // A shared machine's entry for a project this viewer holds no role in.
      const result = model({
        machines: [machine(ZOO5, "zoo-005", { isRunning: true }, { clusterAccess: "member" })],
        fromMachineId: ZOO5,
      });
      expect(result).toMatchObject({ state: "open", recorded: false, runningHosts: ["zoo-005"] });
      expect(result.thisMachine).toMatchObject({ label: "Running", running: true });
      expect(result.closeAction?.kind).toBe("close");
      expect(result.openAction?.kind).toBe("keep_open");
    });

    it("claims nothing about tabs for a shared machine's entry the server filled nothing in", () => {
      const shared = model({
        machines: [machine(ZOO5, "zoo-005", {}, { clusterAccess: "member" })],
        fromMachineId: ZOO5,
      });
      expect(shared.thisMachine?.label).toBe("Not running");
      const older = model({
        supported: false,
        machines: [machine(ZOO5, "zoo-005", { isRunning: true })],
        fromMachineId: ZOO5,
      });
      expect(older.thisMachine).toMatchObject({ label: "Running", running: true });
    });
  });

  it("does not use a frame the lists have superseded", () => {
    // The viewer was not sent the close; the polled session entry has it.
    const stale = report("open", [part(ZOO5, "zoo-005", "running")]);
    const result = model({
      report: stale,
      session: { openState: "closed", isActive: false },
      machines: [machine(ZOO5, "zoo-005", { openState: "closed", part: "not_started" })],
    });
    expect(result).toMatchObject({ state: "closed", recorded: true, runningHosts: [] });
    expect(result.openAction?.kind).toBe("open");
    expect(result.closeAction).toBeUndefined();
  });

  it("offers nothing without a machine to name, and Delete only to the owner", () => {
    // A roster of untagged tabs: the frame names the host, and no machine.
    const result = model({
      report: report("closed", [part(undefined, "zoo-005", "not_started")]),
      session: { openState: "closed" },
      target: null,
    });
    expect(result.request).toBeNull();
    expect(result.openAction).toBeUndefined();
    expect(result.closeAction).toBeUndefined();
    expect(result.canDelete).toBe(false);
    // No frame and no entry: nothing names a machine either.
    const unknown = model({ session: { openState: "open", isActive: false }, target: null });
    expect(unknown.request).toBeNull();
    expect(unknown.openAction).toBeUndefined();
    expect(unknown.closeAction).toBeUndefined();
    expect(model({ isOwner: true }).canDelete).toBe(true);
    expect(model({ isOwner: false }).canDelete).toBe(false);
  });

  describe("when no machine-list entry lists the project", () => {
    // The server keeps its machine list in memory. After it restarts, a
    // machine that has not registered again is in no list, while the frame
    // still names the project's machines. Open and Close are for the whole
    // project, and the server takes them without that machine connected.
    const offline = { connected: false };

    it("takes the request's machine from the frame, so an open project can be closed", () => {
      const result = model({
        report: report("open", [part(LAPTOP, "laptop", "waiting_offline", offline)]),
        session: { openState: "open", isActive: false },
        machines: [],
        target: null,
      });
      expect(result).toMatchObject({ state: "open", recorded: true });
      expect(result.request).toEqual({ machineId: LAPTOP, projectId: PROJECT });
      expect(result.notice.map((line) => line.text))
        .toEqual(["laptop is offline. Its tabs start when laptop reconnects."]);
      // Nothing an Open would start now; the close is recorded for its return.
      expect(result.openAction).toBeUndefined();
      expect(result.closeAction).toMatchObject({ kind: "close", label: "Close" });
      expect(result.closeAction?.confirmation.body).toEqual([
        "This stops its agents on every machine that runs it.",
        "laptop is offline. If it still runs the project when it reconnects, its agents are stopped then.",
        CLOSE_DELETES_NOTHING,
      ]);
    });

    it("lets a closed project be opened to wait for its machine", () => {
      const result = model({
        report: report("closed", [part(LAPTOP, "laptop", "not_started", offline)]),
        session: { openState: "closed", isActive: false },
        machines: [],
        target: null,
      });
      expect(result.request).toEqual({ machineId: LAPTOP, projectId: PROJECT });
      expect(result.openAction).toMatchObject({ kind: "open", label: "Open" });
      expect(result.openAction?.confirmation.body)
        .toEqual(["laptop is offline. Its tabs start when laptop reconnects."]);
      expect(result.closeAction).toBeUndefined();
    });

    it("prefers a connected tab owner, and skips a host the frame knows only by name", () => {
      const result = model({
        report: report(undefined, [
          part(undefined, "mini", "not_started"),
          part(LAPTOP, "laptop", "not_started", offline),
          part(ZOO6, "zoo-006", "running", { ownsTabs: false }),
          part(ZOO5, "zoo-005", "not_started"),
        ]),
        session: { isActive: true },
        machines: [],
        target: null,
      });
      expect(result.request).toEqual({ machineId: ZOO5, projectId: PROJECT });
      // With no connected tab owner, the first machine named by id.
      const first = model({
        report: report("open", [
          part(undefined, "mini", "waiting_offline", offline),
          part(LAPTOP, "laptop", "waiting_offline", offline),
          part(ZOO6, "zoo-006", "waiting_offline", offline),
        ]),
        session: { openState: "open" },
        machines: [],
        target: null,
      });
      expect(first.request).toEqual({ machineId: LAPTOP, projectId: PROJECT });
    });

    it("leaves the machine to the list while an entry lists the project, and to no one on an older server", () => {
      const held = report("open", [part(LAPTOP, "laptop", "waiting_offline", offline)]);
      // An entry lists it: the caller's answer stands, also when it is none.
      const listed = model({
        report: held,
        session: { openState: "open" },
        machines: [machine(ZOO5, "zoo-005", { openState: "open" })],
        target: null,
      });
      expect(listed.request).toBeNull();
      // A frame kept from a server that no longer records open states.
      const older = model({ supported: false, report: held, session: { isActive: true }, machines: [], target: null });
      expect(older.request).toBeNull();
      expect(older.closeAction).toBeUndefined();
    });
  });

  it("says an Open of a closed project waits for its only machine while that is offline", () => {
    const result = model({
      session: { openState: "closed" },
      machines: [machine(ZOO5, "zoo-005", { openState: "closed" }, { daemonConnected: false })],
    });
    expect(result.openAction?.confirmation.body)
      .toEqual(["zoo-005 is offline. It starts there when zoo-005 reconnects."]);
  });

  it("falls back to this project's name when none is given", () => {
    const result = model({ projectName: undefined, report: report("open", [part(ZOO5, "zoo-005", "running")]), session: { openState: "open" } });
    expect(result.closeAction?.confirmation.title).toBe("Close this project?");
  });
});

describe("shared texts", () => {
  it("quotes the design's sentences exactly", () => {
    expect(CLOSE_DELETES_NOTHING).toBe("Nothing is deleted. Its tabs come back when you open it again.");
    expect(NOTHING_SENT_TEXT).toBe("Not connected to the server. Nothing was sent.");
    expect(keepOpenNote(["zoo-005", "zoo-006"]))
      .toBe("Its tabs start again on zoo-005 and zoo-006 after those machines have been away.");
    // The quoted sentence is kept word for word with one host too.
    expect(keepOpenNote(["zoo-005"])).toBe("Its tabs start again on zoo-005 after those machines have been away.");
  });

  it("words a machine that is offline as a tab on it is refused", () => {
    // One sentence, one source: the notice line and the tab's refusal.
    const result = buildProjectOpenModel({
      supported: false,
      projectIds: [PROJECT],
      panes: [{ pane_id: 1, machine_id: ZOO5, hostname: "zoo-005" }, { pane_id: 2, machine_id: ZOO6, hostname: "zoo-006" }],
      instances: [instance(ZOO5, "zoo-005", true)],
      machines: [machine(ZOO5, "zoo-005", { isRunning: true }), machine(ZOO6, "zoo-006", {}, { daemonConnected: false })],
      target: { machineId: ZOO5, projectId: PROJECT },
    });
    expect(result.notice.map((line) => line.text)).toEqual([stoppedOnHostReason("zoo-006", true)]);
    expect(stoppedOnHostReason("zoo-006", true)).toBe("zoo-006 is not running this project because it is offline.");
  });

  it("keeps a confirmation's last line last when lines are added", () => {
    const copy = { title: "Close mako?", body: ["This stops its agents on zoo-005.", CLOSE_DELETES_NOTHING] };
    expect(withLinesBeforeLast(copy, ["Tab 2 is on autopilot."]).body)
      .toEqual(["This stops its agents on zoo-005.", "Tab 2 is on autopilot.", CLOSE_DELETES_NOTHING]);
    expect(withLinesBeforeLast(copy, [])).toBe(copy);
  });
});
