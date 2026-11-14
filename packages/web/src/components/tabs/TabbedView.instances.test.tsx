import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage } from "@apas/protocol";
import {
  handleServerMessage,
  paneKey,
  useStore,
  type CliLifecycleInventory,
  type MachineWithProjects,
  type PaneConfig,
} from "@/lib/store";
import { PROJECT_INSTANCES_CAPABILITY, type SessionInstance } from "@/lib/projectInstances";
import { TabbedView } from "./TabbedView";

/**
 * The project view of a project whose tabs run on two machines, driven
 * through the real store actions. Every frame the view sends is checked
 * against the protocol schema the server parses.
 */

// xterm.js needs canvas APIs jsdom lacks; these tests are about the view.
// The stub shows what the view tells the terminal about taking input.
vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ paneId, inputDisabledReason }: { paneId: number; inputDisabledReason?: string }) => (
    <div data-testid={`terminal-pane-${paneId}`} data-input-disabled={inputDisabledReason} />
  ),
}));

const SID = "20000000-0000-4000-8000-000000000001";
const CLI = "cli-instances";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const LAPTOP = "50000000-0000-4000-8000-000000000007";
const STOPPED = "zoo-006 is not running this project. Start it on zoo-006 first.";
const initialStore = useStore.getInitialState();

function terminal(pane_id: number, label: string, machine_id: string, hostname: string): PaneConfig {
  return {
    pane_id,
    provider: "claude",
    mode: "interactive",
    kind: "terminal",
    session_id: `${SID}-pane-${pane_id}`,
    is_paused: false,
    label,
    machine_id,
    hostname,
  };
}

function machine(machineId: string, hostname: string, registered: boolean): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: registered ? [{ projectId: SID, path: "/work/project", isRunning: machineId === ZOO5 }] : [],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
  };
}

function instance(machineId: string, hostname: string, primary = false): SessionInstance {
  return { machineId, hostname, primary, runtimeAccess: true };
}

let ws: { readyState: number; send: Mock };

function seed(activePaneId: number, instances: SessionInstance[] = [instance(ZOO5, "zoo-005", true)]) {
  const panes = [terminal(10, "Claude 1", ZOO5, "zoo-005"), terminal(11, "Claude 2", ZOO6, "zoo-006")];
  localStorage.setItem(`apas_layout_${CLI}_active_tab`, String(activePaneId));
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: true,
      isDualPane: true,
      sessionId: SID,
      cliClientId: CLI,
      negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY, "cli_lifecycle_v1"]),
      sessions: [{
        id: SID,
        projectId: SID,
        status: "active",
        isActive: true,
        hostname: "zoo-005",
        gitRemote: "github.com/example/project",
        gitRemoteUrl: "https://github.com/example/project.git",
        instances,
      }],
      sessionInstances: {},
      machines: [
        machine(ZOO5, "zoo-005", true),
        machine(ZOO6, "zoo-006", true),
        machine(LAPTOP, "laptop", false),
      ],
      projectPolicies: {
        [SID]: {
          teamAvailable: false,
          allowedLaunchProfiles: ["terminal:claude:official:default"],
          version: 3,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
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
      cliLifecycleInventories: {},
      cliLifecycleOperations: {},
      cliLifecycleLatestBySession: {},
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      listMachines: vi.fn(),
      showToast: vi.fn(),
    });
  });
}

function sent(type: string): Record<string, unknown>[] {
  return ws.send.mock.calls
    .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
    .filter((frame) => frame.type === type);
}

function expectValid(frame: Record<string, unknown>) {
  const result = validateClientMessage(frame);
  expect(result.errors).toEqual([]);
  expect(result.valid).toBe(true);
}

function deliver(message: Record<string, unknown>) {
  act(() => handleServerMessage(message, useStore.setState, useStore.getState));
}

function openMachineStep() {
  fireEvent.click(screen.getByTitle("New tab"));
  return screen.getByRole("group", { name: "Run on" });
}

function launchList() {
  return screen.getByRole("status", { name: "New tab launches" });
}

beforeEach(() => {
  localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  act(() => useStore.setState(initialStore, true));
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("a tab whose machine is not running the project", () => {
  it("explains it, starts only that machine, and forgets its tabs after confirmation", async () => {
    seed(11);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-11");

    const banner = screen.getAllByRole("status").find((element) => element.textContent?.includes(STOPPED));
    expect(banner).toBeTruthy();
    // The pane's controls are disabled with the reason.
    const reboot = screen.getByRole("button", { name: "Reboot" }) as HTMLButtonElement;
    expect(reboot.disabled).toBe(true);
    expect(reboot.title).toBe(STOPPED);

    fireEvent.click(within(banner as HTMLElement).getByRole("button", { name: "Start on zoo-006" }));
    const [start] = sent("start_machine_project_cli");
    // That machine only: the project view's Open is what starts them all.
    expect(start).toEqual({ type: "start_machine_project_cli", machine_id: ZOO6, project_id: SID });
    expectValid(start);

    fireEvent.click(within(banner as HTMLElement).getByRole("button", { name: "Forget zoo-006's tabs" }));
    expect(String(confirm.mock.calls[0][0])).toContain("They come back if zoo-006 runs this project again.");
    const [forget] = sent("forget_machine_panes");
    expect(forget).toEqual({ type: "forget_machine_panes", session_id: SID, machine_id: ZOO6 });
    expectValid(forget);
  });

  it("does not forget when the confirmation is declined", async () => {
    seed(11);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-11");
    fireEvent.click(screen.getByRole("button", { name: "Forget zoo-006's tabs" }));
    expect(sent("forget_machine_panes")).toEqual([]);
  });

  it("refuses a close before asking, from any surface", async () => {
    seed(10);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-10");

    // The Overview's "Close tab" reaches the same close path as the tab bar.
    fireEvent.click(screen.getByRole("button", { name: "Project overview" }));
    const panels = await screen.findByRole("region", { name: "Interactive project" });
    const removes = within(panels).getAllByTitle("Close this pane (worktree-cleanup prompt opens if applicable)");
    fireEvent.click(removes[1]);

    expect(confirm).not.toHaveBeenCalled();
    expect(sent("remove_pane")).toEqual([]);
    expect(screen.getByText(STOPPED)).toBeTruthy();
  });

  it("keeps the terminal readable but tells it to send no keys or sizes", async () => {
    // Each key or size it sent would only come back refused, as an error
    // toast; opening the tab alone sends two sizes.
    seed(11);
    const { container } = render(<TabbedView />);
    const stopped = await screen.findByTestId("terminal-pane-11");
    expect(stopped.getAttribute("data-input-disabled")).toBe(STOPPED);

    fireEvent.click(container.querySelector('[data-tab-id="10"]') as Element);
    const running = await screen.findByTestId("terminal-pane-10");
    expect(running.hasAttribute("data-input-disabled")).toBe(false);
  });

  it("keeps the conversation input's draft but will not send it", async () => {
    seed(11);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-11");
    fireEvent.click(screen.getByRole("button", { name: /Conversation/ }));
    const input = await screen.findByPlaceholderText(STOPPED);
    fireEvent.change(input, { target: { value: "hello" } });
    expect((screen.getByRole("button", { name: "Send" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("will not answer the stopped machine's questions or plan reviews, and says why", async () => {
    // The server refuses the answer and keeps the question pending; answering
    // here used to toast success, mark the card answered and queue the
    // answer for replay on every reconnect.
    seed(11);
    const question = {
      id: "question-11",
      role: "assistant" as const,
      content: "",
      timestamp: new Date(),
      sessionId: SID,
      paneId: 11,
      outputType: {
        type: "tool_use" as const,
        tool: "AskUserQuestion",
        toolUseId: "toolu-question-11",
        input: { questions: [{ question: "Ship it?", options: [{ label: "Yes" }, { label: "No" }] }] },
      },
    };
    act(() => useStore.setState({
      paneMessages: { ...useStore.getState().paneMessages, [paneKey(11)]: [question] },
      planReviewPending: [{ paneId: 11, toolUseId: "toolu-plan-11", toolName: "Bash", input: { command: "ls" }, arrivedAt: Date.now() }],
    }));
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-11");
    fireEvent.click(screen.getByRole("button", { name: /Conversation/ }));

    const yes = await screen.findByRole("button", { name: "Yes" }) as HTMLButtonElement;
    expect(yes.disabled).toBe(true);
    fireEvent.click(yes);
    const submit = screen.getByRole("button", { name: "Submit" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    expect(submit.title).toBe(STOPPED);
    fireEvent.click(submit);
    const reasons = screen.getAllByRole("status").filter((element) => element.textContent === STOPPED);
    // The question card and the plan review each say why, beside the banner.
    expect(reasons.length).toBeGreaterThanOrEqual(2);

    const approve = screen.getByRole("button", { name: "Approve" }) as HTMLButtonElement;
    expect(approve.disabled).toBe(true);
    fireEvent.click(approve);

    expect(sent("answer_question")).toEqual([]);
    expect(sent("plan_review_answer")).toEqual([]);
    const state = useStore.getState();
    expect(state.answeredQuestions.has("toolu-question-11")).toBe(false);
    expect(state.pendingAnswers).toEqual([]);
    expect(state.planReviewPending).toHaveLength(1);
    expect(state.showToast).not.toHaveBeenCalledWith("Answer sent to Claude", "success");
  });

  it("offers no Start beside a CLI without multi-machine support, and says which to reboot", async () => {
    // zoo-005's daemon is connected but older: its instance owns the project,
    // so the server would refuse zoo-006's instance beside it.
    seed(11);
    act(() => useStore.setState({
      machines: [
        { ...machine(ZOO5, "zoo-005", true), projectInstancesAvailable: false },
        machine(ZOO6, "zoo-006", true),
        machine(LAPTOP, "laptop", false),
      ],
    }));
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-11");
    const start = screen.getByRole("button", { name: "Start on zoo-006" }) as HTMLButtonElement;
    expect(start.disabled).toBe(true);
    expect(start.title).toBe("Reboot zoo-005 to update");
    fireEvent.click(start);
    expect(sent("start_machine_project_cli")).toEqual([]);
  });

  it("calls a machine whose instance is reconnecting disconnected, with nothing to start", async () => {
    // zoo-006's daemon still runs the project; only its instance's socket is
    // gone, and it comes back by itself.
    seed(11);
    act(() => useStore.setState({
      machines: [
        machine(ZOO5, "zoo-005", true),
        { ...machine(ZOO6, "zoo-006", true), projects: [{ projectId: SID, path: "/work/project", isRunning: true }] },
        machine(LAPTOP, "laptop", false),
      ],
    }));
    const { container } = render(<TabbedView />);
    const pane = await screen.findByTestId("terminal-pane-11");
    const reconnecting = "zoo-006 is reconnecting to this project. Its tabs work again once it reconnects.";
    expect(screen.getAllByRole("status").some((element) => element.textContent?.includes(reconnecting))).toBe(true);
    expect(pane.getAttribute("data-input-disabled")).toBe(reconnecting);
    expect(container.querySelector('[data-tab-id="11"]')?.textContent).toContain("disconnected");
    expect(screen.queryByRole("button", { name: "Start on zoo-006" })).toBeNull();
    // A sleeping laptop's daemon reads as connected and running for hours, so
    // its tabs can still be forgotten from here.
    expect(screen.getByRole("button", { name: "Forget zoo-006's tabs" })).toBeTruthy();
  });
});

describe("choosing a machine for a new tab", () => {
  it("adds the tab on the chosen machine and shows the launch until its result", async () => {
    seed(10, [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")]);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-10");

    const step = openMachineStep();
    // A new tab defaults to the selected tab's machine.
    expect((within(step).getByRole("radio", { name: /^zoo-005/ }) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(within(step).getByRole("radio", { name: /^zoo-006/ }));
    fireEvent.click(screen.getByText("Claude"));

    const [add] = sent("add_pane");
    expect(add).toMatchObject({ type: "add_pane", session_id: SID, machine_id: ZOO6, kind: "terminal", label: "Claude 3" });
    expect(typeof add.request_id).toBe("string");
    expectValid(add);
    expect(launchList().textContent).toContain("Claude 3 on zoo-006 — Requesting the tab…");

    deliver({ type: "add_pane_result", session_id: SID, request_id: add.request_id, status: "failed", machine_id: ZOO6, error: "zoo-006 refused the tab" });
    expect(launchList().textContent).toContain("zoo-006 refused the tab");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss Claude 3 on zoo-006" }));
    expect(screen.queryByRole("status", { name: "New tab launches" })).toBeNull();
    // Nothing was ever sent again.
    expect(sent("add_pane")).toHaveLength(1);
  });

  it("starts a registered machine first, and reports a lost answer as unconfirmed without resending", async () => {
    seed(10);
    // The store's own socket, so dropping it is the real connection loss.
    localStorage.setItem("apas_token", "test-token");
    act(() => useStore.setState({ ws: null }));
    act(() => useStore.getState().connect());
    await act(() => new Promise((done) => setTimeout(done, 10)));
    const socket = useStore.getState().ws as unknown as { readyState: number; send: Mock; onclose?: (event: CloseEvent) => void };
    ws = socket;
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-10");

    fireEvent.click(within(openMachineStep()).getByRole("radio", { name: /^zoo-006/ }));
    fireEvent.click(screen.getByText("Claude"));
    const [add] = sent("add_pane");
    expect(add.machine_id).toBe(ZOO6);

    deliver({ type: "add_pane_result", session_id: SID, request_id: add.request_id, status: "starting", machine_id: ZOO6 });
    expect(launchList().textContent).toContain("Starting the project on zoo-006…");
    // In flight: nothing to dismiss yet.
    expect(screen.queryByRole("button", { name: /^Dismiss/ })).toBeNull();

    // The answer goes only to the connection that asked; once it is gone the
    // launch can only be reported, never resent.
    act(() => socket.onclose?.(new CloseEvent("close", { code: 1000 })));
    await waitFor(() => expect(launchList().querySelector('[data-launch-status="unconfirmed"]')).toBeTruthy());
    expect(launchList().textContent).toContain("It was not resent");
    expect(screen.getByRole("button", { name: "Dismiss Claude 3 on zoo-006" })).toBeTruthy();
    expect(sent("add_pane")).toHaveLength(1);
  });

  it("clones onto an owned machine only after confirmation, then adds the tab there", async () => {
    seed(10);
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValue(true);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-10");

    fireEvent.click(within(openMachineStep()).getByRole("radio", { name: /^laptop/ }));
    fireEvent.click(screen.getByText("Claude"));
    expect(String(confirm.mock.calls[0][0])).toContain("Clone this project onto laptop?");
    expect(sent("create_project_instance")).toEqual([]);

    fireEvent.click(within(openMachineStep()).getByRole("radio", { name: /^laptop/ }));
    fireEvent.click(screen.getByText("Claude"));
    const [clone] = sent("create_project_instance");
    expect(clone).toMatchObject({
      type: "create_project_instance",
      machine_id: LAPTOP,
      git_remote: "github.com/example/project",
      clone_url: "https://github.com/example/project.git",
      instance_name: "project",
      join_project_id: SID,
    });
    expectValid(clone);
    expect(launchList().textContent).toContain("Cloning the repository onto laptop…");

    deliver({ type: "project_instance_created", request_id: clone.request_id, machine_id: LAPTOP, project_id: SID });
    const [add] = sent("add_pane");
    expect(add).toMatchObject({ machine_id: LAPTOP, label: "Claude 3" });
    expectValid(add);
    expect(launchList().textContent).toContain("Requesting the tab…");
  });
});

describe("rebooting one machine's instance", () => {
  it("confirms with that machine's inventory and sends the request to it", async () => {
    seed(10, [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")]);
    const inventory = (mode: "live_adoptable" | "restart_required_on_cli_reboot"): CliLifecycleInventory => ({
      persistent_terminal_hosting: true,
      panes: [{ pane_id: 1, mode }],
    });
    act(() => useStore.setState({
      cliLifecycleInventories: { [SID]: { [ZOO5]: inventory("live_adoptable"), [ZOO6]: inventory("restart_required_on_cli_reboot") } },
    }));
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-10");

    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    fireEvent.click(screen.getByText("Reboot CLI on zoo-006"));
    expect(String(confirm.mock.calls[0][0])).toContain("1 terminal pane will restart and resume where supported.");
    const [request] = sent("cli_lifecycle_request");
    expect(request).toMatchObject({ session_id: SID, operation: "reboot_cli", machine_id: ZOO6 });
    expectValid(request);
  });
});

describe("another machine starting or stopping the project", () => {
  it("leaves the selected tab where it is", async () => {
    // The remembered tab is keyed by the session's CLI connection. Every
    // attached instance reports the session, listed in no fixed order, so
    // following the first one listed would re-key it and jump to tab 10.
    seed(11, [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")]);
    act(() => useStore.setState((state) => ({
      sessions: state.sessions.map((session) => ({ ...session, cliClientId: CLI })),
      cliClients: [{ id: CLI, status: "online", activeSession: SID }],
    })));
    render(<TabbedView />);
    await screen.findByTestId("terminal-pane-11");

    const client = (id: string) => ({ id, name: id, status: "online", last_seen: null, active_session: SID });
    deliver({ type: "cli_clients", clients: [client("cli-zoo-006"), client(CLI)] });
    deliver({ type: "cli_clients", clients: [client(CLI)] });

    expect(useStore.getState().cliClientId).toBe(CLI);
    // Tab 10 was never selected, so its pane was never mounted.
    expect(screen.queryByTestId("terminal-pane-10")).toBeNull();
    expect(screen.getByTestId("terminal-pane-11")).toBeTruthy();
  });
});
