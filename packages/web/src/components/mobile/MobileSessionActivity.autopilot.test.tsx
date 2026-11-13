import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage, type PaneAutopilotState } from "@apas/protocol";
import {
  handleServerMessage,
  paneKey,
  useStore,
  type MachineWithProjects,
  type Message,
  type PaneConfig,
} from "@/lib/store";
import { writeSelectedPane } from "@/lib/mobileSelectedPane";
import { PROJECT_INSTANCES_CAPABILITY, type SessionInstance } from "@/lib/projectInstances";
import { paneAutopilotKey, TAB_AUTOPILOT_CAPABILITY, type PaneAutopilotEntry } from "@/lib/autopilot";
import { MobileSessionActivity } from "./MobileSessionActivity";

/**
 * Autopilot on the mobile-browser session screen: starting from the tab's
 * actions, the state notice above the composer, the chip glyph, the marker
 * line and the confirmations, driven through the real store. Every frame the
 * screen sends is checked against the protocol schema the server parses.
 */

vi.mock("@/components/tabs/TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => <div>Raw terminal pane {paneId}</div>,
}));

const SID = "20000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const AUTOPILOT = "60000000-0000-4000-8000-000000000001";
const PROMPT = "Check the build and fix what fails.";
const NO_ACCESS = "Access denied: you cannot use zoo-006, which runs this project instance";
const STOP_NOW =
  "Stop now interrupts the turn in progress. Background work already launched keeps running, and a tab that was only waiting for such work can still start a turn when it finishes.";
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

type View = NonNullable<PaneAutopilotState["autopilot"]>;

function autopilot(kind: string, detail?: string, overrides: Partial<View> = {}): PaneAutopilotState {
  return {
    autopilot: {
      autopilot_id: AUTOPILOT,
      spec: {
        prompt: PROMPT,
        repeat: { kind: "after_each_turn" },
        iteration_limit: { kind: "at_most", value: 20 },
        time_limit: { kind: "none" },
        end_without_tool_call: true,
      },
      started_at: "2026-11-13T14:00:00Z",
      iterations: 3,
      condition: { kind, detail },
      ...overrides,
    },
  };
}

function ended(reason: string, turnSince = false): PaneAutopilotState {
  return {
    last_ended: {
      autopilot_id: AUTOPILOT,
      spec: {
        prompt: PROMPT,
        repeat: { kind: "every", seconds: 900 },
        iteration_limit: { kind: "none" },
        time_limit: { kind: "at_most", value: 8 * 3600 },
        end_without_tool_call: false,
      },
      started_at: "2026-11-13T06:00:00Z",
      iterations: 31,
      reason,
      ended_at: "2026-11-13T14:00:00Z",
      turn_since: turnSince,
    },
  };
}

function machine(machineId: string, hostname: string): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [{ projectId: SID, path: "/work/project", isRunning: true }],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
  };
}

let ws: { readyState: number; send: Mock };

function seed(
  panes: PaneConfig[],
  selectedPaneId: number,
  options: {
    negotiated?: boolean;
    entries?: Record<number, PaneAutopilotEntry>;
    instances?: SessionInstance[];
    paneStatuses?: Record<string, string | null>;
    paneMessages?: Record<string, Message[]>;
  } = {},
) {
  writeSelectedPane(SID, selectedPaneId);
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  const capabilities = [PROJECT_INSTANCES_CAPABILITY];
  if (options.negotiated ?? true) capabilities.push(TAB_AUTOPILOT_CAPABILITY);
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: true,
      sessionId: SID,
      negotiatedCapabilities: new Set(capabilities),
      sessions: [{
        id: SID,
        projectId: SID,
        status: "active",
        isActive: true,
        hostname: "zoo-006",
        workingDir: "/work/project",
        instances: options.instances ?? [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess: true }],
      }],
      sessionInstances: {},
      machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
      projectPolicies: {},
      paneConfigs: panes,
      paneListReceived: true,
      paneMessages: options.paneMessages ?? {},
      messages: [],
      paneStatuses: options.paneStatuses ?? {},
      paneHasMore: {},
      pendingTerminalDeliveries: [],
      answeredQuestions: new Map(),
      planReviewPending: [],
      addPaneRequests: {},
      paneAutopilot: Object.fromEntries(Object.entries(options.entries ?? {})
        .map(([paneId, entry]) => [paneAutopilotKey(SID, Number(paneId)), entry])),
      autopilotRequests: {},
      loadSessionActivity: vi.fn(),
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      listMachines: vi.fn(),
      showToast: vi.fn(),
    });
  });
}

function frames(type: string): Record<string, unknown>[] {
  return ws.send.mock.calls
    .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
    .filter((frame) => frame.type === type)
    .map((frame) => {
      expect(validateClientMessage(frame).errors).toEqual([]);
      return frame;
    });
}

/** The autopilot actions sent so far, each from a frame the server parses. */
function sentActions(): Record<string, unknown>[] {
  return frames("control_pane_autopilot").map((frame) => {
    expect(frame).toMatchObject({ session_id: SID });
    return frame.action as Record<string, unknown>;
  });
}

function deliver(message: Record<string, unknown>) {
  act(() => handleServerMessage(message, useStore.setState, useStore.getState));
}

function report(paneId: number, state: PaneAutopilotState | undefined, support = "supported") {
  deliver({ type: "pane_autopilot_state", session_id: SID, pane_id: paneId, support, state });
}

function renderScreen(connected = true) {
  const view = render(<MobileSessionActivity connected={connected} onBack={vi.fn()} onReconnect={vi.fn()} />);
  return (next: boolean) =>
    view.rerender(<MobileSessionActivity connected={next} onBack={vi.fn()} onReconnect={vi.fn()} />);
}

/** The state notice above the composer. */
function notice(): HTMLElement {
  return screen.getByRole("group", { name: "Autopilot" });
}

function openActions(): HTMLElement {
  fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  return screen.getByRole("dialog", { name: "Pane actions" });
}

function composer(): HTMLTextAreaElement {
  return screen.getByPlaceholderText("Message this terminal conversation") as HTMLTextAreaElement;
}

function button(scope: HTMLElement, name: string | RegExp): HTMLButtonElement {
  return within(scope).getByRole("button", { name }) as HTMLButtonElement;
}

function iterationMessage(id: string, input: unknown, at: string): Message {
  return {
    id,
    role: "assistant",
    content: "",
    timestamp: new Date(at),
    sessionId: SID,
    paneId: 7,
    outputType: { type: "tool_use", tool: "ApasAutopilotIteration", toolUseId: `toolu-${id}`, input },
  };
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

describe("starting autopilot from a phone", () => {
  it("offers Start autopilot among the tab's actions, with the desktop's choices and statement", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    renderScreen();
    // A tab with no autopilot has no state notice.
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();

    fireEvent.click(button(openActions(), "Start autopilot"));
    expect(screen.queryByRole("dialog", { name: "Pane actions" })).toBeNull();
    const sheet = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
    // A bottom sheet, and the keyboard does not open over the statement.
    expect(sheet.className).toContain("rounded-t-[1.4rem]");
    expect(sheet.parentElement?.className).toContain("items-end");
    expect(document.activeElement).not.toBe(within(sheet).getByLabelText("Prompt"));

    const statement = within(sheet).getByRole("list", { name: "What autopilot is and is not" }).textContent ?? "";
    expect(statement).toContain("This tab keeps running on zoo-006");
    expect(statement).toContain("it spends the OMP login configured there, whoever starts it");
    expect(statement).toContain("APAS adds no sandbox, no independent review and no spending limit.");
    expect(statement).toContain("are not a security boundary");
    expect(statement).toContain("it may have to be answered in the terminal");
    expect(statement).toContain("This tab works in the project's shared checkout.");

    // No limit is what the sheet proposes, and it says what that means.
    expect((within(sheet).getByRole("radio", { name: "As soon as each turn ends" }) as HTMLInputElement).checked).toBe(true);
    for (const radio of within(sheet).getAllByRole("radio", { name: "No limit" }) as HTMLInputElement[]) {
      expect(radio.checked).toBe(true);
    }
    expect(within(sheet).getByTestId("autopilot-limits-statement").textContent)
      .toContain("No limit is set. Autopilot then runs until someone stops it.");
    // Opening the sheet sent nothing.
    expect(sentActions()).toEqual([]);

    fireEvent.change(within(sheet).getByLabelText("Prompt"), { target: { value: PROMPT } });
    fireEvent.click(within(sheet).getByRole("radio", { name: "Every" }));
    expect(within(sheet).getByTestId("autopilot-limits-statement").textContent)
      .toContain("it runs until a limit is reached or a person stops it");
    const [iterations] = within(sheet).getAllByRole("radio", { name: "At most" });
    fireEvent.click(iterations);
    fireEvent.change(within(sheet).getByLabelText("Iteration limit"), { target: { value: "20" } });
    fireEvent.click(button(sheet, "Start autopilot"));

    expect(sentActions()).toEqual([{
      kind: "start",
      spec: {
        prompt: PROMPT,
        repeat: { kind: "every", seconds: 900 },
        iteration_limit: { kind: "at_most", value: 20 },
        time_limit: { kind: "none" },
        end_without_tool_call: false,
      },
    }]);
    expect(frames("control_pane_autopilot")[0]).toMatchObject({ pane_id: 7 });
    expect(screen.queryByRole("dialog", { name: "Start autopilot — OMP 1" })).toBeNull();
    expect(screen.getByRole("status", { name: "Autopilot requests" }).textContent).toContain("Start autopilot on OMP 1");
    // Nothing is shown as the tab's state until the tab reports it.
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();

    // A second start would only be refused while the first is on its way.
    const again = button(openActions(), /^Start autopilot/);
    expect(again.disabled).toBe(true);
    expect(again.textContent).toContain("A start is on its way to this tab.");
  });

  it("says a tab in a worktree of its own does not work in the shared checkout", () => {
    seed([tab(7, "OMP 1", { worktree_path: "/work/.apas-worktrees/pane-7" })], 7, {
      entries: { 7: { support: "supported" } },
    });
    renderScreen();
    fireEvent.click(button(openActions(), "Start autopilot"));
    expect(screen.getByRole("list", { name: "What autopilot is and is not" }).textContent)
      .toContain("This tab works in its own worktree, not in the project's shared checkout.");
  });

  it("refuses a time limit shorter than the interval before anything is sent", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    renderScreen();
    fireEvent.click(button(openActions(), "Start autopilot"));
    const sheet = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
    fireEvent.change(within(sheet).getByLabelText("Prompt"), { target: { value: PROMPT } });
    fireEvent.click(within(sheet).getByRole("radio", { name: "Every" }));
    fireEvent.click(within(sheet).getAllByRole("radio", { name: "At most" })[1]);
    fireEvent.change(within(sheet).getByLabelText("Time limit"), { target: { value: "5" } });
    fireEvent.change(within(sheet).getByLabelText("Time limit unit"), { target: { value: "minutes" } });
    fireEvent.click(button(sheet, "Start autopilot"));
    expect(within(sheet).getByRole("alert").textContent).toBe("The time limit is shorter than the repeat interval.");
    expect(sentActions()).toEqual([]);
  });

  it("offers nothing about autopilot for other tabs, legacy bot history, or on a server without it", () => {
    seed([
      tab(7, "Claude TTY", { provider: "claude" }),
      tab(8, "Old bot", { mode: "deadloop" }),
    ], 7, { entries: { 7: { support: "supported" }, 8: { support: "supported" } } });
    const view = renderScreen();
    expect(openActions().textContent).not.toMatch(/autopilot/i);
    fireEvent.click(screen.getByRole("button", { name: "Close actions" }));

    fireEvent.click(screen.getByRole("button", { name: "Old bot" }));
    expect(openActions().textContent).not.toMatch(/autopilot/i);
    fireEvent.click(screen.getByRole("button", { name: "Close actions" }));

    // An OMP tab on autopilot, on a server that negotiated nothing: no
    // action, no notice and no glyph.
    seed([tab(7, "OMP 1")], 7, {
      negotiated: false,
      entries: { 7: { support: "supported", state: autopilot("running") } },
    });
    view(true);
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.queryByTestId("chip-autopilot-glyph")).toBeNull();
    expect(screen.getByRole("button", { name: "OMP 1" })).toBeTruthy();
    expect(openActions().textContent).not.toMatch(/autopilot/i);
  });

  it.each([
    ["needs_agent_reboot", "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress."],
    ["instance_update_required", "zoo-006 runs an older APAS that cannot run autopilot. Reboot zoo-006 to update it."],
    ["unsupported_omp", "This tab's OMP version cannot run autopilot."],
    ["unavailable_on_platform", "Autopilot is not available for tabs on this machine."],
  ] as const)("shows Start autopilot disabled with the reason for %s", (support, reason) => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support } } });
    renderScreen();
    const sheet = openActions();
    const start = button(sheet, /^Start autopilot/);
    expect(start.disabled).toBe(true);
    // The reason is on the screen, not only in a title a phone never shows,
    // and it is not dimmed with the action it explains.
    expect(start.textContent).toContain(reason);
    expect(within(start).getByText(reason).className).not.toContain("opacity");
    expect(start.className).not.toContain("opacity");
    expect(within(start).getByText("Start autopilot").className).toContain("opacity-40");
    // Nothing is offered that would only be refused, and nothing reboots.
    expect(within(sheet).queryByRole("button", { name: /^Stop autopilot/ })).toBeNull();
    fireEvent.click(start);
    expect(screen.queryByRole("dialog", { name: /^Start autopilot/ })).toBeNull();
    expect(sentActions()).toEqual([]);
    expect(frames("reboot_pane")).toEqual([]);
  });
});

describe("the state notice", () => {
  it("shows a tab waiting for its next iteration with the time, offers Pause and Stop, and leaves the composer usable", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: autopilot("waiting", "next_iteration", { next_iteration_at: "2026-11-13T14:30:00Z" }),
        },
      },
    });
    renderScreen();

    expect(notice().getAttribute("data-autopilot")).toBe("on");
    expect(within(notice()).getByRole("status").textContent)
      .toMatch(/^Autopilot · Waiting for the next iteration at \S.* · 3 of 20 iterations$/);
    const offered = within(notice()).getAllByRole("button").map((element) => element.textContent);
    expect(offered).toEqual(["Pause", "Stop", "Details"]);
    for (const element of within(notice()).getAllByRole("button") as HTMLButtonElement[]) {
      expect(element.disabled).toBe(false);
    }
    // A state notice above the composer, not one of the occasional actions.
    expect(notice().compareDocumentPosition(composer()) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(openActions().textContent).not.toMatch(/autopilot/i);
    fireEvent.click(screen.getByRole("button", { name: "Close actions" }));

    // Steering the tab is an ordinary message, and no autopilot request.
    expect(composer().disabled).toBe(false);
    fireEvent.change(composer(), { target: { value: "Also check the docs build." } });
    const send = screen.getByRole("button", { name: "Send conversation message" }) as HTMLButtonElement;
    expect(send.disabled).toBe(false);
    fireEvent.click(send);
    expect(frames("terminal_conversation_input")).toHaveLength(1);
    expect(frames("terminal_conversation_input")[0]).toMatchObject({ session_id: SID, pane_id: 7 });
    expect(sentActions()).toEqual([]);
    // The notice is still what the tab reported.
    expect(within(notice()).getByRole("status").textContent).toContain("Waiting for the next iteration");
  });

  it("stands beside the tab's status and never replaces it", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("running") } },
      paneStatuses: { [paneKey(7)]: "Working" },
    });
    renderScreen();
    expect(within(notice()).getByRole("status").textContent).toContain("Autopilot · Running an iteration");
    const statuses = screen.getAllByRole("status").map((element) => element.textContent);
    expect(statuses).toContain("Working");
  });

  it("pauses and resumes from the notice, changing only when the tab reports", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    renderScreen();

    fireEvent.click(button(notice(), "Pause"));
    expect(sentActions()).toEqual([{ kind: "pause", autopilot_id: AUTOPILOT }]);
    // An accepted request is not the state.
    const [requestId] = Object.keys(useStore.getState().autopilotRequests);
    deliver({ type: "pane_autopilot_receipt", session_id: SID, pane_id: 7, request_id: requestId, outcome: "accepted" });
    expect(within(notice()).getByRole("status").textContent).toContain("Running an iteration");

    report(7, autopilot("paused", "requested", { iterations: 7 }));
    expect(notice().getAttribute("data-autopilot")).toBe("paused");
    expect(within(notice()).getByRole("status").textContent).toBe("Autopilot · Paused · 7 of 20 iterations");
    expect(within(notice()).getAllByRole("button").map((element) => element.textContent))
      .toEqual(["Resume", "Stop", "Details"]);

    fireEvent.click(button(notice(), "Resume"));
    expect(sentActions()[1]).toEqual({ kind: "resume", autopilot_id: AUTOPILOT });
  });

  it("asks whether to let the turn finish or interrupt it now, and says what Stop now leaves running", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("running") } },
      paneStatuses: { [paneKey(7)]: "Working" },
    });
    renderScreen();

    fireEvent.click(button(notice(), "Stop"));
    let sheet = screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" });
    // Choosing Stop on the notice has sent nothing yet.
    expect(sentActions()).toEqual([]);
    expect(button(sheet, /^Stop(?! now)/).textContent).toContain("A turn in progress runs to its end.");
    expect(button(sheet, /^Stop now/).textContent).toContain(STOP_NOW);
    fireEvent.click(button(sheet, "Cancel"));
    expect(screen.queryByRole("dialog", { name: "Stop autopilot on OMP 1" })).toBeNull();
    expect(sentActions()).toEqual([]);

    fireEvent.click(button(notice(), "Stop"));
    sheet = screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" });
    fireEvent.click(button(sheet, /^Stop now/));
    expect(sentActions()).toEqual([{ kind: "stop", autopilot_id: AUTOPILOT, now: true }]);
    expect(screen.queryByRole("dialog", { name: "Stop autopilot on OMP 1" })).toBeNull();

    fireEvent.click(button(notice(), "Stop"));
    fireEvent.click(button(screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" }), /^Stop(?! now)/));
    expect(sentActions()[1]).toEqual({ kind: "stop", autopilot_id: AUTOPILOT, now: false });

    // "Stopped" is shown once the tab reports it, not before.
    expect(within(notice()).getByRole("status").textContent).toContain("Running an iteration");
    report(7, ended("stopped"));
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.getByRole("status", { name: "Autopilot" }).textContent).toContain("Autopilot · Stopped");
  });

  it("closes the stop sheet with the autopilot it was opened for", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    renderScreen();
    fireEvent.click(button(notice(), "Stop"));
    expect(screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" })).toBeTruthy();

    report(7, ended("iteration_limit"));
    expect(screen.queryByRole("dialog", { name: "Stop autopilot on OMP 1" })).toBeNull();
    // The next autopilot does not bring the sheet back open by itself.
    report(7, autopilot("running"));
    expect(notice()).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "Stop autopilot on OMP 1" })).toBeNull();
  });

  it("shows the prompt, the timing, the limits or that none is set, the start, the progress and the time limit's end", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: autopilot("waiting", "settle", {
            spec: {
              prompt: PROMPT,
              repeat: { kind: "every", seconds: 900 },
              iteration_limit: { kind: "none" },
              time_limit: { kind: "at_most", value: 8 * 3600 },
              end_without_tool_call: false,
            },
            deadline: "2026-11-13T22:00:00Z",
          }),
        },
      },
    });
    renderScreen();
    fireEvent.click(button(notice(), "Details"));
    const sheet = screen.getByRole("dialog", { name: "Autopilot details" });
    expect(sheet.textContent).toContain("Waiting for the tab to settle");
    expect(sheet.textContent).toContain(PROMPT);
    const rows = Object.fromEntries(Array.from(sheet.querySelectorAll("dt"))
      .map((term) => [term.textContent, term.nextElementSibling?.textContent]));
    expect(rows).toMatchObject({
      Repeats: "Every 15 minutes",
      "Iteration limit": "No limit",
      "Time limit": "8 hours",
      Progress: "3 iterations",
    });
    expect(rows.Started).toBeTruthy();
    expect(rows["Time limit ends"]).toBeTruthy();
    // Reading the details sends nothing.
    expect(sentActions()).toEqual([]);

    fireEvent.click(button(sheet, "Close autopilot details"));
    expect(screen.queryByRole("dialog", { name: "Autopilot details" })).toBeNull();

    // Without a time limit there is no end to show.
    report(7, autopilot("running"));
    fireEvent.click(button(notice(), "Details"));
    const unlimited = screen.getByRole("dialog", { name: "Autopilot details" });
    expect(unlimited.textContent).toContain("3 of 20 iterations");
    expect(unlimited.textContent).not.toContain("Time limit ends");
    expect(Array.from(unlimited.querySelectorAll("dt")).find((term) => term.textContent === "Time limit")
      ?.nextElementSibling?.textContent).toBe("No limit");
  });

  it("offers no Resume, Stop or Stop now for an autopilot paused because the agent process exited", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: { ...autopilot("paused", "agent_exited"), reported_by_cli: true },
        },
      },
    });
    renderScreen();
    expect(within(notice()).getByRole("status").textContent).toContain("Autopilot · Paused: the agent process exited");
    expect(within(notice()).getByRole("note").textContent)
      .toBe("This tab's agent is not running. Reboot it first; its autopilot comes back paused.");
    // The state and its details stay readable; nothing could apply a request.
    expect(within(notice()).getAllByRole("button").map((element) => element.textContent)).toEqual(["Details"]);
    expect(openActions().textContent).not.toMatch(/autopilot/i);
    expect(sentActions()).toEqual([]);
  });

  it("shows an ended autopilot's reason with Start again, which offers that autopilot's values", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: ended("time_limit") } } });
    renderScreen();
    // Ended is not on: no notice with actions, and no glyph on the chip.
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.queryByTestId("chip-autopilot-glyph")).toBeNull();
    const line = screen.getByRole("status", { name: "Autopilot" });
    expect(line.textContent).toContain("Autopilot · Ended: time limit reached");
    expect(line.textContent).not.toMatch(/verified|delivered/i);

    fireEvent.click(button(line, "Start again"));
    const sheet = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
    expect((within(sheet).getByLabelText("Prompt") as HTMLTextAreaElement).value).toBe(PROMPT);
    expect((within(sheet).getByRole("radio", { name: "Every" }) as HTMLInputElement).checked).toBe(true);
    expect((within(sheet).getByLabelText("Repeat interval") as HTMLInputElement).value).toBe("15");
    expect((within(sheet).getByLabelText("Time limit") as HTMLInputElement).value).toBe("8");
    expect(within(sheet).getByTestId("autopilot-limits-statement").textContent).toContain("Autopilot ends after 8 hours.");
    // Nothing is sent until the person confirms the start.
    expect(sentActions()).toEqual([]);
    fireEvent.click(button(sheet, "Cancel"));
    expect(sentActions()).toEqual([]);

    // The tab's actions offer the same start, with the reason.
    const start = button(openActions(), /^Start autopilot/);
    expect(start.disabled).toBe(false);
    expect(start.textContent).toContain("Ended: time limit reached");
  });

  it("keeps the ended reason on the tab's actions once a later turn has run, and off the screen", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: ended("iteration_limit", true) } } });
    renderScreen();
    expect(screen.queryByRole("status", { name: "Autopilot" })).toBeNull();
    expect(button(openActions(), /^Start autopilot/).textContent).toContain("Ended: iteration limit reached");
  });
});

describe("who may use the tab from a phone", () => {
  it("lets a collaborator with runtime access on the tab's machine start, pause, resume and stop", () => {
    // Shared with this viewer and hosted by another account; the server says
    // they may use that machine.
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    act(() => useStore.setState((state) => ({
      sessions: state.sessions.map((session) => ({ ...session, isShared: true, shareRole: "user" as const })),
    })));
    renderScreen();

    const start = button(openActions(), "Start autopilot");
    expect(start.disabled).toBe(false);
    fireEvent.click(start);
    const sheet = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
    fireEvent.change(within(sheet).getByLabelText("Prompt"), { target: { value: "go" } });
    fireEvent.click(button(sheet, "Start autopilot"));

    report(7, autopilot("running"));
    expect(button(notice(), "Pause").disabled).toBe(false);
    fireEvent.click(button(notice(), "Pause"));
    report(7, autopilot("paused", "requested"));
    expect(button(notice(), "Resume").disabled).toBe(false);
    fireEvent.click(button(notice(), "Resume"));
    report(7, autopilot("running"));
    fireEvent.click(button(notice(), "Stop"));
    fireEvent.click(button(screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" }), /^Stop(?! now)/));

    expect(sentActions().map((action) => action.kind)).toEqual(["start", "pause", "resume", "stop"]);
  });

  it("shows a viewer without runtime access the state, with every action disabled and the reason", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("paused", "interrupted") } },
      instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess: false }],
    });
    renderScreen();

    expect(within(notice()).getByRole("status").textContent).toContain("Autopilot · Paused: the turn was interrupted");
    expect(within(notice()).getByRole("note").textContent).toBe(NO_ACCESS);
    for (const name of ["Resume", "Stop"]) {
      const action = button(notice(), name);
      expect(action.disabled).toBe(true);
      expect(action.title).toBe(NO_ACCESS);
      fireEvent.click(action);
    }
    expect(screen.queryByRole("dialog", { name: "Stop autopilot on OMP 1" })).toBeNull();
    // The state and its prompt stay readable.
    fireEvent.click(button(notice(), "Details"));
    expect(screen.getByRole("dialog", { name: "Autopilot details" }).textContent).toContain(PROMPT);
    expect(sentActions()).toEqual([]);
  });

  it("does not let a viewer without runtime access start either", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: ended("stopped") } },
      instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess: false }],
    });
    renderScreen();
    const line = screen.getByRole("status", { name: "Autopilot" });
    expect(button(line, "Start again").disabled).toBe(true);
    expect(line.textContent).toContain(NO_ACCESS);
    const start = button(openActions(), /^Start autopilot/);
    expect(start.disabled).toBe(true);
    expect(start.textContent).toContain(NO_ACCESS);
    fireEvent.click(start);
    expect(screen.queryByRole("dialog", { name: /^Start autopilot/ })).toBeNull();
    expect(sentActions()).toEqual([]);
  });
});

describe("a tab whose autopilot state is not reported", () => {
  it("shows no notice, and offers Stop and Stop now among the tab's actions", () => {
    // Nothing is held for the tab, as in the first seconds after a launch.
    seed([tab(7, "OMP 1")], 7);
    renderScreen();
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.queryByRole("status", { name: "Autopilot" })).toBeNull();
    expect(screen.queryByTestId("chip-autopilot-glyph")).toBeNull();

    const sheet = openActions();
    const actions = within(sheet).getByRole("group", { name: "Autopilot" });
    expect(actions.textContent).toContain("This tab has not reported its autopilot state.");
    // Not a reason to reboot, and never shown as off.
    expect(actions.textContent).not.toMatch(/Reboot/);
    const offered = within(actions).getAllByRole("button") as HTMLButtonElement[];
    expect(offered).toHaveLength(2);
    expect(offered[0].textContent).toMatch(/^Stop autopilot(?! now)/);
    expect(offered[1].textContent).toMatch(/^Stop autopilot now/);
    expect(offered[1].textContent).toContain(STOP_NOW);
    expect(offered.every((element) => !element.disabled)).toBe(true);
    // Starting, Pause and Resume need a state.
    expect(within(sheet).queryByRole("button", { name: /^Start autopilot/ })).toBeNull();
    expect(within(sheet).queryByRole("button", { name: /Pause|Resume/ })).toBeNull();

    // A stop needs no state, and names no autopilot.
    fireEvent.click(offered[0]);
    expect(sentActions()).toEqual([{ kind: "stop", now: false }]);
    expect(screen.queryByRole("dialog", { name: "Pane actions" })).toBeNull();
    expect(screen.getByRole("status", { name: "Autopilot requests" }).textContent).toContain("Stop autopilot on OMP 1");

    fireEvent.click(button(openActions(), /^Stop autopilot now/));
    expect(sentActions()[1]).toEqual({ kind: "stop", now: true });
    // Still no notice: nothing was reported.
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
  });

  it("says when the tab last reported, once its report has gone stale", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "not_reported", reportedAt: "2026-11-13T14:00:00Z" } },
    });
    renderScreen();
    const actions = within(openActions()).getByRole("group", { name: "Autopilot" });
    expect(actions.textContent).toMatch(/This tab has not reported its autopilot state\. Last reported at \S/);
  });

  it("goes back to not reported when the tab's machine drops off", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    renderScreen();
    expect(notice()).toBeTruthy();
    report(7, undefined, "not_reported");
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.queryByTestId("chip-autopilot-glyph")).toBeNull();
    expect(within(openActions()).getByRole("group", { name: "Autopilot" }).textContent)
      .toContain("This tab has not reported its autopilot state.");
  });

  it("disables Stop and Stop now with the reason while the project is not running", () => {
    seed([tab(7, "OMP 1")], 7, { instances: [] });
    act(() => useStore.setState((state) => ({
      isAttached: false,
      sessions: state.sessions.map((session) => ({ ...session, isActive: false, status: "offline" })),
    })));
    renderScreen();
    const actions = within(openActions()).getByRole("group", { name: "Autopilot" });
    expect(actions.textContent).toContain("This project is not running. Open it first.");
    for (const element of within(actions).getAllByRole("button") as HTMLButtonElement[]) {
      expect(element.disabled).toBe(true);
      fireEvent.click(element);
    }
    expect(sentActions()).toEqual([]);
  });
});

describe("offline or with the tab's machine unreachable", () => {
  it("disables every autopilot control with the reason and queues nothing for later", () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2"), tab(9, "OMP 3")], 7, {
      entries: {
        7: { support: "supported", state: autopilot("running") },
        8: { support: "supported" },
      },
    });
    ws.readyState = WebSocket.CLOSED;
    act(() => useStore.setState({ connected: false }));
    const view = renderScreen(false);

    expect(within(notice()).getByRole("note").textContent).toBe("Not connected to the server.");
    for (const name of ["Pause", "Stop"]) {
      const action = button(notice(), name);
      expect(action.disabled).toBe(true);
      expect(action.title).toBe("Not connected to the server.");
      fireEvent.click(action);
    }
    // The last reported state and its details stay readable.
    expect(within(notice()).getByRole("status").textContent).toContain("Running an iteration");
    expect(button(notice(), "Details").disabled).toBe(false);
    // Drafting a message stays possible, as it was.
    expect(composer().disabled).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: /^OMP 2/ }));
    const start = button(openActions(), /^Start autopilot/);
    expect(start.disabled).toBe(true);
    expect(start.textContent).toContain("Not connected to the server.");
    fireEvent.click(start);
    fireEvent.click(screen.getByRole("button", { name: "Close actions" }));

    // A tab the page holds nothing for, as every tab is once the socket has
    // closed: Stop and Stop now are listed, disabled, with the same reason.
    fireEvent.click(screen.getByRole("button", { name: /^OMP 3/ }));
    const stops = within(openActions()).getByRole("group", { name: "Autopilot" });
    expect(stops.textContent).toContain("Not connected to the server.");
    for (const element of within(stops).getAllByRole("button") as HTMLButtonElement[]) {
      expect(element.disabled).toBe(true);
      fireEvent.click(element);
    }
    fireEvent.click(screen.getByRole("button", { name: "Close actions" }));

    // The store refuses too, so no path can hold a request for later.
    const refused = useStore.getState().controlPaneAutopilot(SID, 7, { kind: "pause" });
    expect(refused).toEqual({ success: false, error: "Not connected to the server." });
    expect(useStore.getState().autopilotRequests).toEqual({});
    expect(ws.send).not.toHaveBeenCalled();

    // Back online: nothing that was tapped while offline is sent now.
    ws.readyState = WebSocket.OPEN;
    act(() => useStore.setState({ connected: true }));
    view(true);
    expect(ws.send).not.toHaveBeenCalled();
    expect(screen.queryByRole("status", { name: "Autopilot requests" })).toBeNull();
  });

  it("keeps a start that could not be sent in its sheet, with the reason and the prompt", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    const view = renderScreen();
    fireEvent.click(button(openActions(), "Start autopilot"));
    const sheet = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
    fireEvent.change(within(sheet).getByLabelText("Prompt"), { target: { value: PROMPT } });

    // The connection goes while the sheet is open.
    ws.readyState = WebSocket.CLOSED;
    act(() => useStore.setState({ connected: false }));
    view(false);
    fireEvent.click(button(sheet, "Start autopilot"));
    expect(within(sheet).getByRole("alert").textContent).toBe("Not connected to the server.");
    expect((within(sheet).getByLabelText("Prompt") as HTMLTextAreaElement).value).toBe(PROMPT);
    expect(ws.send).not.toHaveBeenCalled();
    expect(useStore.getState().autopilotRequests).toEqual({});
  });

  it("disables the actions with the tab's reason while its machine is not running the project", () => {
    // The project runs on zoo-005 only; the tab is zoo-006's.
    seed([tab(6, "OMP on 5", { machine_id: ZOO5, hostname: "zoo-005" }), tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("paused", "requested") } },
      instances: [{ machineId: ZOO5, hostname: "zoo-005", primary: true, runtimeAccess: true }],
    });
    act(() => useStore.setState((state) => ({
      machines: state.machines.map((entry) => entry.machine.machineId === ZOO6
        ? { ...entry, projects: [{ projectId: SID, path: "/work/project", isRunning: false }] }
        : entry),
    })));
    renderScreen();
    const reason = "zoo-006 is not running this project. Start it on zoo-006 first.";
    expect(within(notice()).getByRole("note").textContent).toBe(reason);
    for (const name of ["Resume", "Stop"]) {
      expect(button(notice(), name).disabled).toBe(true);
      fireEvent.click(button(notice(), name));
    }
    expect(sentActions()).toEqual([]);
  });
});

describe("requests from a phone", () => {
  it("keeps a request that was not confirmed on screen and does not send it again", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    renderScreen();
    fireEvent.click(button(notice(), "Stop"));
    fireEvent.click(button(screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" }), /^Stop(?! now)/));
    const [requestId] = Object.keys(useStore.getState().autopilotRequests);

    deliver({ type: "pane_autopilot_receipt", session_id: SID, pane_id: 7, request_id: requestId, outcome: "uncertain" });

    const requests = screen.getByRole("status", { name: "Autopilot requests" });
    expect(requests.textContent).toContain("Stop autopilot on OMP 1");
    expect(requests.textContent).toContain("Not confirmed.");
    expect(requests.textContent).toContain("It was not sent again.");
    expect(sentActions()).toHaveLength(1);
    // The state shown is still the tab's own report.
    expect(within(notice()).getByRole("status").textContent).toContain("Running an iteration");

    fireEvent.click(within(requests).getByRole("button", { name: "Dismiss Stop autopilot on OMP 1" }));
    expect(screen.queryByRole("status", { name: "Autopilot requests" })).toBeNull();
    expect(sentActions()).toHaveLength(1);
  });

  it("shows a refusal with the server's reason", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("paused", "requested") } } });
    renderScreen();
    fireEvent.click(button(notice(), "Resume"));
    const [requestId] = Object.keys(useStore.getState().autopilotRequests);
    deliver({
      type: "pane_autopilot_receipt",
      session_id: SID,
      pane_id: 7,
      request_id: requestId,
      outcome: "refused",
      reason: "This autopilot is no longer current.",
    });
    expect(screen.getByRole("status", { name: "Autopilot requests" }).textContent)
      .toContain("Resume autopilot on OMP 1 — This autopilot is no longer current.");
    expect(sentActions()).toHaveLength(1);
  });
});

describe("the raw terminal view", () => {
  function openTerminal(): HTMLElement {
    fireEvent.click(button(openActions(), "Open raw terminal"));
    return screen.getByRole("region", { name: "Mobile terminal" });
  }

  it("keeps the state notice and its actions while the terminal is open, on the same terminal", async () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: autopilot("waiting", "next_iteration", { next_iteration_at: "2026-11-13T14:30:00Z" }),
        },
      },
    });
    renderScreen();
    const terminal = openTerminal();
    const pane = await screen.findByText("Raw terminal pane 7");

    // The composer and the notice above it are hidden with the conversation;
    // the terminal carries the one notice that is on screen.
    expect(terminal.contains(notice())).toBe(true);
    expect(notice().getAttribute("data-autopilot")).toBe("on");
    expect(within(notice()).getByRole("status").textContent).toMatch(/^Autopilot · Waiting for the next iteration at \S/);
    expect(within(notice()).getAllByRole("button").map((element) => element.textContent))
      .toEqual(["Pause", "Stop", "Details"]);
    // It stands above the terminal, not over it.
    expect(notice().compareDocumentPosition(pane) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

    fireEvent.click(button(notice(), "Pause"));
    expect(sentActions()).toEqual([{ kind: "pause", autopilot_id: AUTOPILOT }]);
    // Still what the tab reported, until it reports the pause.
    expect(within(notice()).getByRole("status").textContent).toContain("Waiting for the next iteration");
    report(7, autopilot("paused", "requested"));
    expect(notice().getAttribute("data-autopilot")).toBe("paused");
    expect(within(notice()).getByRole("status").textContent).toBe("Autopilot · Paused");
    expect(button(notice(), "Resume").disabled).toBe(false);

    // Stop asks here too, and Details opens over the terminal.
    fireEvent.click(button(notice(), "Stop"));
    const sheet = screen.getByRole("dialog", { name: "Stop autopilot on OMP 1" });
    expect(button(sheet, /^Stop now/).textContent).toContain(STOP_NOW);
    expect(sentActions()).toHaveLength(1);
    fireEvent.click(button(sheet, /^Stop(?! now)/));
    expect(sentActions()[1]).toEqual({ kind: "stop", autopilot_id: AUTOPILOT, now: false });
    fireEvent.click(button(notice(), "Details"));
    expect(screen.getByRole("dialog", { name: "Autopilot details" }).textContent).toContain(PROMPT);
    fireEvent.click(screen.getByRole("button", { name: "Close autopilot details" }));

    // None of it remounted the terminal or left its view.
    expect(screen.getByText("Raw terminal pane 7")).toBe(pane);
    expect(screen.getByRole("region", { name: "Mobile terminal" })).toBe(terminal);

    // Gone with the autopilot; an ended one has its line in the conversation.
    report(7, ended("stopped"));
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    expect(screen.getByText("Raw terminal pane 7")).toBe(pane);
  });

  it("shows nothing there for a tab with no autopilot or with a state that is not reported", () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2")], 7, { entries: { 7: { support: "supported" } } });
    renderScreen();
    openTerminal();
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Conversation" }));

    fireEvent.click(screen.getByRole("button", { name: /^OMP 2/ }));
    openTerminal();
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
  });

  it("disables the actions there for a viewer without runtime access, saying why once", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("paused", "interrupted") } },
      instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true, runtimeAccess: false }],
    });
    renderScreen();
    const terminal = openTerminal();

    expect(within(notice()).getByRole("status").textContent).toBe("Autopilot · Paused: the turn was interrupted");
    for (const name of ["Resume", "Stop"]) {
      const action = button(notice(), name);
      expect(action.disabled).toBe(true);
      expect(action.title).toBe(NO_ACCESS);
      fireEvent.click(action);
    }
    expect(button(notice(), "Details").disabled).toBe(false);
    // The terminal already says why it takes no input; the notice does not
    // say it a second time.
    expect(within(notice()).queryByRole("note")).toBeNull();
    expect(terminal.textContent?.split(NO_ACCESS)).toHaveLength(2);
    expect(sentActions()).toEqual([]);
  });

  it("says there why nothing can be sent while offline, and that an exited agent must be rebooted first", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: { ...autopilot("paused", "agent_exited"), reported_by_cli: true },
        },
      },
    });
    const view = renderScreen();
    openTerminal();
    expect(within(notice()).getByRole("status").textContent).toBe("Autopilot · Paused: the agent process exited");
    expect(within(notice()).getByRole("note").textContent)
      .toBe("This tab's agent is not running. Reboot it first; its autopilot comes back paused.");
    expect(within(notice()).getAllByRole("button").map((element) => element.textContent)).toEqual(["Details"]);

    report(7, autopilot("running"));
    ws.readyState = WebSocket.CLOSED;
    act(() => useStore.setState({ connected: false }));
    view(false);
    expect(within(notice()).getByRole("note").textContent).toBe("Not connected to the server.");
    for (const name of ["Pause", "Stop"]) {
      const action = button(notice(), name);
      expect(action.disabled).toBe(true);
      fireEvent.click(action);
    }
    expect(ws.send).not.toHaveBeenCalled();
    expect(useStore.getState().autopilotRequests).toEqual({});
  });
});

describe("the tab's chip", () => {
  it("marks a tab on autopilot with a glyph and says so in its name, leaving other chips as they were", () => {
    seed([tab(7, "OMP 1"), tab(8, "OMP 2"), tab(9, "OMP 3"), tab(10, "OMP 4")], 8, {
      entries: {
        7: { support: "supported", state: autopilot("running") },
        8: { support: "supported" },
        9: { support: "supported", state: autopilot("paused", "failures") },
        10: { support: "supported", state: ended("time_limit") },
      },
      paneStatuses: { [paneKey(7)]: "Working" },
    });
    renderScreen();

    const on = screen.getByRole("button", { name: "OMP 1 · working, on autopilot" });
    const glyph = within(on).getByTestId("chip-autopilot-glyph");
    expect(glyph.getAttribute("data-autopilot")).toBe("on");
    expect(glyph.getAttribute("aria-hidden")).toBe("true");
    // The chip keeps its words; the glyph adds none.
    expect(on.textContent).toBe("OMP 1 · working");

    const paused = screen.getByRole("button", { name: "OMP 3, autopilot paused" });
    expect(within(paused).getByTestId("chip-autopilot-glyph").getAttribute("data-autopilot")).toBe("paused");

    // No autopilot, or an ended one: the chip and its name are unchanged.
    for (const name of ["OMP 2", "OMP 4"]) {
      const plain = screen.getByRole("button", { name });
      expect(plain.hasAttribute("aria-label")).toBe(false);
      expect(within(plain).queryByTestId("chip-autopilot-glyph")).toBeNull();
    }
    expect(screen.getAllByTestId("chip-autopilot-glyph")).toHaveLength(2);
  });

  it("keeps a chip that names a machine within the row", () => {
    // Tabs on two machines: each chip names its machine on a second line.
    seed([tab(6, "OMP on 5", { machine_id: ZOO5, hostname: "zoo-005" }), tab(7, "A long name for an OMP tab")], 6, {
      entries: { 7: { support: "supported", state: autopilot("waiting", "background_work") } },
      instances: [
        { machineId: ZOO5, hostname: "zoo-005", primary: true, runtimeAccess: true },
        { machineId: ZOO6, hostname: "zoo-006", primary: false, runtimeAccess: true },
      ],
    });
    renderScreen();

    const chip = screen.getByRole("button", { name: "A long name for an OMP tab · zoo-006, on autopilot" });
    expect(chip.className.split(" ")).toEqual(expect.arrayContaining(["flex-col", "max-w-full", "shrink-0"]));
    const [first, machineLine] = Array.from(chip.children) as HTMLElement[];
    // The glyph does not shrink; the name gives way beside it.
    expect(first.className.split(" ")).toEqual(expect.arrayContaining(["flex", "max-w-full"]));
    const [glyph, label] = Array.from(first.children) as HTMLElement[];
    expect(glyph.getAttribute("data-testid")).toBe("chip-autopilot-glyph");
    expect(glyph.getAttribute("class")?.split(" ")).toContain("shrink-0");
    expect(label.textContent).toBe("A long name for an OMP tab");
    expect(label.className.split(" ")).toEqual(expect.arrayContaining(["min-w-0", "truncate"]));
    expect(machineLine.textContent).toBe("zoo-006");

    // The other machine's chip is as it was.
    const plain = screen.getByRole("button", { name: "OMP on 5 · zoo-005" });
    expect((plain.children[0] as HTMLElement).className.split(" ")).toEqual(expect.arrayContaining(["max-w-full", "truncate"]));
    expect(within(plain).queryByTestId("chip-autopilot-glyph")).toBeNull();
  });
});

describe("the activity list", () => {
  it("marks where each iteration starts, with the prompt behind the first marker only", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: { 7: { support: "supported", state: autopilot("running") } },
      paneMessages: {
        [paneKey(7)]: [
          iterationMessage("it-1", { autopilot_id: AUTOPILOT, iteration: 1, limit: 20, prompt: PROMPT }, "2026-11-13T14:00:00Z"),
          { id: "reply-1", role: "assistant", content: "Fixed the lint error.", timestamp: new Date("2026-11-13T14:01:00Z"), sessionId: SID, paneId: 7, outputType: { type: "text" } },
          iterationMessage("it-2", { autopilot_id: AUTOPILOT, iteration: 2, limit: 20 }, "2026-11-13T14:02:00Z"),
          iterationMessage("it-3", { autopilot_id: AUTOPILOT, iteration: 3 }, "2026-11-13T14:03:00Z"),
        ],
      },
    });
    renderScreen();

    const log = screen.getByRole("log", { name: "Conversation activity" });
    const markers = within(log).getAllByTestId("autopilot-iteration-marker");
    expect(markers.map((marker) => marker.querySelector("span.font-medium")?.textContent)).toEqual([
      "Autopilot · iteration 1 of 20",
      "Autopilot · iteration 2 of 20",
      "Autopilot · iteration 3",
    ]);
    // The first marker opens to the prompt; later ones show the number only.
    expect(within(markers[0]).getByText("Prompt")).toBeTruthy();
    expect(markers[0].textContent).toContain(PROMPT);
    expect(markers[1].textContent).not.toContain(PROMPT);
    expect(within(markers[1]).queryByText("Prompt")).toBeNull();

    // Not a message from the person, and not a tool call.
    expect(log.querySelectorAll('[data-message-role="user"]')).toHaveLength(0);
    expect(log.textContent).not.toContain("ApasAutopilotIteration");
    expect(log.querySelectorAll("article")).toHaveLength(1);
    expect(log.textContent).toContain("Fixed the lint error.");
  });

  it("leaves a block it cannot read as a tool entry", () => {
    seed([tab(7, "OMP 1")], 7, {
      paneMessages: {
        [paneKey(7)]: [iterationMessage("odd", { autopilot_id: AUTOPILOT }, "2026-11-13T14:00:00Z")],
      },
    });
    renderScreen();
    const log = screen.getByRole("log", { name: "Conversation activity" });
    expect(within(log).queryByTestId("autopilot-iteration-marker")).toBeNull();
    expect(log.textContent).toContain("Using ApasAutopilotIteration");
  });
});

describe("OMP's own modes on a phone", () => {
  it("shows goal and loop mode with how to stop them in the terminal", () => {
    seed([tab(7, "OMP 1")], 7, {
      entries: {
        7: {
          support: "supported",
          state: {
            omp_modes: {
              goal: { status: "active", objective: "Make the suite green", token_budget: 500000, tokens_used: 1200 },
              loop: { status: "paused" },
            },
          },
        },
      },
    });
    renderScreen();

    const goal = screen.getByTestId("omp-mode-goal");
    expect(goal.textContent).toContain("This tab is in OMP's own goal mode and continues by itself.");
    expect(goal.textContent).toContain("Objective: Make the suite green");
    expect(goal.textContent).toContain("Token budget: 500,000");
    expect(goal.textContent).toContain("Tokens used: 1,200");
    expect(goal.textContent).toContain("Esc pauses it, and /goal drop removes the goal.");

    const loop = screen.getByTestId("omp-mode-loop");
    expect(loop.textContent).toContain("This tab is in OMP's own loop mode, paused.");
    expect(loop.textContent).toContain("/loop switches the loop off");
    // The composer below types into the same terminal.
    expect(loop.textContent).toContain("A message sent to this tab now becomes the prompt the loop repeats.");
    expect(loop.compareDocumentPosition(composer()) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

    // Shown, never switched: nothing here acts on the mode.
    expect(within(goal).queryByRole("button")).toBeNull();
    expect(within(loop).queryByRole("button")).toBeNull();
    // It is not an autopilot, and starting one is still the tab's action.
    expect(screen.queryByRole("group", { name: "Autopilot" })).toBeNull();
  });

  it("shows nothing where OMP reports no mode", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: {} } } });
    renderScreen();
    expect(screen.queryByTestId("omp-mode-goal")).toBeNull();
    expect(screen.queryByTestId("omp-mode-loop")).toBeNull();
  });
});

describe("the phone's two confirmations", () => {
  it("says closing a tab on autopilot ends its autopilot with the tab's agent", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("waiting", "next_iteration") } } });
    renderScreen();
    fireEvent.click(button(openActions(), "Close this pane"));
    const sheet = screen.getByRole("dialog", { name: "Close OMP 1" });
    expect(within(sheet).getByRole("note").textContent)
      .toBe("This tab is on autopilot. Closing the tab ends its autopilot together with the tab's agent.");
    // The confirmation still decides: nothing is removed by opening it.
    expect(frames("remove_pane")).toEqual([]);
    fireEvent.click(button(sheet, "Close pane"));
    expect(frames("remove_pane")).toEqual([{ type: "remove_pane", session_id: SID, pane_id: 7 }]);
  });

  it("says a rebooted agent's autopilot comes back paused until someone resumes it", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("running") } } });
    renderScreen();
    fireEvent.click(button(openActions(), "Reboot this pane"));
    const sheet = screen.getByRole("dialog", { name: "Reboot OMP 1" });
    expect(within(sheet).getByRole("note").textContent)
      .toBe("This tab is on autopilot. After the reboot its autopilot comes back paused, and stays paused until someone resumes it.");
    expect(frames("reboot_pane")).toEqual([]);
    fireEvent.click(button(sheet, "Reboot pane"));
    expect(frames("reboot_pane")).toEqual([{ type: "reboot_pane", session_id: SID, pane_id: 7 }]);
  });

  it("says a paused autopilot is discarded with the tab, and stays paused after a reboot", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported", state: autopilot("paused", "failures") } } });
    renderScreen();
    fireEvent.click(button(openActions(), "Close this pane"));
    const close = screen.getByRole("dialog", { name: "Close OMP 1" });
    expect(within(close).getByRole("note").textContent)
      .toBe("This tab has a paused autopilot. Closing the tab discards it.");
    fireEvent.click(button(close, "Cancel"));
    fireEvent.click(button(openActions(), "Reboot this pane"));
    expect(within(screen.getByRole("dialog", { name: "Reboot OMP 1" })).getByRole("note").textContent)
      .toBe("This tab's autopilot stays paused after the reboot, until someone resumes it.");
  });

  it("leaves both confirmations as they were for a tab with no autopilot", () => {
    seed([tab(7, "OMP 1")], 7, { entries: { 7: { support: "supported" } } });
    renderScreen();
    fireEvent.click(button(openActions(), "Close this pane"));
    expect(within(screen.getByRole("dialog", { name: "Close OMP 1" })).queryByRole("note")).toBeNull();
    fireEvent.click(button(screen.getByRole("dialog", { name: "Close OMP 1" }), "Cancel"));
    fireEvent.click(button(openActions(), "Reboot this pane"));
    expect(within(screen.getByRole("dialog", { name: "Reboot OMP 1" })).queryByRole("note")).toBeNull();
  });
});
