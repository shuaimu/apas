import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage, type PaneAutopilotState } from "@apas/protocol";
import {
  handleServerMessage,
  paneKey,
  useStore,
  type MachineWithProjects,
  type PaneConfig,
} from "@/lib/store";
import { PROJECT_INSTANCES_CAPABILITY, type SessionInstance } from "@/lib/projectInstances";
import { paneAutopilotKey, TAB_AUTOPILOT_CAPABILITY, type PaneAutopilotEntry } from "@/lib/autopilot";
import { DEFAULT_AUTOPILOT_TEMPLATE } from "@/lib/autopilotTemplates";
import { TabbedView } from "./TabbedView";

/**
 * Autopilot in the project view: the toolbar control and where it stands, the
 * tab marker, the second status line and the confirmations, driven through
 * the real store. Every frame the view sends is checked against the protocol
 * schema the server parses.
 */

// xterm.js needs canvas APIs jsdom lacks; these tests are about the view.
vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => <div data-testid={`terminal-pane-${paneId}`} />,
}));

const SID = "20000000-0000-4000-8000-000000000001";
const CLI = "cli-autopilot";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const AUTOPILOT = "60000000-0000-4000-8000-000000000001";
const initialStore = useStore.getInitialState();

function tab(pane_id: number, label: string, overrides: Partial<PaneConfig> = {}): PaneConfig {
  return {
    pane_id,
    provider: "omp",
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

function autopilot(kind: string, detail?: string): PaneAutopilotState {
  return {
    autopilot: {
      autopilot_id: AUTOPILOT,
      spec: {
        prompt: "Check the build and fix what fails.",
        repeat: { kind: "after_each_turn" },
        iteration_limit: { kind: "at_most", value: 20 },
        time_limit: { kind: "none" },
        end_without_tool_call: true,
      },
      started_at: "2026-11-13T14:00:00Z",
      iterations: 3,
      condition: { kind, detail },
    },
  };
}

const zoo6: MachineWithProjects = {
  machine: { machineId: ZOO6, hostname: "zoo-006", os: "linux", arch: "x86_64" },
  projects: [{ projectId: SID, path: "/work/project", isRunning: true }],
  clusterAccess: "owner",
  daemonConnected: true,
  projectInstancesAvailable: true,
};

let ws: { readyState: number; send: Mock };

function seed(
  panes: PaneConfig[],
  activePaneId: number,
  options: {
    negotiated?: boolean;
    entries?: Record<number, PaneAutopilotEntry>;
    instances?: SessionInstance[];
    paneStatuses?: Record<string, string | null>;
  } = {},
) {
  localStorage.setItem(`apas_layout_${CLI}_active_tab`, String(activePaneId));
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  const capabilities = [PROJECT_INSTANCES_CAPABILITY];
  if (options.negotiated ?? true) capabilities.push(TAB_AUTOPILOT_CAPABILITY);
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: true,
      isDualPane: true,
      sessionId: SID,
      cliClientId: CLI,
      negotiatedCapabilities: new Set(capabilities),
      sessions: [{
        id: SID,
        projectId: SID,
        status: "active",
        isActive: true,
        hostname: "zoo-006",
        instances: options.instances ?? [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess: true }],
      }],
      sessionInstances: {},
      machines: [zoo6],
      messages: [],
      paneConfigs: panes,
      paneListReceived: true,
      paneMessages: Object.fromEntries(panes.map((pane) => [paneKey(pane.pane_id), []])),
      paneHasMore: {},
      paneStatuses: options.paneStatuses ?? {},
      paneModes: {},
      pausedPanes: [],
      loadingMorePane: null,
      hasMoreMessages: false,
      isLoadingMore: false,
      teamRecords: [],
      addPaneRequests: {},
      paneAutopilot: Object.fromEntries(Object.entries(options.entries ?? {})
        .map(([paneId, entry]) => [paneAutopilotKey(SID, Number(paneId)), entry])),
      autopilotRequests: {},
      usageLimits: new Map([[CLI, { omp: { sevenDay: { utilization: 0.1 } } }]]),
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      listMachines: vi.fn(),
    });
  });
}

function sentActions(): Record<string, unknown>[] {
  return ws.send.mock.calls
    .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
    .filter((frame) => frame.type === "control_pane_autopilot")
    .map((frame) => {
      expect(validateClientMessage(frame).errors).toEqual([]);
      return frame.action as Record<string, unknown>;
    });
}

function deliver(message: Record<string, unknown>) {
  act(() => handleServerMessage(message, useStore.setState, useStore.getState));
}

function controlButton(): HTMLButtonElement {
  return within(screen.getByTestId("autopilot-control")).getByRole("button", { name: "Autopilot" }) as HTMLButtonElement;
}

function tabButton(paneId: number): HTMLElement {
  const element = document.querySelector(`[data-tab-id="${paneId}"]`);
  if (!element) throw new Error(`no tab ${paneId}`);
  return element as HTMLElement;
}

beforeEach(() => {
  localStorage.clear();
  // jsdom has no layout engine; TabBar scrolls the active tab into view.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  act(() => {
    useStore.setState(initialStore, true);
  });
  localStorage.clear();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("where the control is offered", () => {
  it("stands before the view switch, with nothing between that switch and the usage status", async () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");

    const control = screen.getByTestId("autopilot-control");
    const viewSwitch = screen.getByRole("group", { name: "Terminal pane view" });
    const usage = screen.getByText(/Usage$/);
    expect(control.compareDocumentPosition(viewSwitch) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(viewSwitch.compareDocumentPosition(usage) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    // The switch's own wrapper is followed directly by the usage status.
    expect(viewSwitch.parentElement?.nextElementSibling?.contains(usage)).toBe(true);
    expect(viewSwitch.parentElement?.previousElementSibling).toBe(control);
  });

  it("shows no control at all on a server without autopilot", async () => {
    seed([tab(7, "OMP 1")], 7, { negotiated: false });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.queryByTestId("autopilot-control")).toBeNull();
    expect(screen.getByRole("group", { name: "Terminal pane view" })).toBeTruthy();
  });

  it("shows no control for a terminal tab of another provider or a legacy structured pane", async () => {
    seed([
      tab(7, "Claude TTY", { provider: "claude" }),
      tab(8, "Legacy", { provider: "claude", kind: undefined }),
    ], 7);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.queryByTestId("autopilot-control")).toBeNull();

    fireEvent.click(tabButton(8));
    expect(await screen.findByPlaceholderText("Type a message...")).toBeTruthy();
    expect(screen.queryByTestId("autopilot-control")).toBeNull();
  });

  it("shows no control for read-only legacy bot history", async () => {
    seed([tab(7, "Old bot", { mode: "deadloop" })], 7);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.queryByTestId("autopilot-control")).toBeNull();
  });

  it("says a just-launched tab has not reported, and does not ask for a reboot", async () => {
    // Nothing is held for the tab.
    seed([tab(7, "OMP 1")], 7);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Not reported");
    fireEvent.click(controlButton());
    const menu = screen.getByRole("group", { name: "Autopilot" });
    expect(menu.textContent).toContain("This tab has not reported its autopilot state.");
    expect(menu.textContent).not.toContain("Reboot");
    // Stop and Stop now only: no start, no Pause, no Resume.
    const offered = within(menu).getAllByRole("button").map((button) => button.textContent ?? "");
    expect(offered).toHaveLength(2);
    expect(offered[0]).toMatch(/^Stop(?! now)/);
    expect(offered[1]).toMatch(/^Stop now/);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("the start dialog", () => {
  it("names the tab's machine and says whether the tab works in the shared checkout", async () => {
    seed([
      tab(7, "OMP 1"),
      tab(8, "OMP 2", { worktree_path: "/work/.apas-worktrees/pane-8" }),
    ], 7, { entries: { 7: { support: "supported" }, 8: { support: "supported" } } });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");

    fireEvent.click(controlButton());
    expect(screen.getByRole("dialog", { name: "Start autopilot — OMP 1" })).toBeTruthy();
    let statement = screen.getByRole("list", { name: "What autopilot is and is not" }).textContent ?? "";
    expect(statement).toContain("This tab keeps running on zoo-006");
    expect(statement).toContain("It can change, commit and push from zoo-006");
    expect(statement).toContain("This tab works in the project's shared checkout.");
    // Opening the dialog sent nothing.
    expect(sentActions()).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    fireEvent.click(tabButton(8));
    await screen.findByTestId("terminal-pane-8");
    fireEvent.click(controlButton());
    statement = screen.getByRole("list", { name: "What autopilot is and is not" }).textContent ?? "";
    expect(statement).toContain("This tab works in its own worktree, not in the project's shared checkout.");
  });
});

describe("one control per tab", () => {
  it("starts closed on the tab selected next: a half-typed start does not carry over", async () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2")], 7, {
      entries: { 7: { support: "supported" }, 8: { support: "supported", state: autopilot("running") } },
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    fireEvent.click(controlButton());
    fireEvent.change(screen.getByLabelText("Prompt"), { target: { value: "Only for OMP 1." } });

    // The other tab is on autopilot: its control is a closed menu, and
    // nothing of the first tab's dialog is left on screen.
    fireEvent.click(tabButton(8));
    await screen.findByTestId("terminal-pane-8");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Running an iteration");
    fireEvent.click(controlButton());
    expect(screen.getByRole("group", { name: "Autopilot" })).toBeTruthy();

    // Back on the first tab, its menu is not open and the start is asked for afresh.
    fireEvent.click(tabButton(7));
    await screen.findByTestId("terminal-pane-7");
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(controlButton());
    expect(screen.getByRole("dialog", { name: "Start autopilot — OMP 1" })).toBeTruthy();
    expect((screen.getByLabelText("Prompt") as HTMLTextAreaElement).value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    expect(sentActions()).toEqual([]);
  });
});

describe("Interrupt", () => {
  it("is not offered on a working terminal tab", async () => {
    seed([tab(7, "OMP 1")], 7, { paneStatuses: { [paneKey(7)]: "Working" } });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.queryByRole("button", { name: "Interrupt" })).toBeNull();
  });

  it("is not offered on a working terminal tab of another provider either", async () => {
    seed([tab(7, "Claude TTY", { provider: "claude" })], 7, { paneStatuses: { [paneKey(7)]: "Working" } });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.queryByRole("button", { name: "Interrupt" })).toBeNull();
  });

  it("is still offered on a working legacy structured pane", async () => {
    seed([tab(8, "Legacy", { provider: "claude", kind: undefined })], 8, {
      paneStatuses: { [paneKey(8)]: "Working" },
    });
    const interruptPane = vi.fn();
    act(() => useStore.setState({ interruptPane }));
    vi.stubGlobal("confirm", vi.fn(() => true));
    render(<TabbedView />);
    fireEvent.click(await screen.findByRole("button", { name: "Interrupt" }));
    expect(interruptPane).toHaveBeenCalledWith(8);
  });
});

describe("using the control", () => {
  it("keeps the same terminal instance and the selected view through a start and a stop", async () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    render(<TabbedView />);
    const terminal = await screen.findByTestId("terminal-pane-7");
    const viewSwitch = screen.getByRole("group", { name: "Terminal pane view" });
    const pressed = (name: string) =>
      within(viewSwitch).getByRole("button", { name }).getAttribute("aria-pressed");
    expect(pressed("Terminal")).toBe("true");

    fireEvent.click(controlButton());
    fireEvent.change(screen.getByLabelText("Prompt"), { target: { value: "Fix the next failing test." } });
    fireEvent.click(screen.getByRole("button", { name: "Start autopilot" }));
    expect(sentActions()).toHaveLength(1);
    expect(sentActions()[0].kind).toBe("start");

    // The tab reports the autopilot; only then does the control show it.
    deliver({
      type: "pane_autopilot_state",
      session_id: SID,
      pane_id: 7,
      support: "supported",
      state: autopilot("running"),
    });
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Running an iteration");

    fireEvent.click(controlButton());
    fireEvent.click(within(screen.getByRole("group", { name: "Autopilot" })).getByRole("button", { name: /^Stop(?! now)/ }));
    expect(sentActions()[1]).toEqual({ kind: "stop", autopilot_id: AUTOPILOT, now: false });

    expect(screen.getByTestId("terminal-pane-7")).toBe(terminal);
    expect(terminal.closest(".hidden")).toBeNull();
    expect(pressed("Terminal")).toBe("true");
    expect(pressed("Conversation")).toBe("false");
  });

  it("leaves the Conversation view selected too", async () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    render(<TabbedView />);
    const terminal = await screen.findByTestId("terminal-pane-7");
    const viewSwitch = screen.getByRole("group", { name: "Terminal pane view" });
    fireEvent.click(within(viewSwitch).getByRole("button", { name: "Conversation" }));
    await screen.findByPlaceholderText("Message the agent…");

    fireEvent.click(controlButton());
    fireEvent.click(within(screen.getByRole("group", { name: "Autopilot" })).getByRole("button", { name: /^Pause/ }));
    expect(sentActions()).toEqual([{ kind: "pause", autopilot_id: AUTOPILOT }]);

    expect(within(viewSwitch).getByRole("button", { name: "Conversation" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByPlaceholderText("Message the agent…")).toBeTruthy();
    expect(screen.getByTestId("terminal-pane-7")).toBe(terminal);
  });

  it("keeps a request that was not confirmed on screen and does not send it again", async () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    fireEvent.click(controlButton());
    fireEvent.click(within(screen.getByRole("group", { name: "Autopilot" })).getByRole("button", { name: /^Stop(?! now)/ }));
    const [requestId] = Object.keys(useStore.getState().autopilotRequests);

    deliver({ type: "pane_autopilot_receipt", session_id: SID, pane_id: 7, request_id: requestId, outcome: "uncertain" });

    const requests = screen.getByRole("status", { name: "Autopilot requests" });
    expect(requests.textContent).toContain("Stop autopilot on OMP 1");
    expect(requests.textContent).toContain("Not confirmed.");
    expect(requests.textContent).toContain("It was not sent again.");
    expect(sentActions()).toHaveLength(1);
    // The state shown is still the tab's own report.
    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Running an iteration");

    fireEvent.click(within(requests).getByRole("button", { name: "Dismiss Stop autopilot on OMP 1" }));
    expect(screen.queryByRole("status", { name: "Autopilot requests" })).toBeNull();
    expect(sentActions()).toHaveLength(1);
  });
});

describe("who may use the tab", () => {
  it("lets a collaborator with runtime access on the tab's machine start, pause, resume and stop", async () => {
    // Shared with this viewer, hosted by another account; the server says
    // they may use that machine.
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    act(() => useStore.setState((state) => ({
      sessions: state.sessions.map((session) => ({ ...session, isShared: true, shareRole: "user" as const })),
    })));
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    const act_ = (name: RegExp) => {
      fireEvent.click(controlButton());
      const action = within(screen.getByRole("group", { name: "Autopilot" })).getByRole("button", { name }) as HTMLButtonElement;
      expect(action.disabled).toBe(false);
      fireEvent.click(action);
    };
    const report = (state: PaneAutopilotState) =>
      deliver({ type: "pane_autopilot_state", session_id: SID, pane_id: 7, support: "supported", state });

    expect(controlButton().disabled).toBe(false);
    fireEvent.click(controlButton());
    fireEvent.change(screen.getByLabelText("Prompt"), { target: { value: "go" } });
    fireEvent.click(screen.getByRole("button", { name: "Start autopilot" }));
    report(autopilot("running"));
    act_(/^Pause/);
    report(autopilot("paused", "requested"));
    act_(/^Resume/);
    report(autopilot("running"));
    act_(/^Stop(?! now)/);

    expect(sentActions().map((action) => action.kind)).toEqual(["start", "pause", "resume", "stop"]);
  });

  it("shows a viewer without runtime access the state, with every action disabled and the tab's reason", async () => {
    const reason = "Access denied: you cannot use zoo-006, which runs this project instance";
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("paused", "interrupted") } },
      instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess: false }],
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");

    expect(screen.getByTestId("autopilot-condition").textContent).toBe("Paused: the turn was interrupted");
    fireEvent.click(controlButton());
    const menu = screen.getByRole("group", { name: "Autopilot" });
    for (const name of [/^Resume/, /^Stop(?! now)/, /^Stop now/]) {
      const action = within(menu).getByRole("button", { name }) as HTMLButtonElement;
      expect(action.disabled).toBe(true);
      expect(action.title).toBe(reason);
      fireEvent.click(action);
    }
    expect(sentActions()).toEqual([]);
    fireEvent.click(within(menu).getByRole("button", { name: "Details" }));
    expect(screen.getByTestId("autopilot-details").textContent).toContain("Check the build and fix what fails.");
  });

  it("puts a stopped machine's reason before the access reason: it applies only while the instance is attached", async () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "not_reported" } },
      // Another machine keeps the project attached; this tab's is not.
      instances: [{ machineId: "50000000-0000-4000-8000-000000000005", hostname: "zoo-005", primary: true, runtimeAccess: true }],
    });
    act(() => useStore.setState({
      machines: [{ ...zoo6, projects: [{ projectId: SID, path: "/work/project", isRunning: false }] }],
    }));
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    fireEvent.click(controlButton());
    const stop = within(screen.getByRole("group", { name: "Autopilot" })).getByRole("button", { name: /^Stop(?! now)/ }) as HTMLButtonElement;
    expect(stop.disabled).toBe(true);
    expect(stop.title).toBe("zoo-006 is not running this project. Start it on zoo-006 first.");
  });
});

describe("the tab marker", () => {
  it("marks a tab on autopilot with a spelled-out name, and leaves other tabs' names unchanged", async () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2"), tab(9, "OMP 3"), tab(10, "OMP 4")], 8, {
      entries: {
        7: { support: "supported", state: autopilot("waiting", "settle") },
        9: { support: "supported", state: autopilot("paused", "failures") },
        10: {
          support: "supported",
          state: {
            last_ended: {
              ...autopilot("running").autopilot!,
              reason: "iteration_limit",
              ended_at: "2026-11-13T15:00:00Z",
            },
          },
        },
      },
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-8");

    expect(tabButton(7).getAttribute("aria-label")).toBe("OMP 1, on autopilot");
    expect(within(tabButton(7)).getByTestId("tab-autopilot-marker").getAttribute("data-autopilot")).toBe("on");
    // Paused keeps the marker, in a paused style.
    expect(tabButton(9).getAttribute("aria-label")).toBe("OMP 3, autopilot paused");
    expect(within(tabButton(9)).getByTestId("tab-autopilot-marker").getAttribute("data-autopilot")).toBe("paused");
    // Not on autopilot, and ended: named by their content, exactly as before.
    for (const paneId of [8, 10]) {
      expect(tabButton(paneId).hasAttribute("aria-label")).toBe(false);
      expect(within(tabButton(paneId)).queryByTestId("tab-autopilot-marker")).toBeNull();
    }
    // Never the worker's icon or words.
    expect(tabButton(7).textContent).not.toMatch(/bot|worker/i);
  });

  it("names every tab as before on a server without autopilot", async () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2")], 7, {
      negotiated: false,
      entries: { 7: { support: "supported", state: autopilot("running") } },
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    for (const paneId of [7, 8]) {
      expect(tabButton(paneId).hasAttribute("aria-label")).toBe(false);
      expect(within(tabButton(paneId)).queryByTestId("tab-autopilot-marker")).toBeNull();
    }
  });
});

describe("the second status line", () => {
  it("shows the autopilot condition beside the tab's status, not in its place", async () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("waiting", "settle") } },
      paneStatuses: { [paneKey(7)]: "Working" },
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    expect(screen.getByText("Working")).toBeTruthy();
    const line = screen.getByTestId("autopilot-status-line");
    expect(line.textContent).toContain("Autopilot · Waiting for the tab to settle");
    expect(line.textContent).toContain("3 of 20 iterations");
  });

  it("shows OMP's own modes while OMP reports them, with how to stop them in the terminal", async () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: {
            omp_modes: {
              goal: { status: "active", objective: "Ship the parser", token_budget: 200000 },
              loop: { status: "paused" },
            },
          },
        },
      },
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    const goal = screen.getByTestId("omp-mode-goal").textContent ?? "";
    expect(goal).toContain("This tab is in OMP's own goal mode and continues by itself.");
    expect(goal).toContain("Objective: Ship the parser");
    expect(goal).toContain("Token budget: 200,000");
    expect(goal).toContain("Esc pauses it, and /goal drop removes the goal.");
    const loop = screen.getByTestId("omp-mode-loop").textContent ?? "";
    expect(loop).toContain("This tab is in OMP's own loop mode, paused.");
    expect(loop).toContain("Esc pauses it, and /loop switches the loop off.");
    expect(loop).toContain("A message sent to this tab now becomes the prompt the loop repeats.");
    expect(screen.queryByTestId("autopilot-status-line")).toBeNull();

    // Once OMP no longer reports a mode, nothing about it is shown: /loop is
    // a toggle, and typing it then would switch a new loop on.
    deliver({ type: "pane_autopilot_state", session_id: SID, pane_id: 7, support: "supported", state: {} });
    expect(screen.queryByTestId("omp-mode-goal")).toBeNull();
    expect(screen.queryByTestId("omp-mode-loop")).toBeNull();
  });
});

describe("confirmations", () => {
  it("says that closing a tab on autopilot ends its autopilot with the tab's agent", async () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2")], 7, {
      entries: { 7: { support: "supported", state: autopilot("running") } },
    });
    const removePane = vi.fn();
    act(() => useStore.setState({ removePane }));
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");

    fireEvent.click(within(tabButton(7)).getByTitle("Close tab"));
    expect(confirm).toHaveBeenLastCalledWith(
      "Close this tab?\n\nThis tab is on autopilot. Closing the tab ends its autopilot together with the tab's agent.",
    );
    expect(removePane).not.toHaveBeenCalled();

    // A tab that is not on autopilot is asked what it always was.
    fireEvent.click(within(tabButton(8)).getByTitle("Close tab"));
    expect(confirm).toHaveBeenLastCalledWith("Close this tab?");
  });

  it("says it in the worktree dialog too", async () => {
    seed([tab(7, "OMP 1", { worktree_path: "/work/.apas-worktrees/pane-7" })], 7, {
      entries: { 7: { support: "supported", state: autopilot("paused", "requested") } },
    });
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    fireEvent.click(within(tabButton(7)).getByTitle("Close tab"));
    expect(await screen.findByText("Close pane with isolated worktree")).toBeTruthy();
    expect(screen.getByRole("note").textContent)
      .toBe("This tab has a paused autopilot. Closing the tab discards it.");
  });

  it("says that rebooting the agent brings autopilot back paused", async () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2")], 7, {
      entries: { 7: { support: "supported", state: autopilot("waiting", "next_iteration") } },
    });
    const rebootPane = vi.fn();
    act(() => useStore.setState({ rebootPane }));
    const confirm = vi.fn((_message?: string) => true);
    vi.stubGlobal("confirm", confirm);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");

    fireEvent.click(screen.getByRole("button", { name: "Reboot" }));
    const message = confirm.mock.calls.at(-1)?.[0] ?? "";
    expect(message).toMatch(/^Reboot this pane's agent\?/);
    expect(message).toContain(
      "This tab is on autopilot. After the reboot its autopilot comes back paused, and stays paused until someone resumes it.",
    );
    expect(rebootPane).toHaveBeenCalledWith(7);

    fireEvent.click(tabButton(8));
    await screen.findByTestId("terminal-pane-8");
    fireEvent.click(screen.getByRole("button", { name: "Reboot" }));
    expect(confirm.mock.calls.at(-1)?.[0]).not.toContain("autopilot");
  });

  it("names the tabs on autopilot when the project is closed", async () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2"), tab(9, "OMP 3")], 8, {
      entries: {
        7: { support: "supported", state: autopilot("running") },
        // Paused already: closing the project changes nothing for it.
        9: { support: "supported", state: autopilot("paused", "requested") },
      },
    });
    const stopMachineProjectCli = vi.fn();
    act(() => useStore.setState({ stopMachineProjectCli }));
    const confirm = vi.fn((_message?: string) => true);
    vi.stubGlobal("confirm", confirm);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-8");

    fireEvent.click(screen.getByTitle("Close this project and shut down its agents"));
    const message = confirm.mock.calls.at(-1)?.[0] ?? "";
    expect(message).toMatch(/^Close this project\?/);
    expect(message).toContain(
      "OMP 1 is on autopilot. Its autopilot comes back paused when the project is started again.",
    );
    expect(message).not.toContain("OMP 3");
    await waitFor(() => expect(stopMachineProjectCli).toHaveBeenCalled());
  });

  it("asks what it always asked when no tab is on autopilot", async () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    act(() => useStore.setState({ stopMachineProjectCli: vi.fn() }));
    const confirm = vi.fn((_message?: string) => false);
    vi.stubGlobal("confirm", confirm);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-7");
    fireEvent.click(screen.getByTitle("Close this project and shut down its agents"));
    expect(confirm).toHaveBeenLastCalledWith(
      "Close this project? Every agent in it is shut down, and it stops appearing in the idle list until you open it again.",
    );
  });
});
