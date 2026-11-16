import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { paneKey, useStore, type MachineWithProjects, type PaneConfig } from "@/lib/store";
import { PROJECT_INSTANCES_CAPABILITY } from "@/lib/projectInstances";
import { TAB_AUTOPILOT_CAPABILITY } from "@/lib/autopilot";
import { TabbedView } from "./TabbedView";

/**
 * Where the pane toolbar offers the loop prompt: in the autopilot control's
 * place on Codex, Claude (and DeepSeek) and OpenCode terminal tabs, never on
 * OMP, structured, legacy or retired tabs, and disabled with the reason when
 * the tab cannot take a message.
 */

vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => <div data-testid={`terminal-pane-${paneId}`} />,
}));

const SID = "20000000-0000-4000-8000-000000000001";
const CLI = "cli-loop-prompt";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const initialStore = useStore.getInitialState();

function tab(pane_id: number, label: string, overrides: Partial<PaneConfig> = {}): PaneConfig {
  return {
    pane_id,
    provider: "claude",
    mode: "interactive",
    kind: "terminal",
    session_id: `${SID}-pane-${pane_id}`,
    is_paused: false,
    label,
    machine_id: ZOO6,
    hostname: "zoo-006",
    ...overrides,
  };
}

const zoo6: MachineWithProjects = {
  machine: { machineId: ZOO6, hostname: "zoo-006", os: "linux", arch: "x86_64" },
  projects: [{ projectId: SID, path: "/work/project", isRunning: true }],
  clusterAccess: "owner",
  daemonConnected: true,
  projectInstancesAvailable: true,
};

/** A second machine of the owner's that has the project but is not running it. */
const zoo5: MachineWithProjects = {
  machine: { machineId: ZOO5, hostname: "zoo-005", os: "linux", arch: "x86_64" },
  projects: [{ projectId: SID, path: "/work/project", isRunning: false }],
  clusterAccess: "owner",
  daemonConnected: true,
  projectInstancesAvailable: true,
};

let ws: { readyState: number; send: Mock };

function seed(
  panes: PaneConfig[],
  activePaneId: number,
  { attached = true, connected = true, runtimeAccess = true } = {},
) {
  localStorage.setItem(`apas_layout_${CLI}_active_tab`, String(activePaneId));
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected,
      isAttached: attached,
      isDualPane: true,
      sessionId: SID,
      cliClientId: CLI,
      negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY, TAB_AUTOPILOT_CAPABILITY]),
      sessions: [{
        id: SID,
        projectId: SID,
        status: "active",
        isActive: true,
        hostname: "zoo-006",
        instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess }],
      }],
      sessionInstances: {},
      machines: [zoo6, zoo5],
      messages: [],
      paneConfigs: panes,
      paneListReceived: true,
      paneMessages: Object.fromEntries(panes.map((pane) => [paneKey(pane.pane_id), []])),
      paneHasMore: {},
      paneStatuses: {},
      paneModes: {},
      pausedPanes: [],
      loadingMorePane: null,
      hasMoreMessages: false,
      isLoadingMore: false,
      teamRecords: [],
      addPaneRequests: {},
      paneAutopilot: {},
      autopilotRequests: {},
      pendingTerminalDeliveries: [],
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      listMachines: vi.fn(),
    });
  });
}

function loopButton(): HTMLButtonElement {
  return within(screen.getByTestId("loop-prompt-control")).getByRole("button", { name: "Loop prompt" }) as HTMLButtonElement;
}

function tabButton(paneId: number): HTMLElement {
  const element = document.querySelector(`[data-tab-id="${paneId}"]`);
  if (!element) throw new Error(`no tab ${paneId}`);
  return element as HTMLElement;
}

beforeEach(() => {
  localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  act(() => {
    useStore.setState(initialStore, true);
  });
  localStorage.clear();
  vi.clearAllMocks();
});

describe("where the loop prompt is offered", () => {
  it("stands where an OMP tab has Autopilot, before the view switch, on Claude, Codex, OpenCode and DeepSeek tabs", async () => {
    seed([
      tab(3, "Claude 1"),
      tab(4, "Codex 1", { provider: "codex" }),
      tab(5, "OpenCode 1", { provider: "opencode" }),
      tab(6, "DeepSeek 1", { provider: "deepseek" }),
    ], 3);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-3");
    for (const paneId of [3, 4, 5, 6]) {
      fireEvent.click(tabButton(paneId));
      const control = screen.getByTestId("loop-prompt-control");
      expect(loopButton().disabled).toBe(false);
      expect(screen.queryByTestId("autopilot-control")).toBeNull();
      const viewSwitch = screen.getByRole("group", { name: "Terminal pane view" });
      expect(viewSwitch.parentElement?.previousElementSibling).toBe(control);
    }
  });

  it("is not offered on an OMP tab, which has Autopilot", async () => {
    seed([tab(7, "OMP 1", { provider: "omp" })], 7);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.getByTestId("autopilot-control")).toBeTruthy();
    expect(screen.queryByTestId("loop-prompt-control")).toBeNull();
  });

  it("is not offered on a structured pane, legacy bot history or a retired provider's tab", async () => {
    seed([
      tab(3, "Legacy", { kind: undefined }),
      tab(4, "Old bot", { mode: "deadloop" }),
      tab(5, "Saved Pi", { provider: "pi" }),
    ], 3);
    render(<TabbedView />);
    expect(await screen.findByPlaceholderText("Type a message...")).toBeTruthy();
    expect(screen.queryByTestId("loop-prompt-control")).toBeNull();
    fireEvent.click(tabButton(4));
    await screen.findByTestId("terminal-pane-4");
    expect(screen.queryByTestId("loop-prompt-control")).toBeNull();
    fireEvent.click(tabButton(5));
    expect(screen.queryByTestId("loop-prompt-control")).toBeNull();
  });

  it("is disabled with the reason while the project is not running", async () => {
    seed([tab(3, "Claude 1")], 3, { attached: false });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-3");
    expect(loopButton().disabled).toBe(true);
    expect(loopButton().title).toBe("This project is not running. Open it first.");
    expect(within(screen.getByTestId("loop-prompt-control")).getByRole("note").textContent)
      .toBe("This project is not running. Open it first.");
  });

  it("is disabled with the reason while the page is not connected", async () => {
    seed([tab(3, "Claude 1")], 3, { connected: false });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-3");
    expect(loopButton().disabled).toBe(true);
    expect(loopButton().title).toBe("Reconnect to send a loop prompt.");
  });

  it("is disabled with the server's reason for a viewer who may not use the tab's machine, and sends nothing", async () => {
    seed([tab(3, "Claude 1")], 3, { runtimeAccess: false });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-3");
    const reason = "Access denied: you cannot use zoo-006, which runs this project instance";
    expect(loopButton().disabled).toBe(true);
    expect(loopButton().title).toBe(reason);
    expect(within(screen.getByTestId("loop-prompt-control")).getByRole("note").textContent).toBe(reason);
    fireEvent.click(loopButton());
    expect(screen.queryByRole("dialog", { name: /Loop prompt/ })).toBeNull();
    expect(ws.send.mock.calls.map(([raw]) => JSON.parse(String(raw)).type)).not.toContain("terminal_conversation_input");
  });

  it("is disabled with the reason on a tab whose machine is not running the project, and sends nothing", async () => {
    seed([
      tab(3, "Claude 1"),
      tab(4, "Codex on zoo-005", { provider: "codex", machine_id: ZOO5, hostname: "zoo-005" }),
    ], 4);
    render(<TabbedView />);
    await screen.findByTestId("loop-prompt-control");
    const reason = "zoo-005 is not running this project. Open the project again to start it there.";
    expect(loopButton().disabled).toBe(true);
    expect(loopButton().title).toBe(reason);
    expect(within(screen.getByTestId("loop-prompt-control")).getByRole("note").textContent).toBe(reason);
    fireEvent.click(loopButton());
    expect(screen.queryByRole("dialog", { name: /Loop prompt/ })).toBeNull();
    expect(ws.send.mock.calls.map(([raw]) => JSON.parse(String(raw)).type)).not.toContain("terminal_conversation_input");
    // The tab on the running machine stays usable.
    fireEvent.click(tabButton(3));
    expect(loopButton().disabled).toBe(false);
  });

  it("opens the dialog without sending anything, keeping the terminal and the selected view", async () => {
    seed([tab(3, "Claude 1")], 3);
    render(<TabbedView />);
    const terminal = await screen.findByTestId("terminal-pane-3");
    fireEvent.click(loopButton());
    const dialog = screen.getByRole("dialog", { name: "Loop prompt for Claude 1" });
    expect(within(dialog).getByTestId("loop-prompt-warning").textContent).toContain("on zoo-006");
    expect(ws.send.mock.calls.map(([raw]) => JSON.parse(String(raw)).type)).not.toContain("terminal_conversation_input");
    expect(screen.getByTestId("terminal-pane-3")).toBe(terminal);
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByTestId("terminal-pane-3")).toBe(terminal);
  });
});
