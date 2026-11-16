import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage } from "@apas/protocol";
import {
  handleServerMessage,
  paneKey,
  useStore,
  type MachineWithProjects,
  type PaneConfig,
  type SessionInfo,
} from "@/lib/store";
import { PROJECT_INSTANCES_CAPABILITY, type SessionInstance } from "@/lib/projectInstances";
import { PROJECT_OPEN_STATE_CAPABILITY } from "@/lib/projectOpenState";
import { TabbedView } from "./TabbedView";

/**
 * The project view of a project that is open or closed as a whole, driven
 * through the real store: the frames the server sends are delivered to its
 * handler, and every frame the view sends is checked against the protocol
 * schema the server parses.
 */

// xterm.js needs canvas APIs jsdom lacks; these tests are about the view.
vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ paneId, inputDisabledReason }: { paneId: number; inputDisabledReason?: string }) => (
    <div data-testid={`terminal-pane-${paneId}`} data-input-disabled={inputDisabledReason} />
  ),
}));

const SID = "20000000-0000-4000-8000-000000000001";
const CLI = "cli-open-state";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const ZOO7 = "50000000-0000-4000-8000-000000000007";
const NOTHING_DELETED = "Nothing is deleted. Its tabs come back when you open it again.";
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

function machine(
  machineId: string,
  hostname: string,
  overrides: { running?: boolean; connected?: boolean } = {},
): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [{ projectId: SID, path: "/work/apas", isRunning: overrides.running === true }],
    clusterAccess: "owner",
    daemonConnected: overrides.connected !== false,
    projectInstancesAvailable: true,
  };
}

function instance(machineId: string, hostname: string, primary = false): SessionInstance {
  return { machineId, hostname, primary, runtimeAccess: true };
}

const BOTH = [instance(ZOO5, "zoo-005", true), instance(ZOO6, "zoo-006")];

let ws: { readyState: number; send: Mock };

interface SeedOptions {
  /** The machines with an instance attached; each also reports it running. */
  instances?: SessionInstance[];
  /** False: the server did not echo `project_open_state_v1`. */
  negotiated?: boolean;
  activePaneId?: number;
  panes?: PaneConfig[];
  machines?: MachineWithProjects[];
  session?: Partial<SessionInfo>;
}

function seed(options: SeedOptions = {}) {
  const instances = options.instances ?? BOTH;
  const attached = (machineId: string) => instances.some((item) => item.machineId === machineId);
  const panes = options.panes
    ?? [terminal(10, "Claude 1", ZOO5, "zoo-005"), terminal(11, "Claude 2", ZOO6, "zoo-006")];
  const capabilities = [PROJECT_INSTANCES_CAPABILITY, "cli_lifecycle_v1"];
  if (options.negotiated ?? true) capabilities.push(PROJECT_OPEN_STATE_CAPABILITY);
  localStorage.setItem(`apas_layout_${CLI}_active_tab`, String(options.activePaneId ?? 10));
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: instances.length > 0,
      isDualPane: true,
      sessionId: SID,
      cliClientId: CLI,
      token: "test-token",
      negotiatedCapabilities: new Set(capabilities),
      sessions: [{
        id: SID,
        projectId: SID,
        status: "active",
        isActive: instances.length > 0,
        hostname: "zoo-005",
        workingDir: "/work/apas",
        instances,
        ...options.session,
      }],
      sessionInstances: {},
      machines: options.machines ?? [
        machine(ZOO5, "zoo-005", { running: attached(ZOO5) }),
        machine(ZOO6, "zoo-006", { running: attached(ZOO6) }),
      ],
      projectOpenStates: {},
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

type Part = "running" | "starting" | "waiting_offline" | "not_started" | "still_running";

function part(
  machine_id: string,
  hostname: string,
  state: Part,
  overrides: { reason?: string; owns_tabs?: boolean; connected?: boolean } = {},
) {
  return { machine_id, hostname, part: state, owns_tabs: true, connected: true, ...overrides };
}

/** One `project_open_state` frame, as the server sends it. No `state` is a
 *  project nobody has recorded. */
function report(state: "open" | "closed" | undefined, machines: ReturnType<typeof part>[]) {
  act(() => handleServerMessage(
    {
      type: "project_open_state",
      project_id: SID,
      ...(state ? { state, set_by: "owner@example.com", set_kind: "request", set_at: "2026-11-16T10:00:00Z" } : {}),
      machines,
    },
    useStore.setState,
    useStore.getState,
  ));
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

/** The project's Open and Close controls that are offered, in order. */
function projectButtons(): string[] {
  return ["Open", "Keep open", "Open again", "Close", "Close again"]
    .filter((name) => screen.queryByRole("button", { name }) !== null);
}

function notice(): HTMLElement | null {
  return screen.queryByRole("status", { name: "Machines not following this project" });
}

function dialogLines(): (string | null)[] {
  return Array.from(screen.getByRole("dialog").querySelectorAll("p")).map((line) => line.textContent);
}

function confirmDialog(label: string) {
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: label }));
}

async function renderView(paneId = 10) {
  const view = render(<TabbedView />);
  if (useStore.getState().isAttached) await screen.findByTestId(`terminal-pane-${paneId}`);
  return view;
}

let confirmSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
  // Every confirmation here is asked in the page.
  confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
  confirmSpy.mockClear();
});

afterEach(() => {
  expect(confirmSpy).not.toHaveBeenCalled();
  act(() => useStore.setState(initialStore, true));
  localStorage.clear();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("a closed project", () => {
  it("offers Open and not Close, opens the whole project once, and then shows what the server reports", async () => {
    seed({ instances: [], session: { openState: "closed" } });
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    await renderView();

    expect(projectButtons()).toEqual(["Open"]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Closed");
    // A closed project that runs nowhere has no machine out of line.
    expect(notice()).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("dialog", { name: "Open apas?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    expect(sent("start_machine_project_cli")).toEqual([]);

    confirmDialog("Open");
    const [open] = sent("start_machine_project_cli");
    expect(open).toEqual({
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    });
    expectValid(open);
    // Nothing is claimed before the server says so.
    expect(screen.getByTestId("project-open-state").textContent).toBe("Closed");

    report("open", [part(ZOO5, "zoo-005", "starting"), part(ZOO6, "zoo-006", "starting")]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");
    expect(notice()?.textContent).toContain("zoo-005 is starting this project.");
    expect(notice()?.textContent).toContain("zoo-006 is starting this project.");

    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    expect(projectButtons()).toEqual(["Close"]);
    expect(notice()).toBeNull();
    // The request went out once.
    expect(sent("start_machine_project_cli")).toHaveLength(1);
    expect(sent("stop_machine_project_cli")).toEqual([]);
  });

  it("opens nothing when the confirmation is declined", async () => {
    seed({ instances: [], session: { openState: "closed" } });
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    await renderView();
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    confirmDialog("Cancel");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(ws.send).not.toHaveBeenCalled();
  });

  it("says a machine still runs it, offers Close again, and leaves that machine's tabs their real status", async () => {
    seed({ instances: [instance(ZOO6, "zoo-006", true)], activePaneId: 11 });
    const reason = "The account that closed this project may not use zoo-006.";
    report("closed", [
      part(ZOO5, "zoo-005", "not_started"),
      part(ZOO6, "zoo-006", "still_running", { reason }),
    ]);
    const { container } = await renderView(11);

    expect(screen.getByTestId("project-open-state").textContent).toBe("Closed");
    expect(notice()?.textContent).toBe(`zoo-006 still runs this project. ${reason}`);
    expect(projectButtons()).toEqual(["Open", "Close again"]);

    // zoo-006 runs its tab: it takes input and is not shown as stopped.
    expect(screen.getByTestId("terminal-pane-11").hasAttribute("data-input-disabled")).toBe(false);
    const running = container.querySelector('[data-tab-id="11"]') as HTMLElement;
    expect(running.getAttribute("title")).toBeNull();
    expect(within(running).getByTestId("tab-host-badge").textContent).toBe("zoo-006");
    expect((screen.getByRole("button", { name: "Reboot" }) as HTMLButtonElement).disabled).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "Close again" }));
    expect(screen.getByRole("dialog", { name: "Close apas again?" })).toBeTruthy();
    expect(dialogLines()[0]).toBe("zoo-006 still runs this project.");
    expect(dialogLines().at(-1)).toBe(NOTHING_DELETED);
    confirmDialog("Close again");
    const [close] = sent("stop_machine_project_cli");
    expect(close).toEqual({
      type: "stop_machine_project_cli",
      machine_id: ZOO6,
      project_id: SID,
      all_instances: true,
    });
    expectValid(close);
  });
});

describe("an open project", () => {
  it("offers Close and not Open while every machine that owns its tabs runs it", async () => {
    seed({ session: { openState: "open" } });
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    await renderView();
    expect(projectButtons()).toEqual(["Close"]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");
    expect(notice()).toBeNull();
  });

  it("closes the whole project after a confirmation that names the machines and says nothing is deleted", async () => {
    seed({ session: { openState: "open" } });
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    await renderView();
    const tabsBefore = useStore.getState().paneConfigs;

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.getByRole("dialog", { name: "Close apas?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["This stops its agents on zoo-005 and zoo-006.", NOTHING_DELETED]);
    confirmDialog("Close");

    const [close] = sent("stop_machine_project_cli");
    expect(close).toEqual({
      type: "stop_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    });
    expectValid(close);
    // Closing sends one stop and nothing that removes a tab or the project,
    // and it does not open the deletion dialog.
    expect(ws.send).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useStore.getState().paneConfigs).toBe(tabsBefore);

    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Closed");
    expect(projectButtons()).toEqual(["Open"]);
    // Its tabs are still listed.
    expect(document.querySelector('[data-tab-id="10"]')).toBeTruthy();
    expect(document.querySelector('[data-tab-id="11"]')).toBeTruthy();
    expect(ws.send).toHaveBeenCalledTimes(1);
  });

  it("names a machine that is not running it with the server's reason, and offers Open again and Close", async () => {
    seed({ instances: [instance(ZOO5, "zoo-005", true)], session: { openState: "open" } });
    const reason = "zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it.";
    report("open", [
      part(ZOO5, "zoo-005", "running"),
      part(ZOO6, "zoo-006", "not_started", { reason }),
    ]);
    await renderView();

    expect(notice()?.textContent).toBe(`zoo-006 is not running this project. ${reason}`);
    expect(projectButtons()).toEqual(["Open again", "Close"]);
    // No control starts zoo-006 alone, in the notice or anywhere else.
    expect(within(notice() as HTMLElement).queryByRole("button")).toBeNull();
    expect(screen.queryByRole("button", { name: /^Start/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Open again" }));
    expect(screen.getByRole("dialog", { name: "Open apas again?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["zoo-005 already runs it.", "Its tabs start on zoo-006."]);
    confirmDialog("Open again");
    const [open] = sent("start_machine_project_cli");
    expect(open).toEqual({
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    });
    expectValid(open);
  });

  it("says a tab on that machine cannot be used until the project is opened again", async () => {
    seed({ instances: [instance(ZOO5, "zoo-005", true)], activePaneId: 11, session: { openState: "open" } });
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")]);
    await renderView(11);
    const sentence = "zoo-006 is not running this project. Open the project again to start it there.";
    expect(screen.getByTestId("terminal-pane-11").getAttribute("data-input-disabled")).toBe(sentence);
    expect(screen.getAllByRole("status").some((element) => element.textContent?.includes(sentence))).toBe(true);
    expect(screen.queryByRole("button", { name: /^Start on/ })).toBeNull();
  });

  it("says an offline machine's tabs start when it reconnects, and does not offer Open again for it", async () => {
    seed({
      instances: [instance(ZOO5, "zoo-005", true)],
      session: { openState: "open" },
      machines: [machine(ZOO5, "zoo-005", { running: true }), machine(ZOO6, "zoo-006", { connected: false })],
    });
    report("open", [
      part(ZOO5, "zoo-005", "running"),
      part(ZOO6, "zoo-006", "waiting_offline", { connected: false }),
    ]);
    await renderView();
    expect(notice()?.textContent).toBe("zoo-006 is offline. Its tabs start when zoo-006 reconnects.");
    expect(projectButtons()).toEqual(["Close"]);
  });

  it("offers Open again and Close when it runs nowhere", async () => {
    seed({ instances: [], session: { openState: "open" } });
    report("open", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    await renderView();
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");
    expect(projectButtons()).toEqual(["Open again", "Close"]);
    expect(notice()?.textContent).toContain("zoo-005 is not running this project.");
    expect(notice()?.textContent).toContain("zoo-006 is not running this project.");
  });
});

describe("a project nobody has recorded", () => {
  it("offers Keep open while it runs everywhere, and confirming records it and starts nothing by itself", async () => {
    seed();
    report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    await renderView();

    expect(projectButtons()).toEqual(["Keep open", "Close"]);
    // No state is recorded, so none is shown as the server's.
    expect(screen.queryByTestId("project-open-state")).toBeNull();
    expect(notice()).toBeNull();
    const sentence = "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.";
    expect(screen.getByRole("button", { name: "Keep open" }).getAttribute("title")).toBe(sentence);

    fireEvent.click(screen.getByRole("button", { name: "Keep open" }));
    expect(screen.getByRole("dialog", { name: "Keep apas open?" })).toBeTruthy();
    expect(dialogLines()).toEqual(["This records the project as open. Nothing starts now.", sentence]);
    confirmDialog("Keep open");
    // The same request as Open: the whole project, never one machine.
    const [keep] = sent("start_machine_project_cli");
    expect(keep).toEqual({
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    });
    expectValid(keep);
    expect(ws.send).toHaveBeenCalledTimes(1);

    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    expect(projectButtons()).toEqual(["Close"]);
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");
  });

  it("names the machine that is not running it and offers Open and Close", async () => {
    seed({ instances: [instance(ZOO5, "zoo-005", true)] });
    report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")]);
    await renderView();

    expect(notice()?.textContent).toBe("zoo-006 is not running this project.");
    expect(projectButtons()).toEqual(["Open", "Close"]);
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(dialogLines()).toEqual([
      "zoo-005 already runs it.",
      "Its tabs start on zoo-006.",
      "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.",
    ]);
    confirmDialog("Open");
    expect(sent("start_machine_project_cli")).toEqual([{
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    }]);
  });
});

describe("a server that records no open state", () => {
  it("shows a running project from what runs, with Close and never Keep open", async () => {
    seed({ negotiated: false });
    await renderView();
    // A frame this viewer never asked for changes nothing.
    report("closed", [part(ZOO5, "zoo-005", "still_running"), part(ZOO6, "zoo-006", "still_running")]);

    expect(projectButtons()).toEqual(["Close"]);
    expect(screen.queryByTestId("project-open-state")).toBeNull();
    expect(notice()).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(dialogLines()).toEqual(["This stops its agents on zoo-005 and zoo-006.", NOTHING_DELETED]);
    confirmDialog("Close");
    const [close] = sent("stop_machine_project_cli");
    expect(close).toEqual({
      type: "stop_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    });
    expectValid(close);
  });

  it("offers Open for a project that runs nowhere, as one request for the whole project", async () => {
    seed({ negotiated: false, instances: [] });
    await renderView();
    expect(projectButtons()).toEqual(["Open"]);
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    confirmDialog("Open");
    const [open] = sent("start_machine_project_cli");
    expect(open).toEqual({
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    });
    expectValid(open);
  });

  it("names a machine that is not running a half-running project, with nothing that starts it alone", async () => {
    seed({ negotiated: false, instances: [instance(ZOO5, "zoo-005", true)] });
    await renderView();
    expect(notice()?.textContent).toBe("zoo-006 is not running this project.");
    expect(projectButtons()).toEqual(["Open again", "Close"]);
    expect(screen.queryByRole("button", { name: /^Start on/ })).toBeNull();
  });
});

describe("a viewer that has received no project_open_state frame", () => {
  // The server records open states, but no frame for this project has
  // arrived: the polled session and machine lists carry the state and each
  // listed machine's part, and the view is built from them.
  function listed(
    machineId: string,
    hostname: string,
    fields: Partial<MachineWithProjects["projects"][number]>,
  ): MachineWithProjects {
    const entry = machine(machineId, hostname, { running: fields.isRunning === true });
    return { ...entry, projects: [{ ...entry.projects[0], ...fields }] };
  }

  it("shows a closed project from the lists, and Open names the machines whose tabs start", async () => {
    seed({
      instances: [],
      session: { openState: "closed" },
      machines: [
        listed(ZOO5, "zoo-005", { openState: "closed", part: "not_started" }),
        listed(ZOO6, "zoo-006", { openState: "closed", part: "not_started" }),
      ],
    });
    await renderView();
    expect(useStore.getState().projectOpenStates).toEqual({});

    expect(screen.getByTestId("project-open-state").textContent).toBe("Closed");
    expect(projectButtons()).toEqual(["Open"]);
    expect(notice()).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(dialogLines()).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    confirmDialog("Open");
    expect(sent("start_machine_project_cli")).toEqual([{
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: SID,
      all_instances: true,
    }]);
  });

  it("names a machine that did not start an open project, with the reason from the machine list", async () => {
    const reason = "zoo-006 could not start the project: the checkout is missing.";
    seed({
      instances: [instance(ZOO5, "zoo-005", true)],
      session: { openState: "open" },
      machines: [
        listed(ZOO5, "zoo-005", { isRunning: true, openState: "open", part: "running" }),
        listed(ZOO6, "zoo-006", { openState: "open", part: "not_started", partReason: reason }),
      ],
    });
    await renderView();

    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");
    expect(notice()?.textContent).toBe(`zoo-006 is not running this project. ${reason}`);
    expect(projectButtons()).toEqual(["Open again", "Close"]);
  });

  it("offers Keep open for a running project the lists carry no state for", async () => {
    seed({
      machines: [
        listed(ZOO5, "zoo-005", { isRunning: true, part: "running" }),
        listed(ZOO6, "zoo-006", { isRunning: true, part: "running" }),
      ],
    });
    await renderView();
    // Not recorded: no state label, and Open reads as keeping it open.
    expect(screen.queryByTestId("project-open-state")).toBeNull();
    expect(projectButtons()).toEqual(["Keep open", "Close"]);
  });
});

describe("a request that could not be sent", () => {
  it("says nothing was sent, and sends nothing later", async () => {
    seed({ session: { openState: "open" } });
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    await renderView();

    ws.readyState = WebSocket.CLOSED;
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    confirmDialog("Close");
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("Not connected to the server. Nothing was sent.");
    expect(ws.send).not.toHaveBeenCalled();
    // The project is still shown as the server last reported it.
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");

    // The connection coming back does not send the request after all.
    ws.readyState = WebSocket.OPEN;
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    expect(ws.send).not.toHaveBeenCalled();

    fireEvent.click(within(alert).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).toBeNull();

    // Asking again is the person's call, and then it goes out once.
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    confirmDialog("Close");
    expect(sent("stop_machine_project_cli")).toHaveLength(1);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("deleting a project", () => {
  it("is offered to the owner beside Close, as a dialog of its own that names the machines", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    seed({
      instances: [instance(ZOO5, "zoo-005", true)],
      session: { openState: "open", shareRole: "owner" },
      machines: [machine(ZOO5, "zoo-005", { running: true }), machine(ZOO6, "zoo-006", { connected: false })],
    });
    report("open", [
      part(ZOO5, "zoo-005", "running"),
      part(ZOO6, "zoo-006", "waiting_offline", { connected: false }),
    ]);
    await renderView();

    // Closing asks its own question and never opens the deletion dialog.
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.getByRole("dialog", { name: "Close apas?" })).toBeTruthy();
    confirmDialog("Close");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(sent("stop_machine_project_cli")).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Delete project…" }));
    const dialog = screen.getByRole("dialog", { name: "Delete project" });
    expect(within(dialog).getByRole("heading", { name: "Delete apas?" })).toBeTruthy();
    expect(dialog.textContent).toContain("Agents running in the project are stopped first, on zoo-005.");
    expect(dialog.textContent).toContain("zoo-006 is offline and owns tabs of this project");
    // Nothing is deleted until the name is typed and confirmed.
    expect((within(dialog).getByRole("button", { name: "Delete project" }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
    // Only the Close went to the server.
    expect(ws.send).toHaveBeenCalledTimes(1);
  });

  it("is not offered to a user who is not the owner", async () => {
    seed({ session: { openState: "open", isShared: true, shareRole: "user", ownerEmail: "owner@example.com" } });
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    await renderView();
    expect(projectButtons()).toEqual(["Close"]);
    expect(screen.queryByRole("button", { name: /Delete/ })).toBeNull();
  });
});

describe("a new tab on a machine that is not running the project", () => {
  it("says it opens the project and names the other machines whose tabs start", async () => {
    seed({
      instances: [instance(ZOO5, "zoo-005", true)],
      panes: [
        terminal(10, "Claude 1", ZOO5, "zoo-005"),
        terminal(11, "Claude 2", ZOO6, "zoo-006"),
        terminal(12, "Claude 3", ZOO7, "zoo-007"),
      ],
      machines: [
        machine(ZOO5, "zoo-005", { running: true }),
        machine(ZOO6, "zoo-006"),
        machine(ZOO7, "zoo-007"),
      ],
    });
    act(() => useStore.setState({
      projectPolicies: {
        [SID]: {
          teamAvailable: false,
          allowedLaunchProfiles: ["terminal:claude:official:default"],
          version: 3,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
    }));
    report(undefined, [
      part(ZOO5, "zoo-005", "running"),
      part(ZOO6, "zoo-006", "not_started"),
      part(ZOO7, "zoo-007", "not_started"),
    ]);
    await renderView();

    fireEvent.click(screen.getByTitle("New tab"));
    const step = screen.getByRole("group", { name: "Run on" });
    expect(within(step).queryByRole("group", { name: "Starts the project first" })).toBeNull();
    const opens = within(step).getByRole("group", { name: "Opens the project" });
    expect(within(opens).getByText(
      "Registered · opens the project, which also starts its tabs on zoo-007, then the tab opens",
    )).toBeTruthy();
    expect(within(opens).getByText(
      "Registered · opens the project, which also starts its tabs on zoo-006, then the tab opens",
    )).toBeTruthy();

    fireEvent.click(within(opens).getByRole("radio", { name: /^zoo-006/ }));
    fireEvent.click(screen.getByText("Claude"));
    // One tab request naming the chosen machine; the server opens the
    // project. The view sends no start of its own.
    const [add] = sent("add_pane");
    expect(add).toMatchObject({ type: "add_pane", session_id: SID, machine_id: ZOO6 });
    expectValid(add);
    expect(sent("start_machine_project_cli")).toEqual([]);
  });

  it("keeps the older wording against a server that records no open state", async () => {
    seed({ negotiated: false, instances: [instance(ZOO5, "zoo-005", true)] });
    await renderView();
    fireEvent.click(screen.getByTitle("New tab"));
    const step = screen.getByRole("group", { name: "Run on" });
    expect(within(step).getByRole("group", { name: "Starts the project first" })).toBeTruthy();
    expect(within(step).queryByRole("group", { name: "Opens the project" })).toBeNull();
  });
});
