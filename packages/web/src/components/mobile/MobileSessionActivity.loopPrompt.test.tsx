import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage } from "@apas/protocol";
import { useStore, type MachineWithProjects, type PaneConfig } from "@/lib/store";
import { writeSelectedPane } from "@/lib/mobileSelectedPane";
import { PROJECT_INSTANCES_CAPABILITY } from "@/lib/projectInstances";
import { TAB_AUTOPILOT_CAPABILITY } from "@/lib/autopilot";
import { DEFAULT_AUTOPILOT_TEMPLATE } from "@/lib/autopilotTemplates";
import { loopPromptMessage } from "@/lib/loopPrompt";
import { MobileSessionActivity } from "./MobileSessionActivity";

/**
 * The loop prompt on the mobile-browser session screen: where its entry
 * stands among the pane actions, when it is disabled, and the sheet's one
 * send, driven through the real store.
 */

vi.mock("@/components/tabs/TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => <div>Raw terminal pane {paneId}</div>,
}));

const SID = "20000000-0000-4000-8000-000000000001";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
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

let ws: { readyState: number; send: Mock };

function seed(
  panes: PaneConfig[],
  selectedPaneId: number,
  { running = true, runtimeAccess = true }: { running?: boolean; runtimeAccess?: boolean } = {},
) {
  writeSelectedPane(SID, selectedPaneId);
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: running,
      sessionId: SID,
      negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY, TAB_AUTOPILOT_CAPABILITY]),
      sessions: [{
        id: SID,
        projectId: SID,
        status: running ? "active" : "inactive",
        isActive: running,
        hostname: "zoo-006",
        workingDir: "/work/project",
        instances: running ? [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess }] : [],
      }],
      sessionInstances: {},
      machines: [zoo6],
      projectPolicies: {},
      paneConfigs: panes,
      paneListReceived: true,
      paneMessages: {},
      messages: [],
      paneStatuses: {},
      paneHasMore: {},
      pendingTerminalDeliveries: [],
      answeredQuestions: new Map(),
      planReviewPending: [],
      addPaneRequests: {},
      paneAutopilot: {},
      autopilotRequests: {},
      loadSessionActivity: vi.fn(),
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      listMachines: vi.fn(),
      showToast: vi.fn(),
    });
  });
}

function renderScreen() {
  render(<MobileSessionActivity connected onBack={vi.fn()} onReconnect={vi.fn()} />);
}

function openActions(): HTMLElement {
  fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  return screen.getByRole("dialog", { name: "Pane actions" });
}

function loopEntry(actions: HTMLElement): HTMLButtonElement | null {
  return within(actions).queryByRole("button", { name: /^Send a loop prompt/ }) as HTMLButtonElement | null;
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

describe("the loop prompt on a phone", () => {
  it("is among a Claude tab's pane actions, and its sheet sends one message", () => {
    seed([tab(3, "Claude 1")], 3);
    renderScreen();
    const entry = loopEntry(openActions())!;
    expect(entry.disabled).toBe(false);
    // Starting autopilot is not offered on a Claude tab.
    expect(within(screen.getByRole("dialog", { name: "Pane actions" })).queryByRole("button", { name: /Start autopilot/ })).toBeNull();
    fireEvent.click(entry);

    const sheet = screen.getByRole("dialog", { name: "Loop prompt for Claude 1" });
    expect(within(sheet).getByTestId("loop-prompt-warning").textContent).toContain("This is a shortcut, not autopilot.");
    expect(within(sheet).getByTestId("loop-prompt-warning").textContent).toContain("permissions on zoo-006");
    // A phone's field text is 16px, so iOS does not zoom on focus.
    expect((within(sheet).getByLabelText("Prompt") as HTMLTextAreaElement).className.split(" ")).toContain("text-[16px]");
    expect(within(sheet).getByTestId("loop-prompt-footer").className.split(" ")).toContain("shrink-0");

    fireEvent.click(within(sheet).getByRole("button", { name: "Send loop prompt" }));
    const frames = ws.send.mock.calls
      .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
      .filter((frame) => frame.type === "terminal_conversation_input");
    expect(frames).toHaveLength(1);
    expect(validateClientMessage(frames[0]).errors).toEqual([]);
    expect(frames[0]).toMatchObject({ pane_id: 3, text: loopPromptMessage(DEFAULT_AUTOPILOT_TEMPLATE.prompt) });
    expect(within(sheet).queryByRole("button", { name: "Send loop prompt" })).toBeNull();
    // No autopilot notice, marker or state appears for the tab.
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(useStore.getState().paneAutopilot).toEqual({});
  });

  it("is offered on Codex and OpenCode tabs too", () => {
    seed([tab(3, "Codex 1", { provider: "codex" }), tab(4, "OpenCode 1", { provider: "opencode" })], 3);
    renderScreen();
    expect(loopEntry(openActions())).not.toBeNull();
  });

  it("is disabled with the reason while the project is not running", () => {
    seed([tab(3, "Claude 1")], 3, { running: false });
    renderScreen();
    const entry = loopEntry(openActions())!;
    expect(entry.disabled).toBe(true);
    expect(entry.textContent).toContain("This project is not running. Open it first.");
  });

  it("is disabled with the server's reason for a viewer who may not use the tab's machine, and sends nothing", () => {
    seed([tab(3, "Claude 1")], 3, { runtimeAccess: false });
    renderScreen();
    const entry = loopEntry(openActions())!;
    const reason = "Access denied: you cannot use zoo-006, which runs this project instance";
    expect(entry.disabled).toBe(true);
    expect(entry.title).toBe(reason);
    expect(entry.textContent).toContain(reason);
    fireEvent.click(entry);
    expect(screen.queryByRole("dialog", { name: /Loop prompt/ })).toBeNull();
    expect(ws.send.mock.calls.map(([raw]) => JSON.parse(String(raw)).type)).not.toContain("terminal_conversation_input");
  });

  it("is not offered on an OMP tab, which has autopilot", () => {
    seed([tab(7, "OMP 1", { provider: "omp" })], 7);
    renderScreen();
    expect(loopEntry(openActions())).toBeNull();
  });

  it("is not offered on read-only legacy bot history", () => {
    seed([tab(3, "Old bot", { mode: "deadloop" })], 3);
    renderScreen();
    expect(loopEntry(openActions())).toBeNull();
  });
});
