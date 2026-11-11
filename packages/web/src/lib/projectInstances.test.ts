import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  instanceHostSummary,
  instanceIsLegacy,
  paneMachineId,
  parseSessionInstances,
  primaryInstance,
  projectPlacement,
  resolvePaneInstance,
  selectSessionInstances,
  type SessionInstance,
} from "./projectInstances";
import type { MachineWithProjects, PaneConfig } from "./store";

/** The protocol's golden frames, exactly as the server sends them. */
function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8"));
}

const ZOO5 = "50000000-0000-4000-8000-000000000001";
const ZOO6 = "50000000-0000-4000-8000-000000000002";
const LAPTOP = "50000000-0000-4000-8000-000000000003";

function instance(
  machineId: string | undefined,
  hostname: string,
  overrides: Partial<SessionInstance> = {},
): SessionInstance {
  return { machineId, hostname, primary: false, ...overrides };
}

function pane(pane_id: number, machine_id?: string | null, hostname?: string | null): PaneConfig {
  return {
    pane_id,
    provider: "claude",
    mode: "interactive",
    kind: "terminal",
    session_id: `pane-${pane_id}`,
    is_paused: false,
    machine_id,
    hostname,
  };
}

function machine(
  machineId: string,
  hostname: string,
  flags: Partial<MachineWithProjects> = {},
): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
    ...flags,
  };
}

describe("parseSessionInstances", () => {
  it("reads the golden frames, including a legacy instance with no machine", () => {
    expect(parseSessionInstances(fixture("server-session-instances.json").instances)).toEqual([
      { machineId: ZOO5, hostname: "zoo-005", workingDir: "/workspace/project", primary: true, runtimeAccess: true },
      { machineId: ZOO6, hostname: "zoo-006", workingDir: "/workspace/project", primary: false, runtimeAccess: false },
    ]);
    expect(parseSessionInstances(fixture("server-session-instances-legacy.json").instances)).toEqual([
      { machineId: undefined, hostname: "laptop", workingDir: "/workspace/project", primary: true, runtimeAccess: undefined },
    ]);
  });

  it("keeps an absent list distinct from an empty one and skips malformed entries", () => {
    expect(parseSessionInstances(undefined)).toBeUndefined();
    expect(parseSessionInstances([])).toEqual([]);
    expect(parseSessionInstances([null, { machine_id: ZOO5 }, { hostname: "zoo-005", machine_id: null }]))
      .toEqual([{ machineId: undefined, hostname: "zoo-005", workingDir: undefined, primary: false, runtimeAccess: undefined }]);
  });
});

describe("selectSessionInstances", () => {
  it("reads the session's list entry, and the pushed record only for a session the list lacks", () => {
    const listed = [instance(ZOO5, "zoo-005", { primary: true })];
    const pushed = [instance(ZOO6, "zoo-006", { primary: true })];
    const state = {
      sessions: [{ id: "listed", instances: listed }, { id: "listed-empty" }],
      sessionInstances: { listed: pushed, "listed-empty": pushed, unlisted: pushed },
    };
    expect(selectSessionInstances(state, "listed")).toBe(listed);
    // A list entry without instances means nothing is attached.
    expect(selectSessionInstances(state, "listed-empty")).toEqual([]);
    expect(selectSessionInstances(state, "unlisted")).toBe(pushed);
    expect(selectSessionInstances(state, "unknown")).toEqual([]);
    expect(selectSessionInstances(state, null)).toEqual([]);
    // Stable when empty, so it can be a store selector without re-rendering
    // its subscribers on every change.
    expect(selectSessionInstances(state, "unknown")).toBe(selectSessionInstances(state, "listed-empty"));
  });
});

describe("host summary", () => {
  it("names the primary host and counts the other instances", () => {
    const instances = [
      instance(ZOO6, "zoo-006"),
      instance(ZOO5, "zoo-005", { primary: true }),
    ];
    expect(primaryInstance(instances)?.hostname).toBe("zoo-005");
    expect(instanceHostSummary(instances, "recorded")).toEqual({ primaryHost: "zoo-005", additionalCount: 1 });
    // An older server reports only the session's recorded host.
    expect(instanceHostSummary([], "zoo-005")).toEqual({ primaryHost: "zoo-005", additionalCount: 0 });
  });
});

describe("instanceIsLegacy", () => {
  it("recognises instances that cannot run beside another machine's", () => {
    const machines = [
      machine(ZOO5, "zoo-005"),
      machine(ZOO6, "zoo-006", { projectInstancesAvailable: false }),
      machine(LAPTOP, "laptop", { daemonConnected: false, projectInstancesAvailable: false }),
    ];
    expect(instanceIsLegacy(instance(undefined, "old"), machines)).toBe(true);
    expect(instanceIsLegacy(instance(ZOO6, "zoo-006"), machines)).toBe(true);
    expect(instanceIsLegacy(instance(ZOO5, "zoo-005"), machines)).toBe(false);
    // Not known to be older: assumed capable, so the server decides.
    expect(instanceIsLegacy(instance(LAPTOP, "laptop"), machines)).toBe(false);
    expect(instanceIsLegacy(instance("unlisted", "unlisted"), machines)).toBe(false);
  });
});

describe("resolvePaneInstance", () => {
  const zoo5 = instance(ZOO5, "zoo-005", { primary: true });
  const zoo6 = instance(ZOO6, "zoo-006");

  it("routes a tagged pane only to its own machine", () => {
    expect(resolvePaneInstance(pane(1, ZOO6, "zoo-006"), [zoo5, zoo6])).toBe(zoo6);
    // Its machine is stopped: never the primary instead.
    expect(resolvePaneInstance(pane(1, ZOO6, "zoo-006"), [zoo5])).toBeNull();
  });

  it("routes an untagged pane by its attributed host, then to the primary only without one", () => {
    expect(resolvePaneInstance(pane(1, null, "ZOO-006"), [zoo5, zoo6])).toBe(zoo6);
    expect(resolvePaneInstance(pane(1, null, null), [zoo5, zoo6])).toBe(zoo5);
    // Attributed to a host that is not running the project: stopped there,
    // even though another machine is attached.
    expect(resolvePaneInstance(pane(1, null, "zoo-007"), [zoo5, zoo6])).toBeNull();
    expect(resolvePaneInstance(pane(1, null, null), [])).toBeNull();
  });

  it("gives every untagged pane to a sole instance without multi-machine support", () => {
    const old = instance(undefined, "laptop", { primary: true });
    expect(resolvePaneInstance(pane(1, null, "zoo-005"), [old])).toBe(old);
    const olderDaemon = instance(ZOO6, "zoo-006", { primary: true });
    const machines = [machine(ZOO6, "zoo-006", { projectInstancesAvailable: false })];
    expect(resolvePaneInstance(pane(1, null, "zoo-005"), [olderDaemon], machines)).toBe(olderDaemon);
    // A capable sole instance does not own another host's untagged tabs.
    expect(resolvePaneInstance(pane(1, null, "zoo-005"), [zoo6], [machine(ZOO6, "zoo-006")])).toBeNull();
  });
});

describe("projectPlacement", () => {
  it("reads the golden pane list as spanning two machines, one stopped", () => {
    const panes = fixture("server-pane-list-instances.json").panes as PaneConfig[];
    const placement = projectPlacement(panes, [instance(ZOO5, "zoo-005", { primary: true })]);
    expect(placement.spansMachines).toBe(true);
    expect(placement.panes.get(3)).toMatchObject({ machineId: ZOO5, hostname: "zoo-005", connected: true, usable: true });
    expect(placement.panes.get(4)).toMatchObject({ machineId: ZOO6, hostname: "zoo-006", connected: false, usable: false, instance: null });
    expect(placement.machines).toEqual([
      { key: ZOO5, machineId: ZOO5, hostname: "zoo-005", attached: true, primary: true, paneIds: [3] },
      { key: ZOO6, machineId: ZOO6, hostname: "zoo-006", attached: false, primary: false, paneIds: [4] },
    ]);
  });

  it("does not span machines for a one-machine project, tagged or not", () => {
    const instances = [instance(ZOO5, "zoo-005", { primary: true })];
    expect(projectPlacement([pane(1, ZOO5, "zoo-005"), pane(2, null, "zoo-005")], instances).spansMachines)
      .toBe(false);
    expect(projectPlacement([pane(1, null, null), pane(2, null, null)], instances).spansMachines).toBe(false);
    expect(projectPlacement([], []).spansMachines).toBe(false);
  });

  it("places an untagged pane under its attributed host and maps that host to a machine", () => {
    const placement = projectPlacement(
      [pane(1, ZOO5, "zoo-005"), pane(2, null, "zoo-006")],
      [instance(ZOO5, "zoo-005", { primary: true })],
      [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    );
    expect(placement.spansMachines).toBe(true);
    // Stopped on zoo-006, which the inventory knows, so "Start on zoo-006"
    // has a machine to start.
    expect(placement.panes.get(2)).toMatchObject({ machineId: ZOO6, hostname: "zoo-006", connected: false });
    expect(placement.machines.map((state) => [state.hostname, state.attached, state.paneIds]))
      .toEqual([["zoo-005", true, [1]], ["zoo-006", false, [2]]]);
  });

  it("counts an untagged pane with no attributed host as the primary's, with no badge", () => {
    const placement = projectPlacement(
      [pane(1, ZOO6, "zoo-006"), pane(2, null, null)],
      [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
    );
    expect(placement.panes.get(2)).toMatchObject({ machineId: ZOO5, hostname: undefined, connected: true });
    expect(placement.spansMachines).toBe(true);
  });

  it("disables tabs of an instance this viewer may not use", () => {
    const placement = projectPlacement(
      [pane(1, ZOO6, "zoo-006")],
      [instance(ZOO6, "zoo-006", { primary: true, runtimeAccess: false })],
    );
    expect(placement.panes.get(1)).toMatchObject({ connected: true, usable: false });
  });

  it("shows a renamed host by its attached instance's current name", () => {
    const placement = projectPlacement(
      [pane(1, ZOO5, null), pane(2, ZOO6, "old-name")],
      [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
    );
    expect(placement.panes.get(1)?.hostname).toBe("zoo-005");
    // The pane's own stamp is what the server sent for that tab.
    expect(placement.panes.get(2)?.hostname).toBe("old-name");
    expect(placement.machines.find((state) => state.machineId === ZOO6)?.hostname).toBe("zoo-006");
  });

  it("does not map a hostname two machines share", () => {
    const placement = projectPlacement(
      [pane(1, null, "twin")],
      [],
      [machine(ZOO5, "twin"), machine(ZOO6, "twin")],
    );
    expect(placement.panes.get(1)).toMatchObject({ machineId: undefined, hostname: "twin", connected: false });
  });
});

describe("paneMachineId", () => {
  it("names the machine of a tagged pane, or of an untagged pane's attributed host", () => {
    const machines = [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")];
    expect(paneMachineId(pane(1, ZOO6, "zoo-006"), [], machines)).toBe(ZOO6);
    expect(paneMachineId(pane(1, null, "zoo-005"), [], machines)).toBe(ZOO5);
    expect(paneMachineId(pane(1, null, "elsewhere"), [], machines)).toBeUndefined();
    expect(paneMachineId(pane(1, null, null), [instance(ZOO6, "zoo-006", { primary: true })], machines))
      .toBe(ZOO6);
  });
});
