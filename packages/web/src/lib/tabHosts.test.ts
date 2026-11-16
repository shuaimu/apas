import { describe, expect, it } from "vitest";
import type { AddPaneRequest, CliLifecycleInventory, MachineWithProjects, PaneConfig } from "./store";
import { LEGACY_MACHINE_KEY, type SessionInstance } from "./projectInstances";
import { buildMachineChoices } from "./machineChoice";
import {
  cloneHereCopy,
  cloneInstanceName,
  confirmationText,
  describeTabHosts,
  forgetMachineTabsCopy,
  groupMachineChoices,
  hostList,
  instanceRebootTargets,
  launchStatus,
  machineChoiceDetail,
  projectHostLabel,
  sessionIsOwned,
  stoppedOnHostReason,
  type TabHostsInput,
} from "./tabHosts";

const PROJECT = "30000000-0000-4000-8000-000000000001";
const SESSION = "20000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const LAPTOP = "50000000-0000-4000-8000-000000000007";

type Pane = Pick<PaneConfig, "pane_id" | "machine_id" | "hostname">;

function pane(pane_id: number, machine_id: string | null, hostname: string | null): Pane {
  return { pane_id, machine_id, hostname };
}

function instance(machineId: string | undefined, hostname: string, overrides: Partial<SessionInstance> = {}): SessionInstance {
  return { machineId, hostname, primary: false, ...overrides };
}

function machine(machineId: string, hostname: string, overrides: Partial<MachineWithProjects> & { registered?: boolean } = {}): MachineWithProjects {
  const { registered = true, ...rest } = overrides;
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: registered ? [{ projectId: PROJECT, path: "/work/project", isRunning: false }] : [],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
    ...rest,
  };
}

function hosts(overrides: Partial<TabHostsInput>) {
  return describeTabHosts({
    panes: [],
    instances: [],
    machines: [],
    instancesSupported: true,
    projectIds: [PROJECT, SESSION],
    isOwner: true,
    ...overrides,
  });
}

describe("describeTabHosts", () => {
  it("says nothing per tab for a single machine running the project", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO5, "zoo-005")],
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [machine(ZOO5, "zoo-005")],
    });
    expect(result.spansMachines).toBe(false);
    for (const id of [1, 2]) {
      expect(result.tabs.get(id)).toMatchObject({ hostname: "zoo-005", state: "running", badge: false });
      expect(result.tabs.get(id)?.refusal).toBeUndefined();
    }
  });

  it("badges every tab once the roster spans machines", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    });
    expect(result.spansMachines).toBe(true);
    expect([...result.tabs.values()].map((tab) => [tab.hostname, tab.state, tab.badge]))
      .toEqual([["zoo-005", "running", true], ["zoo-006", "running", true]]);
  });

  it("refuses a stopped machine's tabs in the server's words and offers only to forget them", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    });
    const stopped = result.tabs.get(2);
    expect(stopped).toMatchObject({ hostname: "zoo-006", machineId: ZOO6, state: "stopped", badge: true });
    expect(stopped?.refusal).toBe("zoo-006 is not running this project. Open the project again to start it there.");
    // Starting one machine alone is not an action any more: the model
    // carries nothing a "Start on zoo-006" could send.
    expect(stopped?.stopped).toEqual({ machineId: ZOO6, hostname: "zoo-006", canForget: true });
    expect(result.tabs.get(1)?.refusal).toBeUndefined();
  });

  it("words the refusal by whether the tab's machine is connected", () => {
    const base = {
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
    };
    const offline = hosts({ ...base, machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { daemonConnected: false, projectInstancesAvailable: false })] });
    expect(offline.tabs.get(2)).toMatchObject({
      state: "offline",
      refusal: "zoo-006 is not running this project because it is offline.",
    });
    expect(offline.tabs.get(2)?.stopped).toEqual({ machineId: ZOO6, hostname: "zoo-006", canForget: true });

    // Connected, whatever keeps it from starting: an older daemon, or a
    // project it does not list. Opening the project again is what asks it,
    // and the project's notice carries the server's reason.
    for (const overrides of [{ projectInstancesAvailable: false }, { registered: false }]) {
      const connected = hosts({ ...base, machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", overrides)] });
      expect(connected.tabs.get(2)).toMatchObject({
        state: "stopped",
        refusal: "zoo-006 is not running this project. Open the project again to start it there.",
      });
    }

    // A machine missing from the viewer's machine list reads as connected,
    // which is what the server assumes until it knows better.
    const foreign = hosts({ ...base, machines: [machine(ZOO5, "zoo-005")] });
    expect(foreign.tabs.get(2)?.refusal).toBe(stoppedOnHostReason("zoo-006"));
    expect(foreign.tabs.get(2)?.stopped).toEqual({ machineId: ZOO6, hostname: "zoo-006", canForget: true });
  });

  it("still disables the new-tab choice beside an attached CLI without multi-machine support", () => {
    // zoo-005's daemon is connected but older, so its instance owns the whole
    // project: the server would refuse zoo-006's instance beside it.
    const panes = [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")];
    const machines = [machine(ZOO5, "zoo-005", { projectInstancesAvailable: false }), machine(ZOO6, "zoo-006")];
    const instances = [instance(ZOO5, "zoo-005", { primary: true })];
    const result = hosts({ panes, instances, machines });
    expect(result.tabs.get(2)).toMatchObject({ state: "stopped", refusal: stoppedOnHostReason("zoo-006") });
    const picker = buildMachineChoices({
      projectId: PROJECT,
      instances,
      machines,
      instancesSupported: true,
      canClone: false,
    });
    expect(picker.choices.find((choice) => choice.machineId === ZOO6)?.disabledReason)
      .toBe("Reboot zoo-005 to update");
  });

  it("calls a machine that still runs the project disconnected, with nothing to open", () => {
    // zoo-006's instance lost its socket; its daemon is connected and still
    // reports the project running, so the tabs come back by themselves.
    const panes = [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006"), pane(3, null, "zoo-006")];
    const instances = [instance(ZOO5, "zoo-005", { primary: true })];
    const running = (machineId: string, hostname: string) => machine(machineId, hostname, {
      projects: [{ projectId: PROJECT, path: "/work/project", isRunning: true }],
    });
    const result = hosts({ panes, instances, machines: [running(ZOO5, "zoo-005"), running(ZOO6, "zoo-006")] });
    for (const id of [2, 3]) {
      const tab = result.tabs.get(id);
      expect(tab).toMatchObject({
        hostname: "zoo-006",
        machineId: ZOO6,
        state: "disconnected",
        badge: true,
        refusal: "zoo-006 is reconnecting to this project. Its tabs work again once it reconnects.",
      });
      // Its tabs can still be forgotten: a daemon that dropped silently
      // reads this way for hours.
      expect(tab?.stopped).toEqual({ machineId: ZOO6, hostname: "zoo-006", canForget: true });
    }
    expect(result.tabs.get(1)?.state).toBe("running");

    // Once the daemon reports it stopped, it is stopped.
    const stopped = hosts({ panes, instances, machines: [running(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")] });
    expect(stopped.tabs.get(2)).toMatchObject({ state: "stopped", refusal: stoppedOnHostReason("zoo-006") });

    // A running entry under another project's id says nothing about this one.
    const otherProject = hosts({
      panes,
      instances,
      machines: [running(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", {
        projects: [{ projectId: "30000000-0000-4000-8000-000000000999", path: "/work/other", isRunning: true }],
      })],
    });
    expect(otherProject.tabs.get(2)?.state).toBe("stopped");

    // A daemon that is gone is offline whatever it last reported.
    const offline = hosts({
      panes,
      instances,
      machines: [running(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", {
        daemonConnected: false,
        projects: [{ projectId: PROJECT, path: "/work/project", isRunning: true }],
      })],
    });
    expect(offline.tabs.get(2)?.state).toBe("offline");
  });

  it("keeps the real status of the tabs of a machine that still runs a closed project", () => {
    // The project is recorded closed, but zoo-006 could not be stopped and
    // its instance is still attached. Its tabs work, and say so.
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO6, "zoo-006", { primary: true })],
      machines: [
        machine(ZOO5, "zoo-005", { projects: [{ projectId: PROJECT, path: "/work/project", isRunning: false, openState: "closed", part: "not_started" }] }),
        machine(ZOO6, "zoo-006", { projects: [{ projectId: PROJECT, path: "/work/project", isRunning: true, openState: "closed", part: "still_running" }] }),
      ],
    });
    expect(result.tabs.get(2)).toMatchObject({ hostname: "zoo-006", state: "running" });
    expect(result.tabs.get(2)?.refusal).toBeUndefined();
    expect(result.tabs.get(1)).toMatchObject({ state: "stopped", refusal: stoppedOnHostReason("zoo-005") });
  });

  it("refuses a stopped machine's tab in the server's two sentences", () => {
    // Design Decision 11: one sentence while that machine's daemon is
    // connected, one while it is not. A disabled control and a refused
    // request read the same.
    expect(stoppedOnHostReason("zoo-005"))
      .toBe("zoo-005 is not running this project. Open the project again to start it there.");
    expect(stoppedOnHostReason("zoo-005", true))
      .toBe("zoo-005 is not running this project because it is offline.");
    expect(stoppedOnHostReason(undefined)).toBe("This tab's machine is not running this project.");
    expect(stoppedOnHostReason(undefined, true)).toBe("This tab's machine is not running this project.");
    for (const offline of [false, true]) {
      expect(stoppedOnHostReason("zoo-005", offline)).not.toMatch(/Start it on/);
    }
  });

  it("lets only the owner forget a machine's tabs", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      isOwner: false,
    });
    expect(result.tabs.get(2)?.stopped?.canForget).toBe(false);
  });

  it("disables an instance this viewer has no runtime access to, with the reason", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true, runtimeAccess: true }), instance(ZOO6, "zoo-006", { runtimeAccess: false })],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    });
    // The server's own refusal, so the disabled tab and a refused control
    // (sent before this viewer learned of the change) read the same.
    expect(result.tabs.get(2)).toMatchObject({
      state: "no_access",
      refusal: "Access denied: you cannot use zoo-006, which runs this project instance",
      badge: true,
    });
    expect(result.tabs.get(2)?.stopped).toBeUndefined();
    expect(result.tabs.get(1)?.state).toBe("running");
  });

  it("keeps a stopped host's untagged tabs under that host when a joined checkout runs first", () => {
    // The spec's laptop scenario: zoo-005's pre-upgrade tabs, zoo-005
    // stopped, a joined laptop attached with no tabs of its own.
    const result = hosts({
      panes: [pane(1, null, "zoo-005"), pane(2, null, "zoo-005")],
      instances: [instance(LAPTOP, "laptop", { primary: true })],
      machines: [machine(ZOO5, "zoo-005"), machine(LAPTOP, "laptop")],
    });
    expect(result.spansMachines).toBe(false);
    for (const id of [1, 2]) {
      const tab = result.tabs.get(id);
      expect(tab).toMatchObject({ hostname: "zoo-005", machineId: ZOO5, state: "stopped", badge: true });
      expect(tab?.refusal).toBe("zoo-005 is not running this project. Open the project again to start it there.");
      expect(tab?.stopped?.machineId).toBe(ZOO5);
    }
  });

  it("shows no host for an untagged tab with no attributed host, whose controls reach the primary", () => {
    const result = hosts({
      panes: [pane(1, null, null), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    });
    expect(result.tabs.get(1)).toMatchObject({ hostname: undefined, state: "running", badge: false });
  });

  it("says nothing per tab when nothing is attached and nothing spans machines", () => {
    const panes = [pane(1, ZOO5, "zoo-005")];
    // An older server: no instances ever.
    expect(hosts({ panes, instancesSupported: false, machines: [machine(ZOO5, "zoo-005")] }).tabs.size).toBe(0);
    // A stopped single-machine project: the Open button is the way to start it.
    expect(hosts({ panes, machines: [machine(ZOO5, "zoo-005")] }).tabs.size).toBe(0);
  });

  it("marks every tab stopped on its machine when a project spanning machines is stopped", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { projectInstancesAvailable: false })],
    });
    expect([...result.tabs.values()].map((tab) => [tab.hostname, tab.state, tab.badge]))
      .toEqual([["zoo-005", "stopped", true], ["zoo-006", "stopped", true]]);
    // Opening the project is what starts them; no tab offers its machine alone.
    for (const tab of result.tabs.values()) {
      expect(Object.keys(tab.stopped ?? {}).sort()).toEqual(["canForget", "hostname", "machineId"]);
    }
  });
});

describe("hostList", () => {
  it("joins host names for a sentence", () => {
    expect(hostList([])).toBe("");
    expect(hostList(["zoo-005"])).toBe("zoo-005");
    expect(hostList(["zoo-005", "zoo-006"])).toBe("zoo-005 and zoo-006");
    expect(hostList(["zoo-005", "zoo-006", "laptop"])).toBe("zoo-005, zoo-006 and laptop");
  });
});

describe("sessionIsOwned", () => {
  it("follows the share role, then the shared flag", () => {
    expect(sessionIsOwned({ shareRole: "owner", isShared: true })).toBe(true);
    expect(sessionIsOwned({ shareRole: "user" })).toBe(false);
    expect(sessionIsOwned({ isShared: true })).toBe(false);
    expect(sessionIsOwned({})).toBe(true);
    expect(sessionIsOwned(undefined)).toBe(false);
  });
});

describe("projectHostLabel", () => {
  it("names the primary host plus how many more instances run", () => {
    const label = projectHostLabel([
      instance(ZOO6, "zoo-006"),
      instance(ZOO5, "zoo-005", { primary: true }),
    ], "zoo-005");
    expect(label).toEqual({
      primaryHost: "zoo-005",
      additionalCount: 1,
      text: "zoo-005 +1",
      title: "Running on zoo-005, zoo-006",
      spoken: "zoo-005 and 1 more machine",
    });
  });

  it("falls back to the recorded host when no instance is listed", () => {
    expect(projectHostLabel(undefined, "zoo-005").text).toBe("zoo-005");
    expect(projectHostLabel([], undefined)).toEqual({ additionalCount: 0 });
  });
});

describe("machine choice presentation", () => {
  it("groups choices in rank order and drops empty groups", () => {
    const model = buildMachineChoices({
      projectId: PROJECT,
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [
        machine(ZOO5, "zoo-005"),
        machine(ZOO6, "zoo-006"),
        machine(LAPTOP, "laptop", { registered: false, daemonConnected: false }),
      ],
      instancesSupported: true,
      canClone: true,
    });
    expect(groupMachineChoices(model.choices).map((group) => [group.label, group.choices.map((choice) => choice.hostname)]))
      .toEqual([
        ["Running", ["zoo-005"]],
        ["Starts the project first", ["zoo-006"]],
        ["Unavailable", ["laptop"]],
      ]);
    const [laptop] = model.choices.filter((choice) => choice.hostname === "laptop");
    expect(machineChoiceDetail(laptop)).toBe("offline");
    // An older server starts that machine alone, and the menu keeps saying so.
    const [zoo6] = model.choices.filter((choice) => choice.hostname === "zoo-006");
    expect(zoo6.alsoStarts).toBeUndefined();
    expect(machineChoiceDetail(zoo6)).toBe("Registered · the project starts there, then the tab opens");
  });

  it("says a tab on a machine that is not running the project opens the project, and names what else starts", () => {
    const base = {
      projectId: PROJECT,
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006"), machine(LAPTOP, "laptop")],
      instancesSupported: true,
      canClone: false,
      openStateSupported: true,
    };
    const part = (machineId: string | undefined, hostname: string, value: string, connected = true) =>
      ({ machineId, hostname, part: value, ownsTabs: true, connected });

    // zoo-005 runs it; zoo-006 and the laptop own tabs and do not.
    const model = buildMachineChoices({
      ...base,
      projectMachines: [
        part(ZOO5, "zoo-005", "running"),
        part(ZOO6, "zoo-006", "not_started"),
        part(LAPTOP, "laptop", "not_started"),
      ],
    });
    expect(groupMachineChoices(model.choices).map((group) => [group.label, group.choices.map((choice) => choice.hostname)]))
      .toEqual([
        ["Running", ["zoo-005"]],
        ["Opens the project", ["laptop", "zoo-006"]],
      ]);
    const [laptop, zoo6] = model.choices.filter((choice) => choice.kind === "registered");
    expect(laptop.alsoStarts).toEqual(["zoo-006"]);
    expect(machineChoiceDetail(laptop))
      .toBe("Registered · opens the project, which also starts its tabs on zoo-006, then the tab opens");
    expect(machineChoiceDetail(zoo6))
      .toBe("Registered · opens the project, which also starts its tabs on laptop, then the tab opens");
    // A running instance is not an Open.
    expect(model.choices.find((choice) => choice.kind === "instance")?.alsoStarts).toBeUndefined();

    // Nothing else to start: an offline owner waits, a running one runs, and
    // a host known only by name is matched by that name.
    const alone = buildMachineChoices({
      ...base,
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      projectMachines: [
        part(ZOO5, "zoo-005", "running"),
        part(undefined, "ZOO-006", "not_started"),
        part(LAPTOP, "laptop", "waiting_offline", false),
      ],
    });
    const [only] = alone.choices.filter((choice) => choice.kind === "registered");
    expect(only.alsoStarts).toEqual([]);
    expect(machineChoiceDetail(only)).toBe("Registered · opens the project, then the tab opens");
    expect(groupMachineChoices(alone.choices).map((group) => group.label)).toEqual(["Running", "Opens the project"]);

    // The parts are optional: the group is still named for what the tab does.
    const unnamed = buildMachineChoices({ ...base, instances: [] });
    expect(groupMachineChoices(unnamed.choices)[0].label).toBe("Opens the project");
    expect(unnamed.choices.every((choice) => choice.alsoStarts?.length === 0)).toBe(true);
  });
});

describe("a clone choice that opens the project", () => {
  const input = {
    projectId: PROJECT,
    instances: [],
    // The laptop does not have the project; zoo-005 and zoo-006 own its tabs.
    machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006"), machine(LAPTOP, "laptop", { registered: false })],
    instancesSupported: true,
    canClone: true,
    projectMachines: [
      { machineId: ZOO5, hostname: "zoo-005", part: "not_started", ownsTabs: true, connected: true },
      { machineId: ZOO6, hostname: "zoo-006", part: "not_started", ownsTabs: true, connected: true },
    ],
  };
  const cloneOf = (model: ReturnType<typeof buildMachineChoices>) =>
    model.choices.find((choice) => choice.kind === "clone")!;

  it("says so in the machine step and names the machines whose tabs start", () => {
    const model = buildMachineChoices({ ...input, openStateSupported: true });
    expect(groupMachineChoices(model.choices).map((group) => [group.label, group.choices.map((choice) => choice.hostname)]))
      .toEqual([
        ["Opens the project", ["zoo-005", "zoo-006"]],
        ["Clone here", ["laptop"]],
      ]);
    expect(machineChoiceDetail(cloneOf(model))).toBe(
      "Not there yet · clones the repository and opens the project, which also starts its tabs on zoo-005 and zoo-006, then the tab opens",
    );

    // Nothing else to start: it still says the project is opened.
    const alone = buildMachineChoices({ ...input, openStateSupported: true, projectMachines: [] });
    expect(machineChoiceDetail(cloneOf(alone)))
      .toBe("Not there yet · clones the repository and opens the project, then the tab opens");

    // An older server starts the cloned machine alone, and the line is what
    // it was.
    const older = buildMachineChoices(input);
    expect(machineChoiceDetail(cloneOf(older))).toBe("Not there yet · clone the repository, then start it");
  });

  it("says so in the confirmation, before anything is cloned", () => {
    const model = buildMachineChoices({ ...input, openStateSupported: true });
    const copy = cloneHereCopy("laptop", "github.com/example/chiral-network", cloneOf(model).alsoStarts);
    expect(copy.title).toBe("Clone this project onto laptop?");
    expect(copy.body).toHaveLength(3);
    expect(copy.body[1]).toBe("The new tab opens the project, which also starts its tabs on zoo-005 and zoo-006.");
    expect(copy.body[2]).toBe("Cloning a large repository can take a few minutes.");

    expect(cloneHereCopy("laptop", "github.com/example/chiral-network", []).body[1])
      .toBe("The new tab opens the project.");
    // An older server: nothing about opening, as before.
    const older = cloneHereCopy("laptop", "github.com/example/chiral-network");
    expect(older.body).toHaveLength(2);
    expect(confirmationText(older)).not.toContain("opens the project");
  });
});

describe("launchStatus", () => {
  function request(overrides: Partial<AddPaneRequest>): AddPaneRequest {
    return {
      requestId: "launch-1",
      sessionId: SESSION,
      status: "pending",
      provider: "claude",
      kind: "terminal",
      label: "Claude 3",
      hostname: "zoo-006",
      startedAt: 1,
      updatedAt: 1,
      ...overrides,
    };
  }

  it("describes every step of a launch, and only final ones can be dismissed", () => {
    expect(launchStatus(request({ status: "cloning" }))).toMatchObject({ title: "Claude 3 on zoo-006", detail: "Cloning the repository onto zoo-006…", inFlight: true });
    expect(launchStatus(request({ status: "pending" }))).toMatchObject({ detail: "Requesting the tab…", inFlight: true });
    expect(launchStatus(request({ status: "starting" }))).toMatchObject({ detail: "Starting the project on zoo-006…", inFlight: true });
    expect(launchStatus(request({ status: "routed" }))).toMatchObject({ detail: "Opening…", inFlight: true });
    expect(launchStatus(request({ status: "failed", error: "zoo-006 is offline" }))).toMatchObject({ detail: "zoo-006 is offline", inFlight: false, tone: "error" });
    expect(launchStatus(request({ status: "unconfirmed", error: undefined }))).toMatchObject({ inFlight: false, tone: "warning" });
    expect(launchStatus(request({ status: "pending", hostname: undefined, label: undefined })).title).toBe("New tab");
  });
});

describe("instanceRebootTargets", () => {
  it("lists each attached instance with its own inventory, primary first", () => {
    const zoo5: CliLifecycleInventory = { persistent_terminal_hosting: true, panes: [] };
    const legacy: CliLifecycleInventory = { persistent_terminal_hosting: false, panes: [] };
    expect(instanceRebootTargets(
      [instance(ZOO6, "zoo-006", { runtimeAccess: false }), instance(ZOO5, "zoo-005", { primary: true })],
      { [ZOO5]: zoo5 },
    )).toEqual([
      { key: ZOO5, machineId: ZOO5, hostname: "zoo-005", primary: true, inventory: zoo5, disabledReason: undefined },
      { key: ZOO6, machineId: ZOO6, hostname: "zoo-006", primary: false, inventory: undefined, disabledReason: "Access denied: you cannot use zoo-006, which runs this project instance" },
    ]);
    expect(instanceRebootTargets([instance(undefined, "laptop", { primary: true })], { [LEGACY_MACHINE_KEY]: legacy })[0])
      .toMatchObject({ key: "host:laptop", machineId: undefined, inventory: legacy });
  });
});

describe("confirmation copy", () => {
  it("never promises a forgotten machine's tabs are gone for good", () => {
    const text = confirmationText(forgetMachineTabsCopy("zoo-006"));
    expect(text).toContain("Forget zoo-006's tabs?");
    expect(text).toContain("They come back if zoo-006 runs this project again.");
    expect(text).not.toMatch(/delete|permanent/i);
  });

  it("names the machine, the repository and the folder a clone asks for", () => {
    expect(cloneInstanceName("github.com/example/chiral-network")).toBe("chiral-network");
    expect(cloneInstanceName("gitlab.example.com/group/repo.git")).toBe("repo");
    const copy = cloneHereCopy("laptop", "github.com/example/chiral-network");
    expect(copy.title).toBe("Clone this project onto laptop?");
    // Only asked for: a separate project of the same repository often holds
    // that folder already, and the daemon then clones beside it.
    expect(copy.body[0]).toContain(
      "~/apas_projects/chiral-network (auto-suffixed, as chiral-network-2 and so on, if that folder exists)",
    );
    expect(copy.body[0]).toContain("no new branch");
  });
});
