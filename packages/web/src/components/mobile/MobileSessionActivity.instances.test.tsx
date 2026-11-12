import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage } from "@apas/protocol";
import { handleServerMessage, useStore, type MachineWithProjects, type PaneConfig } from "@/lib/store";
import { writeSelectedPane } from "@/lib/mobileSelectedPane";
import { PROJECT_INSTANCES_CAPABILITY, type SessionInstance } from "@/lib/projectInstances";
import { MobileSessionActivity } from "./MobileSessionActivity";

/**
 * The mobile-web session screen of a project whose panes run on two
 * machines, through the real store actions; every frame it sends is checked
 * against the protocol schema.
 */

vi.mock("@/components/tabs/TerminalPane", () => ({
  TerminalPane: ({ paneId, inputDisabledReason }: { paneId: number; inputDisabledReason?: string }) => (
    <div data-input-disabled={inputDisabledReason}>Raw terminal pane {paneId}</div>
  ),
}));

const SID = "20000000-0000-4000-8000-000000000001";
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

function seed(selectedPaneId: number, instances: SessionInstance[] = [instance(ZOO5, "zoo-005", true)]) {
  writeSelectedPane(SID, selectedPaneId);
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: true,
      sessionId: SID,
      negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY]),
      sessions: [{
        id: SID,
        projectId: SID,
        status: "active",
        isActive: true,
        hostname: "zoo-005",
        workingDir: "/work/project",
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
      paneConfigs: [terminal(10, "Claude 1", ZOO5, "zoo-005"), terminal(11, "Claude 2", ZOO6, "zoo-006")],
      paneListReceived: true,
      paneMessages: {},
      messages: [],
      paneStatuses: {},
      paneHasMore: {},
      pendingTerminalDeliveries: [],
      answeredQuestions: new Map(),
      planReviewPending: [],
      addPaneRequests: {},
      loadSessionActivity: vi.fn(),
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

function renderScreen() {
  render(<MobileSessionActivity connected onBack={vi.fn()} onReconnect={vi.fn()} />);
}

function openInteractiveSheet() {
  fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
  const sheet = screen.getByRole("dialog", { name: "Add agent" });
  fireEvent.click(within(sheet).getByRole("button", { name: /^Interactive agent/ }));
  return sheet;
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

describe("where mobile panes run", () => {
  it("shows each pane's host and the project's primary host plus the others", () => {
    seed(10, [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")]);
    renderScreen();
    expect(screen.getByRole("button", { name: "Claude 1 · zoo-005" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Claude 2 · zoo-006" })).toBeTruthy();
    expect(screen.getByText("zoo-005 +1")).toBeTruthy();
  });

  // A phone's chip row is about 200px wide, too narrow for a pane's name,
  // machine and state on one line: a chip naming a machine puts it on a
  // second line and is never wider than the row, or a long name pushes the
  // machine's state out of view.
  it("keeps a chip naming a machine within the row, its machine's state whole", () => {
    seed(11);
    renderScreen();
    const chip = screen.getByRole("button", { name: "Claude 2 · zoo-006 (stopped)" });
    expect(chip.className.split(" ")).toEqual(expect.arrayContaining(["flex-col", "max-w-full"]));
    const [label, machine] = Array.from(chip.children) as HTMLElement[];
    expect(label.textContent).toBe("Claude 2");
    expect(label.className.split(" ")).toEqual(expect.arrayContaining(["max-w-full", "truncate"]));
    expect(machine.textContent).toBe("zoo-006 · stopped");
    expect(machine.className.split(" ")).toContain("max-w-full");
    const [hostname, state] = Array.from(machine.children) as HTMLElement[];
    expect(hostname.className.split(" ")).toEqual(expect.arrayContaining(["min-w-0", "truncate"]));
    expect(state.textContent).toBe(" · stopped");
    expect(state.className.split(" ")).toEqual(expect.arrayContaining(["shrink-0", "whitespace-pre"]));
  });

  it("explains a stopped machine's pane and offers to start or forget that machine", () => {
    seed(11);
    renderScreen();
    expect(screen.getByRole("button", { name: "Claude 2 · zoo-006 (stopped)" }).getAttribute("title")).toBe(STOPPED);
    const banner = screen.getAllByRole("status").find((element) => element.textContent?.includes(STOPPED)) as HTMLElement;
    expect(banner).toBeTruthy();
    expect((screen.getByRole("button", { name: "Send conversation message" }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(within(banner).getByRole("button", { name: "Start on zoo-006" }));
    const [start] = sent("start_machine_project_cli");
    expect(start).toEqual({ type: "start_machine_project_cli", machine_id: ZOO6, project_id: SID });
    expectValid(start);

    fireEvent.click(within(banner).getByRole("button", { name: "Forget zoo-006's tabs" }));
    const confirmSheet = screen.getByRole("dialog", { name: "Forget zoo-006's tabs?" });
    expect(confirmSheet.textContent).toContain("They come back if zoo-006 runs this project again.");
    fireEvent.click(within(confirmSheet).getByRole("button", { name: "Forget tabs" }));
    const [forget] = sent("forget_machine_panes");
    expect(forget).toEqual({ type: "forget_machine_panes", session_id: SID, machine_id: ZOO6 });
    expectValid(forget);
  });

  it("opens a stopped pane's raw terminal to read, not to type into", async () => {
    seed(11);
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(screen.getByRole("button", { name: "Open raw terminal" }));

    const terminal = screen.getByRole("region", { name: "Mobile terminal" });
    // The session's banner is hidden behind the terminal, so it says why.
    expect(within(terminal).getByRole("status").textContent).toContain(STOPPED);
    expect((await within(terminal).findByText("Raw terminal pane 11")).getAttribute("data-input-disabled")).toBe(STOPPED);
    for (const key of within(terminal).getByRole("toolbar", { name: "Terminal keys" }).querySelectorAll("button")) {
      expect(key.hasAttribute("disabled")).toBe(true);
      expect(key.getAttribute("title")).toBe(STOPPED);
      fireEvent.click(key);
    }
    expect(sent("terminal_input")).toEqual([]);
  });

  it("opens a running pane's raw terminal for typing", async () => {
    seed(10);
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(screen.getByRole("button", { name: "Open raw terminal" }));

    const terminal = screen.getByRole("region", { name: "Mobile terminal" });
    expect(within(terminal).queryByRole("status")).toBeNull();
    expect((await within(terminal).findByText("Raw terminal pane 10")).hasAttribute("data-input-disabled")).toBe(false);
    fireEvent.click(within(terminal).getByRole("button", { name: "Send Escape" }));
    const [input] = sent("terminal_input");
    expect(input).toMatchObject({ type: "terminal_input", session_id: SID, pane_id: 10 });
    expectValid(input);
  });

  it("disables a stopped pane's actions with the reason", () => {
    seed(11);
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    for (const name of ["Close this pane", "Reboot this pane", "Rename conversation"]) {
      const button = screen.getByRole("button", { name }) as HTMLButtonElement;
      expect(button.disabled).toBe(true);
      expect(button.title).toBe(STOPPED);
    }
  });

  it("will not answer a stopped pane's questions, approvals or plan reviews", () => {
    // The server refuses each and keeps it pending; the plan review used to
    // vanish on an optimistic answer and the question to read as answered.
    seed(11);
    const at = new Date();
    act(() => useStore.setState({
      paneMessages: {
        "11": [
          {
            id: "approval-11",
            role: "assistant",
            content: "",
            timestamp: at,
            sessionId: SID,
            paneId: 11,
            outputType: { type: "approval_request", toolCallId: "tool-approval-11", tool: "Bash", description: "rm -rf build" },
          },
          {
            id: "question-11",
            role: "assistant",
            content: "",
            timestamp: at,
            sessionId: SID,
            paneId: 11,
            outputType: {
              type: "tool_use",
              tool: "AskUserQuestion",
              toolUseId: "toolu-question-11",
              input: { questions: [{ question: "Ship it?", options: [{ label: "Yes" }, { label: "No" }] }] },
            },
          },
        ],
      },
      planReviewPending: [{ paneId: 11, toolUseId: "toolu-plan-11", toolName: "Bash", input: { command: "ls" }, arrivedAt: Date.now() }],
    }));
    renderScreen();

    for (const name of ["Approve", "Reject", "Deny", "Yes", "Submit"]) {
      for (const button of screen.getAllByRole("button", { name }) as HTMLButtonElement[]) {
        expect(button.disabled).toBe(true);
        fireEvent.click(button);
      }
    }
    // The plan review, the approval and the question each say why.
    expect(screen.getAllByRole("status").filter((element) => element.textContent === STOPPED).length)
      .toBeGreaterThanOrEqual(3);
    expect(sent("approve")).toEqual([]);
    expect(sent("reject")).toEqual([]);
    expect(sent("answer_question")).toEqual([]);
    expect(sent("plan_review_answer")).toEqual([]);
    const state = useStore.getState();
    expect(state.planReviewPending).toHaveLength(1);
    expect(state.answeredQuestions.has("toolu-question-11")).toBe(false);
  });
});

describe("opening a stopped project", () => {
  function stoppedOn(zoo5: Partial<MachineWithProjects>) {
    // Every tab is zoo-005's. The laptop joined with no tabs and was the
    // last to stop, so it is the session's recorded host and the machine
    // the screen addresses.
    seed(10, []);
    const registration = (machineId: string, hostname: string): MachineWithProjects => ({
      ...machine(machineId, hostname, true),
      projects: [{ projectId: SID, path: "/work/project", isRunning: false }],
    });
    act(() => useStore.setState({
      isAttached: false,
      machines: [{ ...registration(ZOO5, "zoo-005"), ...zoo5 }, registration(LAPTOP, "laptop")],
      paneConfigs: [terminal(10, "Claude 1", ZOO5, "zoo-005"), terminal(11, "Claude 2", ZOO5, "zoo-005")],
      sessions: [{ ...useStore.getState().sessions[0], isActive: false, status: "offline", hostname: "laptop" }],
    }));
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(screen.getByRole("button", { name: "Open project" }));
    // The project is named after its repository, "project".
    return screen.getByRole("dialog", { name: "Open project" });
  }

  it("names the machine with the tabs, which is the one the server starts", () => {
    const sheet = stoppedOn({});
    expect(sheet.textContent).toContain("Its APAS process starts on zoo-005, and its saved panes become available again.");
    expect(sheet.textContent).not.toContain("laptop");

    fireEvent.click(within(sheet).getByRole("button", { name: "Open project" }));
    const [start] = sent("start_machine_project_cli");
    expect(start).toEqual({ type: "start_machine_project_cli", machine_id: LAPTOP, project_id: SID, all_instances: true });
    expectValid(start);
  });

  it("names the machine the request names when no machine with tabs can start", () => {
    const sheet = stoppedOn({ daemonConnected: false, projectInstancesAvailable: false });
    expect(sheet.textContent).toContain("Its APAS process starts on laptop, and its saved panes become available again.");
  });
});

describe("choosing a machine for a new mobile agent", () => {
  it("adds the agent on the chosen machine and shows its launch", () => {
    seed(10, [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")]);
    renderScreen();
    const sheet = openInteractiveSheet();
    const step = within(sheet).getByRole("group", { name: "Run on" });
    expect((within(step).getByRole("radio", { name: /^zoo-005/ }) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(within(step).getByRole("radio", { name: /^zoo-006/ }));
    fireEvent.click(within(sheet).getByRole("button", { name: /^Claude terminal/ }));

    const [add] = sent("add_pane");
    expect(add).toMatchObject({ session_id: SID, machine_id: ZOO6, kind: "terminal" });
    expectValid(add);
    const launches = screen.getByRole("status", { name: "New tab launches" });
    expect(launches.textContent).toContain("on zoo-006 — Requesting the tab…");

    act(() => handleServerMessage(
      { type: "add_pane_result", session_id: SID, request_id: add.request_id, status: "failed", machine_id: ZOO6, error: "zoo-006 refused" },
      useStore.setState,
      useStore.getState,
    ));
    expect(screen.getByRole("status", { name: "New tab launches" }).textContent).toContain("zoo-006 refused");
  });

  it("confirms cloning onto an owned machine before asking for it", () => {
    seed(10);
    renderScreen();
    const sheet = openInteractiveSheet();
    fireEvent.click(within(sheet).getByRole("radio", { name: /^laptop/ }));
    fireEvent.click(within(sheet).getByRole("button", { name: /^Claude terminal/ }));
    expect(sent("create_project_instance")).toEqual([]);

    const confirmSheet = screen.getByRole("dialog", { name: "Clone this project onto laptop?" });
    expect(confirmSheet.textContent).toContain("~/apas_projects/project (auto-suffixed, as project-2 and so on, if that folder exists)");
    fireEvent.click(within(confirmSheet).getByRole("button", { name: "Clone and start" }));
    const [clone] = sent("create_project_instance");
    expect(clone).toMatchObject({ machine_id: LAPTOP, join_project_id: SID, instance_name: "project" });
    expectValid(clone);
    expect(screen.getByRole("status", { name: "New tab launches" }).textContent).toContain("Cloning the repository onto laptop…");
  });

  it("says a stopped project starts first, even when its machine is the only choice", () => {
    // A stopped single-machine project: adding an agent starts it there,
    // relaunching its saved panes, so the sheet says so before the tap.
    seed(10, []);
    act(() => useStore.setState({
      isAttached: false,
      machines: [machine(ZOO5, "zoo-005", true)],
      paneConfigs: [terminal(10, "Claude 1", ZOO5, "zoo-005")],
      sessions: [{ ...useStore.getState().sessions[0], isActive: false, gitRemote: undefined }],
    }));
    renderScreen();
    const sheet = openInteractiveSheet();
    const starts = within(within(sheet).getByRole("group", { name: "Run on" }))
      .getByRole("group", { name: "Starts the project first" });
    expect((within(starts).getByRole("radio", { name: /^zoo-005/ }) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(within(sheet).getByRole("button", { name: /^Claude terminal/ }));
    const [add] = sent("add_pane");
    expect(add).toMatchObject({ session_id: SID, machine_id: ZOO5, kind: "terminal" });
    expectValid(add);
  });

  it("leaves the sheet unchanged with a single eligible machine", () => {
    seed(10);
    act(() => useStore.setState({
      machines: [machine(ZOO5, "zoo-005", true)],
      sessions: [{ ...useStore.getState().sessions[0], gitRemote: undefined }],
    }));
    renderScreen();
    const sheet = openInteractiveSheet();
    expect(within(sheet).queryByRole("group", { name: "Run on" })).toBeNull();
    fireEvent.click(within(sheet).getByRole("button", { name: /^Claude terminal/ }));
    const [add] = sent("add_pane");
    // The default machine is still named, so the tab cannot land elsewhere.
    expect(add.machine_id).toBe(ZOO5);
  });
});
