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
  TerminalPane: ({ paneId }: { paneId: number }) => <div>Raw terminal pane {paneId}</div>,
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
    expect(confirmSheet.textContent).toContain("~/apas_projects/project");
    fireEvent.click(within(confirmSheet).getByRole("button", { name: "Clone and start" }));
    const [clone] = sent("create_project_instance");
    expect(clone).toMatchObject({ machine_id: LAPTOP, join_project_id: SID, instance_name: "project" });
    expectValid(clone);
    expect(screen.getByRole("status", { name: "New tab launches" }).textContent).toContain("Cloning the repository onto laptop…");
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
