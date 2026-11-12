import { readFileSync } from "node:fs";
import { resolve } from "node:path";
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
  projectStartHosts,
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

  it("refuses a stopped machine's tabs in the server's words and offers to start or forget it", () => {
    const result = hosts({
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    });
    const stopped = result.tabs.get(2);
    expect(stopped).toMatchObject({ hostname: "zoo-006", machineId: ZOO6, state: "stopped", badge: true });
    expect(stopped?.refusal).toBe("zoo-006 is not running this project. Start it on zoo-006 first.");
    expect(stopped?.stopped).toEqual({
      machineId: ZOO6,
      hostname: "zoo-006",
      projectId: PROJECT,
      startDisabledReason: undefined,
      canStart: true,
      canForget: true,
    });
    expect(result.tabs.get(1)?.refusal).toBeUndefined();
  });

  it("says why a stopped machine cannot be started from here", () => {
    const base = {
      panes: [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")],
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
    };
    const offline = hosts({ ...base, machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { daemonConnected: false, projectInstancesAvailable: false })] });
    expect(offline.tabs.get(2)).toMatchObject({ state: "offline" });
    expect(offline.tabs.get(2)?.stopped?.startDisabledReason).toBe("zoo-006 is offline.");

    // An older daemon cannot start the project beside a running instance.
    const older = hosts({ ...base, machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { projectInstancesAvailable: false })] });
    expect(older.tabs.get(2)?.stopped?.startDisabledReason).toBe("Reboot zoo-006 to update");

    const unregistered = hosts({ ...base, machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { registered: false })] });
    expect(unregistered.tabs.get(2)?.stopped).toMatchObject({
      projectId: undefined,
      startDisabledReason: "This project is not registered on zoo-006.",
    });

    // A machine missing from the viewer's machine list cannot be started by
    // this viewer at all; its tabs still say why they are stopped.
    const foreign = hosts({ ...base, machines: [machine(ZOO5, "zoo-005")] });
    expect(foreign.tabs.get(2)?.refusal).toBe(stoppedOnHostReason("zoo-006"));
    expect(foreign.tabs.get(2)?.stopped).toMatchObject({ canStart: false, canForget: true });
  });

  it("offers no Start beside an attached CLI without multi-machine support, as the new-tab picker", () => {
    // zoo-005's daemon is connected but older, so its instance owns the whole
    // project: the server would refuse zoo-006's instance beside it.
    const panes = [pane(1, ZOO5, "zoo-005"), pane(2, ZOO6, "zoo-006")];
    const machines = [machine(ZOO5, "zoo-005", { projectInstancesAvailable: false }), machine(ZOO6, "zoo-006")];
    const instances = [instance(ZOO5, "zoo-005", { primary: true })];
    const result = hosts({ panes, instances, machines });
    expect(result.tabs.get(2)).toMatchObject({ state: "stopped", refusal: stoppedOnHostReason("zoo-006") });
    expect(result.tabs.get(2)?.stopped).toMatchObject({
      canStart: true,
      projectId: PROJECT,
      startDisabledReason: "Reboot zoo-005 to update",
    });
    // The same machine in the new-tab picker, for the same reason.
    const picker = buildMachineChoices({
      projectId: PROJECT,
      instances,
      machines,
      instancesSupported: true,
      canClone: false,
    });
    expect(picker.choices.find((choice) => choice.machineId === ZOO6)?.disabledReason)
      .toBe(result.tabs.get(2)?.stopped?.startDisabledReason);

    // A sole CLI that reported no machine is older by definition.
    const machineless = hosts({
      panes,
      instances: [instance(undefined, "zoo-005", { primary: true })],
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
    });
    expect(machineless.tabs.get(2)?.stopped?.startDisabledReason).toBe("Reboot zoo-005 to update");
  });

  it("calls a machine that still runs the project disconnected, with nothing to start", () => {
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
      // A running project has nothing to start. Its tabs can still be
      // forgotten: a daemon that dropped silently reads this way for hours.
      expect(tab?.stopped).toEqual({
        machineId: ZOO6,
        hostname: "zoo-006",
        projectId: undefined,
        startDisabledReason: undefined,
        canStart: false,
        canForget: true,
      });
    }
    expect(result.tabs.get(1)?.state).toBe("running");

    // Once the daemon reports it stopped, it is stopped, with Start.
    const stopped = hosts({ panes, instances, machines: [running(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")] });
    expect(stopped.tabs.get(2)?.state).toBe("stopped");
    expect(stopped.tabs.get(2)?.stopped?.canStart).toBe(true);

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

  it("refuses a stopped machine's tab in the words of the protocol's own refusal", () => {
    // The golden frame for typed input refused because its tab's machine is
    // not running the project: the disabled control and that refusal agree.
    const refusal = JSON.parse(readFileSync(
      resolve(__dirname, "../../../protocol/fixtures/server-mutation-ack-input.json"),
      "utf8",
    )) as { error: string };
    expect(stoppedOnHostReason("zoo-005")).toBe(refusal.error);
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
      expect(tab?.refusal).toBe("zoo-005 is not running this project. Start it on zoo-005 first.");
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
    // Nothing runs beside it, so an older daemon may start the project as it
    // always could.
    expect(result.tabs.get(2)?.stopped?.startDisabledReason).toBeUndefined();
  });
});

describe("projectStartHosts", () => {
  it("names the machines that own tabs, not a tab-less last primary", () => {
    // All tabs are zoo-005's; the laptop, joined with no tabs, stopped last
    // and so is the session's recorded host. Open starts zoo-005 only.
    const machines = [machine(ZOO5, "zoo-005"), machine(LAPTOP, "laptop")];
    const panes = [pane(1, ZOO5, "zoo-005"), pane(2, ZOO5, "zoo-005")];
    expect(projectStartHosts({ panes, machines, projectId: PROJECT })).toEqual(["zoo-005"]);

    // Nothing qualifies when the owner is offline or older: the server then
    // starts the machine the request names, which the caller shows instead.
    for (const owner of [
      machine(ZOO5, "zoo-005", { daemonConnected: false, projectInstancesAvailable: false }),
      machine(ZOO5, "zoo-005", { projectInstancesAvailable: false }),
      machine(ZOO5, "zoo-005", { registered: false }),
    ]) {
      expect(projectStartHosts({ panes, machines: [owner, machine(LAPTOP, "laptop")], projectId: PROJECT })).toEqual([]);
    }
    expect(projectStartHosts({ panes: [], machines, projectId: PROJECT })).toEqual([]);
  });

  it("counts an untagged tab for the machine of its attributed host, and lists every owner", () => {
    const machines = [machine(ZOO6, "zoo-006"), machine(ZOO5, "zoo-005"), machine(LAPTOP, "laptop")];
    expect(projectStartHosts({
      panes: [pane(1, null, " ZOO-005 "), pane(2, ZOO6, "zoo-006"), pane(3, null, null)],
      machines,
      projectId: PROJECT,
    })).toEqual(["zoo-005", "zoo-006"]);
    // Another id is another project's inventory entry.
    expect(projectStartHosts({ panes: [pane(2, ZOO6, "zoo-006")], machines, projectId: SESSION })).toEqual([]);
  });

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
