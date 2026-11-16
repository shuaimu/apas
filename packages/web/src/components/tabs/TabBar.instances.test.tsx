import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TabBar, type ProjectControls } from "./TabBar";
import { useStore, type CliLifecycleInventory, type MachineWithProjects, type PaneConfig } from "@/lib/store";
import { buildMachineChoices, type MachineChoiceModel } from "@/lib/machineChoice";
import type { SessionInstance } from "@/lib/projectInstances";
import { describeTabHosts, instanceRebootTargets, type RebootTarget, type TabHost } from "@/lib/tabHosts";
import {
  buildProjectOpenModel,
  type ProjectMachinePartInfo,
  type ProjectOpenModelInput,
  type ProjectOpenReport,
} from "@/lib/projectOpenState";

/**
 * The tab bar of a project whose tabs run on several machines. The host
 * state and the machine step come from the real helpers the project view
 * uses, so these exercise what a user would see for a given roster.
 */

const PROJECT = "30000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const LAPTOP = "50000000-0000-4000-8000-000000000007";

function tab(pane_id: number, label: string, machine_id: string | null, hostname: string | null): PaneConfig {
  return {
    pane_id,
    provider: "claude",
    mode: "interactive",
    kind: "terminal",
    session_id: `pane-${pane_id}`,
    is_paused: false,
    label,
    machine_id,
    hostname,
  };
}

function instance(machineId: string, hostname: string, overrides: Partial<SessionInstance> = {}): SessionInstance {
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

const TABS = [tab(1, "Claude 1", ZOO5, "zoo-005"), tab(2, "Claude 2", ZOO6, "zoo-006")];

function hostsFor(instances: SessionInstance[], machines: MachineWithProjects[], isOwner = true): Map<number, TabHost> {
  return describeTabHosts({
    panes: TABS,
    instances,
    machines,
    instancesSupported: true,
    projectIds: [PROJECT],
    isOwner,
  }).tabs;
}

function renderBar(props: Partial<Parameters<typeof TabBar>[0]> = {}) {
  const handlers = {
    onSelectTab: vi.fn(),
    onCloseTab: vi.fn(),
    onAddTab: vi.fn(),
    onForgetMachine: vi.fn(),
    onRebootCli: vi.fn(),
  };
  const result = render(
    <TabBar
      tabs={TABS}
      activeTabId={1}
      paneStatuses={{}}
      pausedPanes={[]}
      {...handlers}
      {...props}
    />,
  );
  return { ...result, ...handlers };
}

function tabElement(container: HTMLElement, paneId: number): HTMLElement {
  const element = container.querySelector(`[data-tab-id="${paneId}"]`);
  expect(element).toBeTruthy();
  return element as HTMLElement;
}

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => useStore.setState({ sessionId: null, projectPolicies: {} }));
});

describe("host badges", () => {
  it("shows each tab's host once the roster spans machines", () => {
    const { container } = renderBar({
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      ),
    });
    const first = within(tabElement(container, 1)).getByTestId("tab-host-badge");
    expect(first.textContent).toBe("zoo-005");
    expect(first.getAttribute("title")).toBe("Runs on zoo-005");
    expect(within(tabElement(container, 2)).getByTestId("tab-host-badge").textContent).toBe("zoo-006");
  });

  it("shows no badge for a project on one machine", () => {
    const { container } = renderBar({
      tabs: [TABS[0]],
      tabHosts: describeTabHosts({
        panes: [TABS[0]],
        instances: [instance(ZOO5, "zoo-005", { primary: true })],
        machines: [machine(ZOO5, "zoo-005")],
        instancesSupported: true,
        projectIds: [PROJECT],
        isOwner: true,
      }).tabs,
    });
    expect(container.querySelector('[data-testid="tab-host-badge"]')).toBeNull();
  });

  it("names a stopped machine's state on its tabs and refuses to close them", () => {
    const { container, onCloseTab } = renderBar({
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true })],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      ),
    });
    const stopped = tabElement(container, 2);
    const reason = "zoo-006 is not running this project. Open the project again to start it there.";
    expect(stopped.getAttribute("title")).toBe(reason);
    // Named with its host's state, and the state is in words, not only colour.
    expect(screen.getByRole("button", { name: "Claude 2 — zoo-006 is not running this project" })).toBe(stopped);
    const badge = within(stopped).getByTestId("tab-host-badge");
    expect(badge.textContent).toBe("zoo-006· stopped");
    expect(badge.getAttribute("title")).toBe("zoo-006 is not running this project");
    // The badge gives way before the name (jsdom has no layout, so the
    // classes stand in): a stopped badge used to leave the name "C…".
    expect(stopped.className).toContain("max-w-[320px]");
    expect(badge.className.split(" ")).toContain("shrink");
    expect(badge.className).not.toContain("flex-shrink-0");
    expect(within(stopped).getByText("Claude 2").className.split(" ")).toContain("shrink-0");
    expect(screen.getByRole("button", { name: "Claude 1 — Runs on zoo-005" })).toBe(tabElement(container, 1));

    const close = stopped.querySelector(`[title="${reason}"][aria-disabled="true"]`);
    expect(close).toBeTruthy();
    fireEvent.click(close as Element);
    expect(onCloseTab).not.toHaveBeenCalled();

    // The running machine's tab still closes as before.
    fireEvent.click(tabElement(container, 1).querySelector('[title="Close tab"]') as Element);
    expect(onCloseTab).toHaveBeenCalledWith(1);
  });
});

describe("a stopped machine's tab menu", () => {
  it("offers Forget <host>'s tabs but no Start on <host>, and disables pane controls with the reason", () => {
    const { container, onForgetMachine, onCloseTab } = renderBar({
      onRenameTab: vi.fn(),
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true })],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      ),
    });
    fireEvent.contextMenu(tabElement(container, 2));
    expect(screen.getByText("zoo-006 is not running this project. Open the project again to start it there.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Rename" }) as HTMLButtonElement).disabled).toBe(true);
    const close = screen.getByRole("button", { name: "Close" }) as HTMLButtonElement;
    expect(close.disabled).toBe(true);
    fireEvent.click(close);
    expect(onCloseTab).not.toHaveBeenCalled();

    // One machine is never started alone: opening the project again is
    // what starts it, and the refusal above says so.
    expect(screen.queryByRole("button", { name: /^Start on/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Forget zoo-006's tabs…" }));
    expect(onForgetMachine).toHaveBeenCalledWith(ZOO6, "zoo-006");
  });

  it("says an offline machine's tabs wait for it, with no Start on <host>", () => {
    const { container } = renderBar({
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true })],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { daemonConnected: false })],
        false,
      ),
    });
    fireEvent.contextMenu(tabElement(container, 2));
    expect(screen.getByText("zoo-006 is not running this project because it is offline.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Start on/ })).toBeNull();
    // Only the owner may forget a machine's tabs.
    expect(screen.queryByRole("button", { name: /^Forget/ })).toBeNull();
  });

  it("leaves a running tab's menu as it was", () => {
    const { container } = renderBar({
      onRenameTab: vi.fn(),
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true })],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      ),
    });
    fireEvent.contextMenu(tabElement(container, 1));
    expect((screen.getByRole("button", { name: "Rename" }) as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByRole("button", { name: /^Start on/ })).toBeNull();
  });
});

describe("the new-tab machine step", () => {
  function choices(overrides: Partial<Parameters<typeof buildMachineChoices>[0]> = {}): MachineChoiceModel {
    return buildMachineChoices({
      projectId: PROJECT,
      instances: [instance(ZOO5, "zoo-005", { primary: true })],
      machines: [
        machine(ZOO5, "zoo-005"),
        machine(ZOO6, "zoo-006"),
        machine(LAPTOP, "laptop", { registered: false }),
        machine("50000000-0000-4000-8000-000000000008", "old-box", { projectInstancesAvailable: false }),
      ],
      instancesSupported: true,
      canClone: true,
      selectedPane: TABS[0],
      ...overrides,
    });
  }

  function openInteractive() {
    fireEvent.click(screen.getByTitle("New tab"));
  }

  it("groups the machines, defaults to the selected tab's machine and sends the pick", () => {
    const { onAddTab } = renderBar({ machineChoices: choices() });
    openInteractive();

    const step = screen.getByRole("group", { name: "Run on" });
    expect(within(step).getByRole("group", { name: "Running" })).toBeTruthy();
    expect(within(step).getByRole("group", { name: "Starts the project first" })).toBeTruthy();
    expect(within(step).getByRole("group", { name: "Clone here" })).toBeTruthy();
    const zoo5 = within(step).getByRole("radio", { name: /^zoo-005/ }) as HTMLInputElement;
    expect(zoo5.checked).toBe(true);
    // An older daemon is listed, disabled, with what to do about it.
    const old = within(step).getByRole("radio", { name: /^old-box/ }) as HTMLInputElement;
    expect(old.disabled).toBe(true);
    expect(within(within(step).getByRole("group", { name: "Unavailable" })).getByText("Reboot old-box to update")).toBeTruthy();

    fireEvent.click(within(step).getByRole("radio", { name: /^zoo-006/ }));
    fireEvent.click(screen.getByText("Claude"));
    expect(onAddTab).toHaveBeenCalledWith(
      "claude", undefined, undefined, "terminal",
      expect.objectContaining({ machineId: ZOO6, kind: "registered", hostname: "zoo-006" }),
    );
  });

  it("follows a default that arrives after the menu opened", () => {
    // The menu can open before the machine list and instances have arrived.
    const { onAddTab, rerender } = renderBar({ machineChoices: choices({ instancesSupported: false }) });
    openInteractive();
    expect(screen.queryByRole("group", { name: "Run on" })).toBeNull();

    rerender(
      <TabBar
        tabs={TABS}
        activeTabId={1}
        paneStatuses={{}}
        pausedPanes={[]}
        onSelectTab={vi.fn()}
        onCloseTab={vi.fn()}
        onAddTab={onAddTab}
        machineChoices={choices()}
      />,
    );
    const step = screen.getByRole("group", { name: "Run on" });
    expect((within(step).getByRole("radio", { name: /^zoo-005/ }) as HTMLInputElement).checked).toBe(true);
    const claude = screen.getByText("Claude").closest("button") as HTMLButtonElement;
    expect(claude.disabled).toBe(false);
    fireEvent.click(claude);
    expect(onAddTab).toHaveBeenCalledWith(
      "claude", undefined, undefined, "terminal",
      expect.objectContaining({ machineId: ZOO5, kind: "instance" }),
    );
  });

  it("is not shown with a single eligible machine, and the menu is unchanged", () => {
    const { onAddTab } = renderBar({
      machineChoices: choices({ machines: [machine(ZOO5, "zoo-005")], canClone: false }),
    });
    openInteractive();
    expect(screen.queryByRole("group", { name: "Run on" })).toBeNull();
    fireEvent.click(screen.getByText("Claude"));
    expect(onAddTab).toHaveBeenCalledWith("claude", undefined, undefined, "terminal", undefined);
  });

  it("says a stopped project starts first, even when its machine is the only choice", () => {
    // `+` on a stopped project starts it, relaunching its saved tabs, so the
    // step that says so is never hidden.
    const { onAddTab } = renderBar({
      machineChoices: choices({ instances: [], machines: [machine(ZOO5, "zoo-005")], canClone: false }),
    });
    openInteractive();
    const step = screen.getByRole("group", { name: "Run on" });
    const starts = within(step).getByRole("group", { name: "Starts the project first" });
    const zoo5 = within(starts).getByRole("radio", { name: /^zoo-005/ }) as HTMLInputElement;
    expect(zoo5.checked).toBe(true);
    expect(within(starts).getByText("Registered · the project starts there, then the tab opens")).toBeTruthy();

    fireEvent.click(screen.getByText("Claude"));
    expect(onAddTab).toHaveBeenCalledWith(
      "claude", undefined, undefined, "terminal",
      expect.objectContaining({ machineId: ZOO5, kind: "registered" }),
    );
  });

  it("says a tab on a machine that is not running opens the project, naming the other machines", () => {
    // Against a server that records open states, a tab on laptop opens the
    // project, which also starts zoo-006's tabs.
    const laptop = machine(LAPTOP, "laptop");
    const { onAddTab } = renderBar({
      machineChoices: choices({
        machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006"), laptop],
        canClone: false,
        openStateSupported: true,
        projectMachines: [
          { machineId: ZOO5, hostname: "zoo-005", part: "running", ownsTabs: true, connected: true },
          { machineId: ZOO6, hostname: "zoo-006", part: "not_started", ownsTabs: true, connected: true },
        ],
      }),
    });
    openInteractive();
    const step = screen.getByRole("group", { name: "Run on" });
    expect(within(step).queryByRole("group", { name: "Starts the project first" })).toBeNull();
    const opens = within(step).getByRole("group", { name: "Opens the project" });
    expect(within(opens).getByText(
      "Registered · opens the project, which also starts its tabs on zoo-006, then the tab opens",
    )).toBeTruthy();
    // zoo-006 itself starts no other machine: zoo-005 already runs it.
    expect(within(opens).getByText("Registered · opens the project, then the tab opens")).toBeTruthy();

    fireEvent.click(within(opens).getByRole("radio", { name: /^laptop/ }));
    fireEvent.click(screen.getByText("Claude"));
    expect(onAddTab).toHaveBeenCalledWith(
      "claude", undefined, undefined, "terminal",
      expect.objectContaining({ machineId: LAPTOP, kind: "registered", alsoStarts: ["zoo-006"] }),
    );
  });

  it("requires picking a machine when the only choice is a clone", () => {
    const { onAddTab } = renderBar({
      machineChoices: choices({ instances: [], machines: [machine(LAPTOP, "laptop", { registered: false })] }),
    });
    openInteractive();
    const claude = screen.getByText("Claude").closest("button") as HTMLButtonElement;
    expect(claude.disabled).toBe(true);
    expect(screen.getByText("Choose where the tab runs first.")).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: /^laptop/ }));
    fireEvent.click(screen.getByText("Claude"));
    expect(onAddTab).toHaveBeenCalledWith(
      "claude", undefined, undefined, "terminal",
      expect.objectContaining({ machineId: LAPTOP, kind: "clone" }),
    );
  });
});

describe("what each provider needs on the new tab's machine", () => {
  const MAC = "50000000-0000-4000-8000-0000000000aa";
  const MAC_HOST = "Shuais-MacBook-Air-2.local";
  const OMP_POLICY = ["terminal:claude:official:default", "terminal:omp:official:default"];

  function withProviders(entry: MachineWithProjects, providers: MachineWithProjects["machine"]["providers"]): MachineWithProjects {
    return { ...entry, machine: { ...entry.machine, providers } };
  }

  function allowProfiles() {
    act(() => useStore.setState({
      sessionId: "session-1",
      projectPolicies: {
        "session-1": {
          teamAvailable: false,
          allowedLaunchProfiles: OMP_POLICY,
          version: 1,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
    }));
  }

  function singleMac(providers: MachineWithProjects["machine"]["providers"]): MachineChoiceModel {
    // The Mac is the project's only running instance, so the menu shows no
    // machine step; the tab still goes to the Mac.
    return buildMachineChoices({
      projectId: PROJECT,
      instances: [instance(MAC, MAC_HOST, { primary: true })],
      machines: [withProviders(machine(MAC, MAC_HOST), providers)],
      instancesSupported: true,
      canClone: false,
      selectedPane: null,
    });
  }

  function providerButton(name: string): HTMLButtonElement {
    const menu = screen.getByLabelText("Add agent choices");
    const button = within(menu).getAllByRole("button")
      // The OMP icon draws its mark as text, so match within the label.
      .find((candidate) => candidate.textContent?.includes(name) && !candidate.textContent.includes(`${name} /`));
    expect(button).toBeTruthy();
    return button as HTMLButtonElement;
  }

  it("offers OMP with a note when the machine can install it", () => {
    allowProfiles();
    const { onAddTab } = renderBar({
      machineChoices: singleMac([
        { provider: "claude", installed: true, version: "2.1.233", canInstall: false },
        { provider: "omp", installed: false, canInstall: true },
      ]),
    });
    fireEvent.click(screen.getByTitle("New tab"));
    expect(screen.queryByRole("group", { name: "Run on" })).toBeNull();
    const omp = providerButton("OMP");
    expect(omp.disabled).toBe(false);
    expect(within(omp).getByText("Installs OMP first")).toBeTruthy();
    expect(omp.title).toContain("You may need to log in to OMP in its terminal.");
    // Claude is installed: as it always was.
    expect(providerButton("Claude").textContent).toBe("Claude");
    expect(providerButton("Claude").title).toBe("");

    fireEvent.click(omp);
    expect(onAddTab).toHaveBeenCalledWith(
      "omp", undefined, undefined, "terminal",
      undefined,
    );
  });

  it("disables OMP with the reason when the machine has not got it and cannot install it", () => {
    allowProfiles();
    const { onAddTab } = renderBar({
      machineChoices: singleMac([
        { provider: "claude", installed: true, canInstall: false },
        { provider: "omp", installed: false, canInstall: false },
      ]),
    });
    fireEvent.click(screen.getByTitle("New tab"));
    const omp = providerButton("OMP");
    expect(omp.disabled).toBe(true);
    expect(within(omp).getByText(`OMP is not installed on ${MAC_HOST}.`)).toBeTruthy();
    expect(omp.title).toBe(`OMP is not installed on ${MAC_HOST}.`);
    fireEvent.click(omp);
    expect(onAddTab).not.toHaveBeenCalled();
    expect(providerButton("Claude").disabled).toBe(false);
  });

  it("leaves the menu as it was when the machine reports nothing", () => {
    allowProfiles();
    renderBar({ machineChoices: singleMac(undefined) });
    fireEvent.click(screen.getByTitle("New tab"));
    const omp = providerButton("OMP");
    expect(omp.disabled).toBe(false);
    expect(omp.textContent).not.toMatch(/install/i);
    expect(omp.title).toBe("");
  });

  it("follows the machine picked in the step", () => {
    allowProfiles();
    const { onAddTab } = renderBar({
      machineChoices: buildMachineChoices({
        projectId: PROJECT,
        instances: [instance(ZOO5, "zoo-005", { primary: true }), instance(MAC, MAC_HOST)],
        machines: [
          withProviders(machine(ZOO5, "zoo-005"), [{ provider: "omp", installed: true, version: "18.20.0", canInstall: true }]),
          withProviders(machine(MAC, MAC_HOST), [{ provider: "omp", installed: false, canInstall: false }]),
        ],
        instancesSupported: true,
        canClone: false,
        selectedPane: TABS[0],
      }),
    });
    fireEvent.click(screen.getByTitle("New tab"));
    expect(providerButton("OMP").disabled).toBe(false);

    fireEvent.click(screen.getByRole("radio", { name: new RegExp(`^${MAC_HOST}`) }));
    expect(providerButton("OMP").disabled).toBe(true);
    expect(providerButton("OMP").title).toBe(`OMP is not installed on ${MAC_HOST}.`);
    fireEvent.click(providerButton("Claude"));
    expect(onAddTab).toHaveBeenCalledWith(
      "claude", undefined, undefined, "terminal",
      expect.objectContaining({ machineId: MAC }),
    );
  });
});

describe("rebooting one machine's instance", () => {
  const zoo5Inventory: CliLifecycleInventory = {
    persistent_terminal_hosting: true,
    panes: [{ pane_id: 1, mode: "live_adoptable" }],
  };
  const zoo6Inventory: CliLifecycleInventory = {
    persistent_terminal_hosting: true,
    panes: [
      { pane_id: 2, mode: "restart_required_on_cli_reboot" },
      { pane_id: 3, mode: "restart_required_on_cli_reboot" },
    ],
  };

  function targets(): RebootTarget[] {
    return instanceRebootTargets(
      [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
      { [ZOO5]: zoo5Inventory, [ZOO6]: zoo6Inventory },
    );
  }

  it("confirms against that machine's own panes and reboots only it", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const { onRebootCli } = renderBar({ showRebootButton: true, rebootTargets: targets(), lifecycleInventory: zoo5Inventory });
    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    expect(screen.getByText("Reboot CLI on zoo-005")).toBeTruthy();
    fireEvent.click(screen.getByText("Reboot CLI on zoo-006"));

    const message = String(confirm.mock.calls[0][0]);
    expect(message).toContain("Reboot the project CLI on zoo-006?");
    expect(message).toContain("0 terminal panes will stay live and be reattached.");
    expect(message).toContain("2 terminal panes will restart and resume where supported.");
    expect(message).toContain("The instance on zoo-005 keeps running.");
    expect(onRebootCli).toHaveBeenCalledWith(ZOO6);
  });

  it("keeps the single reboot entry for one instance", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const { onRebootCli } = renderBar({
      showRebootButton: true,
      rebootTargets: instanceRebootTargets([instance(ZOO5, "zoo-005", { primary: true })], { [ZOO5]: zoo5Inventory }),
      lifecycleInventory: zoo5Inventory,
    });
    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    fireEvent.click(screen.getByText("Reboot CLI"));
    expect(String(confirm.mock.calls[0][0])).toContain("Reboot the project CLI?");
    expect(onRebootCli).toHaveBeenCalledWith();
  });

  it("names the machine of a pending operation and blocks only that machine's reboot", () => {
    renderBar({
      showRebootButton: true,
      rebootTargets: targets(),
      lifecycleStatus: {
        sessionId: "session",
        requestId: "request",
        machineId: ZOO6,
        operation: "reboot_cli",
        phase: "preparing",
        message: "Preparing the update",
        startedAt: 1,
        updatedAt: 1,
      },
    });
    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    expect(screen.getByRole("status").textContent).toBe("zoo-006: Preparing the update");
    expect((screen.getByText("Reboot CLI on zoo-006").closest("button") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByText("Reboot CLI on zoo-005").closest("button") as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("opening and closing a project on several machines", () => {
  const NOTHING_DELETED = "Nothing is deleted. Its tabs come back when you open it again.";

  function part(
    machineId: string,
    hostname: string,
    state: ProjectMachinePartInfo["part"],
    overrides: Partial<ProjectMachinePartInfo> = {},
  ): ProjectMachinePartInfo {
    return { machineId, hostname, part: state, ownsTabs: true, connected: true, ...overrides };
  }

  /** The controls the project view builds for a server report. */
  function controlsFor(
    report: Partial<ProjectOpenReport> | undefined,
    overrides: Partial<ProjectOpenModelInput> = {},
    extra: Partial<ProjectControls> = {},
  ) {
    const model = buildProjectOpenModel({
      supported: true,
      projectIds: [PROJECT],
      projectName: "apas",
      report: report ? { projectId: PROJECT, machines: [], ...report } : undefined,
      // The session list carries the same state as the frame; a frame that
      // disagrees with it is not used.
      session: { openState: report?.state },
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      target: { machineId: ZOO5, projectId: PROJECT, hostname: "zoo-005" },
      ...overrides,
    });
    const onOpen = vi.fn();
    const onClose = vi.fn();
    const view = renderBar({
      projectControls: {
        recordedState: model.recorded ? model.stateLabel : undefined,
        openAction: model.openAction,
        closeAction: model.closeAction,
        onOpen,
        onClose,
        ...extra,
      },
    });
    return { ...view, model, onOpen, onClose };
  }

  function dialogLines(): (string | null)[] {
    return Array.from(screen.getByRole("dialog").querySelectorAll("p")).map((line) => line.textContent);
  }

  function projectButtons(): string[] {
    return ["Open", "Keep open", "Open again", "Close", "Close again"]
      .filter((name) => screen.queryByRole("button", { name }) !== null);
  }

  it("offers Open for a closed project and names the machines whose tabs start", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const { onOpen } = controlsFor({
      state: "closed",
      machines: [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")],
    });
    expect(projectButtons()).toEqual(["Open"]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Closed");

    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("dialog", { name: "Open apas?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    expect(onOpen).not.toHaveBeenCalled();
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Open" }));
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("says of an offline tab owner that its tabs start when it reconnects", () => {
    controlsFor({
      state: "closed",
      machines: [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started", { connected: false })],
    });
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(dialogLines()).toEqual([
      "Its tabs start on zoo-005.",
      "zoo-006 is offline. Its tabs start when zoo-006 reconnects.",
    ]);
  });

  it("offers only Close for an open project that every tab owner runs, naming the machines", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const { onClose } = controlsFor({
      state: "open",
      machines: [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")],
    });
    expect(projectButtons()).toEqual(["Close"]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.getByRole("dialog", { name: "Close apas?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["This stops its agents on zoo-005 and zoo-006.", NOTHING_DELETED]);

    // Declining stops nothing.
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }));
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("puts the autopilot note before the line that says nothing is deleted", () => {
    const note = "Claude 1 is on autopilot. Its autopilot comes back paused when the project is opened again.";
    controlsFor(
      { state: "open", machines: [part(ZOO5, "zoo-005", "running")] },
      {},
      { closeNote: note },
    );
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(dialogLines()).toEqual(["This stops its agents on zoo-005.", note, NOTHING_DELETED]);
  });

  it("offers Open again and Close while a connected tab owner is not running an open project", () => {
    controlsFor({
      state: "open",
      machines: [
        part(ZOO5, "zoo-005", "running"),
        part(ZOO6, "zoo-006", "not_started", { reason: "zoo-006 refused the start." }),
      ],
    });
    expect(projectButtons()).toEqual(["Open again", "Close"]);
    // No control starts or stops one machine alone.
    expect(screen.queryByRole("button", { name: /zoo-006/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Open again" }));
    expect(screen.getByRole("dialog", { name: "Open apas again?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["zoo-005 already runs it.", "Its tabs start on zoo-006."]);
  });

  it("does not offer Open again for a machine that is only waiting to reconnect", () => {
    controlsFor({
      state: "open",
      machines: [
        part(ZOO5, "zoo-005", "running"),
        part(ZOO6, "zoo-006", "waiting_offline", { connected: false }),
      ],
    });
    expect(projectButtons()).toEqual(["Close"]);
  });

  it("offers Open again and Close for an open project that runs nowhere", () => {
    controlsFor({ state: "open", machines: [part(ZOO5, "zoo-005", "not_started")] });
    expect(projectButtons()).toEqual(["Open again", "Close"]);
  });

  it("offers Keep open, with what it changes, for a running project nobody has recorded", () => {
    const { onOpen } = controlsFor({
      machines: [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")],
    });
    expect(projectButtons()).toEqual(["Keep open", "Close"]);
    // Nobody has recorded a state, so none is shown as recorded.
    expect(screen.queryByTestId("project-open-state")).toBeNull();
    const sentence = "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.";
    const keep = screen.getByRole("button", { name: "Keep open" });
    expect(keep.getAttribute("title")).toBe(sentence);

    fireEvent.click(keep);
    expect(screen.getByRole("dialog", { name: "Keep apas open?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["This records the project as open. Nothing starts now.", sentence]);
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Keep open" }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("offers Open and Close for a project nobody has recorded that runs on one of two machines", () => {
    controlsFor({
      machines: [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")],
    });
    expect(projectButtons()).toEqual(["Open", "Close"]);
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(dialogLines()).toEqual([
      "zoo-005 already runs it.",
      "Its tabs start on zoo-006.",
      "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.",
    ]);
  });

  it("offers Close again while a machine still runs a closed project", () => {
    controlsFor({
      state: "closed",
      machines: [
        part(ZOO5, "zoo-005", "not_started"),
        part(ZOO6, "zoo-006", "still_running", { reason: "The account that closed it may not use zoo-006." }),
      ],
    });
    expect(projectButtons()).toEqual(["Open", "Close again"]);
    fireEvent.click(screen.getByRole("button", { name: "Close again" }));
    expect(screen.getByRole("dialog", { name: "Close apas again?" })).toBeTruthy();
    const lines = dialogLines();
    expect(lines[0]).toBe("zoo-006 still runs this project.");
    expect(lines.at(-1)).toBe(NOTHING_DELETED);
  });

  it("never offers Keep open against a server that records no open state", () => {
    // No frame and no capability: the project is shown from what runs.
    controlsFor(undefined, {
      supported: false,
      session: { isActive: true },
      panes: TABS,
      instances: [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")],
    });
    expect(projectButtons()).toEqual(["Close"]);
    expect(screen.queryByTestId("project-open-state")).toBeNull();
  });

  it("offers Delete project… only where the project view passes it, and closing never calls it", () => {
    const onDelete = vi.fn();
    const report = { state: "open" as const, machines: [part(ZOO5, "zoo-005", "running")] };
    const owner = controlsFor(report, {}, { onDelete });
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
    expect(owner.onClose).toHaveBeenCalledTimes(1);
    expect(onDelete).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete project…" }));
    expect(onDelete).toHaveBeenCalledTimes(1);
    // Deleting is its own dialog, which the project view owns.
    expect(screen.queryByRole("dialog")).toBeNull();
    owner.unmount();

    controlsFor(report);
    expect(screen.queryByRole("button", { name: /Delete/ })).toBeNull();
  });
});
