import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";
import { useStore, type CliLifecycleInventory, type MachineWithProjects, type PaneConfig } from "@/lib/store";
import { buildMachineChoices, type MachineChoiceModel } from "@/lib/machineChoice";
import type { SessionInstance } from "@/lib/projectInstances";
import { describeTabHosts, instanceRebootTargets, type RebootTarget, type TabHost } from "@/lib/tabHosts";

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
    onStartMachine: vi.fn(),
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
    const reason = "zoo-006 is not running this project. Start it on zoo-006 first.";
    expect(stopped.getAttribute("title")).toBe(reason);
    // Named with its host's state, and the state is in words, not only colour.
    expect(screen.getByRole("button", { name: "Claude 2 — zoo-006 is not running this project" })).toBe(stopped);
    const badge = within(stopped).getByTestId("tab-host-badge");
    expect(badge.textContent).toBe("zoo-006· stopped");
    expect(badge.getAttribute("title")).toBe("zoo-006 is not running this project");
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
  it("offers Start on <host> and Forget <host>'s tabs, and disables pane controls with the reason", () => {
    const { container, onStartMachine, onForgetMachine, onCloseTab } = renderBar({
      onRenameTab: vi.fn(),
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true })],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      ),
    });
    fireEvent.contextMenu(tabElement(container, 2));
    expect(screen.getByText("zoo-006 is not running this project. Start it on zoo-006 first.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Rename" }) as HTMLButtonElement).disabled).toBe(true);
    const close = screen.getByRole("button", { name: "Close" }) as HTMLButtonElement;
    expect(close.disabled).toBe(true);
    fireEvent.click(close);
    expect(onCloseTab).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Start on zoo-006" }));
    expect(onStartMachine).toHaveBeenCalledWith(ZOO6, PROJECT, "zoo-006");

    fireEvent.contextMenu(tabElement(container, 2));
    fireEvent.click(screen.getByRole("button", { name: "Forget zoo-006's tabs…" }));
    expect(onForgetMachine).toHaveBeenCalledWith(ZOO6, "zoo-006");
  });

  it("disables Start on <host> with the reason when that machine cannot start it", () => {
    const { container, onStartMachine } = renderBar({
      tabHosts: hostsFor(
        [instance(ZOO5, "zoo-005", { primary: true })],
        [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006", { daemonConnected: false })],
        false,
      ),
    });
    fireEvent.contextMenu(tabElement(container, 2));
    const start = screen.getByRole("button", { name: "Start on zoo-006" }) as HTMLButtonElement;
    expect(start.disabled).toBe(true);
    expect(start.title).toBe("zoo-006 is offline.");
    fireEvent.click(start);
    expect(onStartMachine).not.toHaveBeenCalled();
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
    fireEvent.click(screen.getByRole("button", { name: /^Interactive agent/ }));
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
  it("says Open and Close act on every machine", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderBar({
      multiMachine: true,
      showBootButton: true,
      onBootCli: vi.fn(),
      showCloseProjectButton: true,
      onCloseProject: vi.fn(),
    });
    fireEvent.click(screen.getByTitle("Open this project"));
    expect(String(confirm.mock.calls[0][0])).toContain("each online machine that has its tabs");
    fireEvent.click(screen.getByTitle("Close this project and shut down its agents"));
    expect(String(confirm.mock.calls[1][0])).toContain("on every machine running it");
  });
});
