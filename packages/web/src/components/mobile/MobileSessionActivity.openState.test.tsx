import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage } from "@apas/protocol";
import { handleServerMessage, useStore, type MachineWithProjects, type PaneConfig } from "@/lib/store";
import { writeSelectedPane } from "@/lib/mobileSelectedPane";
import { PROJECT_INSTANCES_CAPABILITY, type SessionInstance } from "@/lib/projectInstances";
import { PROJECT_OPEN_STATE_CAPABILITY } from "@/lib/projectOpenState";
import { MobileSessionActivity } from "./MobileSessionActivity";

/**
 * The phone's session screen of a project that is open or closed as a whole,
 * against a server that records the state. Driven through the real store:
 * the server's frames go to its handler, and every frame the screen sends is
 * checked against the protocol schema. The phone offers opening only: no
 * Close, no Delete and no control for one machine alone.
 */

vi.mock("@/components/tabs/TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => <div>Raw terminal pane {paneId}</div>,
}));

const SID = "20000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";
const KEEP_OPEN = "Its tabs start again on zoo-005 and zoo-006 after those machines have been away.";
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

function machine(machineId: string, hostname: string, running: boolean, connected = true): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [{ projectId: SID, path: "/work/mako", isRunning: running }],
    clusterAccess: "owner",
    daemonConnected: connected,
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
  selectedPaneId?: number;
  /** Machines whose daemon is not connected. */
  offline?: string[];
}

function seed(options: SeedOptions = {}) {
  const instances = options.instances ?? BOTH;
  const attached = (machineId: string) => instances.some((item) => item.machineId === machineId);
  const connected = (machineId: string) => !(options.offline ?? []).includes(machineId);
  writeSelectedPane(SID, options.selectedPaneId ?? 10);
  ws = { readyState: WebSocket.OPEN, send: vi.fn() };
  act(() => {
    useStore.setState({
      ws: ws as unknown as WebSocket,
      connected: true,
      isAttached: instances.length > 0,
      sessionId: SID,
      negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY, PROJECT_OPEN_STATE_CAPABILITY]),
      sessions: [{
        id: SID,
        projectId: SID,
        status: instances.length > 0 ? "active" : "offline",
        isActive: instances.length > 0,
        hostname: "zoo-005",
        workingDir: "/work/mako",
        instances,
      }],
      sessionInstances: {},
      machines: [
        machine(ZOO5, "zoo-005", attached(ZOO5), connected(ZOO5)),
        machine(ZOO6, "zoo-006", attached(ZOO6), connected(ZOO6)),
      ],
      projectOpenStates: {},
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

function renderScreen() {
  render(<MobileSessionActivity connected onBack={vi.fn()} onReconnect={vi.fn()} />);
}

function openActions(): HTMLElement {
  fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  return screen.getByRole("dialog", { name: "Pane actions" });
}

function lines(dialog: HTMLElement): (string | null)[] {
  return Array.from(dialog.querySelectorAll("p")).map((line) => line.textContent);
}

function stateChip(): string | null {
  return screen.getByTestId("project-open-state").textContent;
}

/** No control of the phone closes or deletes a project, or starts or stops
 *  one machine alone. "Close this pane" and "Close actions" are others. */
function expectOpenOnly(sheet: HTMLElement) {
  const names = within(sheet).getAllByRole("button").map((button) =>
    button.getAttribute("aria-label") ?? button.textContent ?? "");
  expect(names.filter((name) => /close project|^close( again)?$|delete|^start on|^stop /i.test(name.trim()))).toEqual([]);
}

const OPEN_FRAME = { type: "start_machine_project_cli", machine_id: ZOO5, project_id: SID, all_instances: true };

beforeEach(() => {
  localStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  act(() => useStore.setState(initialStore, true));
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("a closed project on the phone", () => {
  it("offers Open project, names the machines whose tabs start, and sends one request for the whole project", () => {
    seed({ instances: [] });
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    renderScreen();
    expect(stateChip()).toBe("Closed");

    const sheet = openActions();
    expectOpenOnly(sheet);
    // A closed project's machines have not failed at anything: no notice.
    expect(within(sheet).queryByRole("status", { name: "Machines not following this project" })).toBeNull();
    fireEvent.click(within(sheet).getByRole("button", { name: "Open project" }));
    expect(sent("start_machine_project_cli")).toEqual([]);

    const confirm = screen.getByRole("dialog", { name: "Open mako?" });
    expect(lines(confirm)).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    fireEvent.click(within(confirm).getByRole("button", { name: "Open project" }));

    const starts = sent("start_machine_project_cli");
    expect(starts).toEqual([OPEN_FRAME]);
    expectValid(starts[0]);
    expect(screen.queryByRole("dialog", { name: "Open mako?" })).toBeNull();

    // What happened is what the server reports afterwards. Nothing is sent
    // again while it does, or after.
    report("open", [part(ZOO5, "zoo-005", "starting"), part(ZOO6, "zoo-006", "starting")]);
    expect(stateChip()).toBe("Open");
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    expect(sent("start_machine_project_cli")).toHaveLength(1);
  });

  it("says of an offline tab owner that its tabs start when it reconnects", () => {
    seed({ instances: [], offline: [ZOO6] });
    report("closed", [
      part(ZOO5, "zoo-005", "not_started"),
      part(ZOO6, "zoo-006", "not_started", { connected: false }),
    ]);
    renderScreen();
    fireEvent.click(within(openActions()).getByRole("button", { name: "Open project" }));
    expect(lines(screen.getByRole("dialog", { name: "Open mako?" }))).toEqual([
      "Its tabs start on zoo-005.",
      "zoo-006 is offline. Its tabs start when zoo-006 reconnects.",
    ]);
  });

  it("sends nothing when the confirmation is cancelled", () => {
    seed({ instances: [] });
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    renderScreen();
    fireEvent.click(within(openActions()).getByRole("button", { name: "Open project" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "Open mako?" })).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Open mako?" })).toBeNull();
    expect(sent("start_machine_project_cli")).toEqual([]);
  });

  it("says nothing was sent when the socket is closed, and sends nothing later", () => {
    seed({ instances: [] });
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    renderScreen();
    fireEvent.click(within(openActions()).getByRole("button", { name: "Open project" }));
    ws.readyState = WebSocket.CLOSED;
    fireEvent.click(within(screen.getByRole("dialog", { name: "Open mako?" })).getByRole("button", { name: "Open project" }));
    expect(screen.getByText("Not connected to the server. Nothing was sent.")).toBeTruthy();

    ws.readyState = WebSocket.OPEN;
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    expect(sent("start_machine_project_cli")).toEqual([]);
  });

  it("keeps the real state of tabs on a machine that still runs it, and offers no Close", () => {
    // The account that closed it may not use zoo-006, which still runs it.
    seed({ instances: [instance(ZOO6, "zoo-006", true)], selectedPaneId: 11 });
    report("closed", [
      part(ZOO5, "zoo-005", "not_started"),
      part(ZOO6, "zoo-006", "still_running", { reason: "The account that closed this project cannot use zoo-006." }),
    ]);
    renderScreen();
    expect(stateChip()).toBe("Closed");
    // zoo-006's tab is as it is, not "(stopped)".
    expect(screen.getByRole("button", { name: "Claude 2 · zoo-006" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Claude 1 · zoo-005 (stopped)" })).toBeTruthy();

    const sheet = openActions();
    expectOpenOnly(sheet);
    const notice = within(sheet).getByRole("status", { name: "Machines not following this project" });
    expect(notice.textContent)
      .toBe("zoo-006 still runs this project. The account that closed this project cannot use zoo-006.");
    // Opening stays the way out of a closed project.
    expect(within(sheet).getByRole("button", { name: "Open project" })).toBeTruthy();
    expect(sent("stop_machine_project_cli")).toEqual([]);
  });
});

describe("an open project on the phone", () => {
  it("offers nothing to open while every machine that owns its tabs runs it", () => {
    seed();
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    renderScreen();
    expect(stateChip()).toBe("Open");
    const sheet = openActions();
    expectOpenOnly(sheet);
    expect(within(sheet).queryByRole("button", { name: /open project|keep project open/i })).toBeNull();
    expect(within(sheet).queryByRole("status", { name: "Machines not following this project" })).toBeNull();
  });

  it("offers to open the project again while a connected machine is not following, with the server's reason", () => {
    seed({ instances: [instance(ZOO5, "zoo-005", true)], selectedPaneId: 11 });
    report("open", [
      part(ZOO5, "zoo-005", "running"),
      part(ZOO6, "zoo-006", "not_started", { reason: "zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it." }),
    ]);
    renderScreen();
    expect(stateChip()).toBe("Open");

    // The selected tab is zoo-006's: its banner gives the refusal and why.
    const banner = screen.getAllByRole("status").find((element) =>
      element.textContent?.includes("zoo-006 is not running this project. Open the project again to start it there.")) as HTMLElement;
    expect(banner.textContent).toContain("Reboot zoo-006 to update it.");
    expect(within(banner).queryByRole("button", { name: /^Start on/ })).toBeNull();

    const sheet = openActions();
    expectOpenOnly(sheet);
    expect(within(sheet).getByRole("status", { name: "Machines not following this project" }).textContent)
      .toBe("zoo-006 is not running this project. zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it.");
    fireEvent.click(within(sheet).getByRole("button", { name: "Open project again" }));

    const confirm = screen.getByRole("dialog", { name: "Open mako again?" });
    expect(lines(confirm)).toEqual(["zoo-005 already runs it.", "Its tabs start on zoo-006."]);
    fireEvent.click(within(confirm).getByRole("button", { name: "Open project again" }));
    const starts = sent("start_machine_project_cli");
    // The whole project again; never zoo-006 alone.
    expect(starts).toEqual([OPEN_FRAME]);
    expectValid(starts[0]);
  });

  it("does not offer to open again for a machine that is offline, and says its tabs start when it reconnects", () => {
    seed({ instances: [instance(ZOO5, "zoo-005", true)], offline: [ZOO6], selectedPaneId: 11 });
    report("open", [
      part(ZOO5, "zoo-005", "running"),
      part(ZOO6, "zoo-006", "waiting_offline", { connected: false }),
    ]);
    renderScreen();
    expect(screen.getAllByRole("status").some((element) =>
      element.textContent?.includes("zoo-006 is not running this project because it is offline."))).toBe(true);

    const sheet = openActions();
    expect(within(sheet).queryByRole("button", { name: /open project|keep project open/i })).toBeNull();
    expect(within(sheet).getByRole("status", { name: "Machines not following this project" }).textContent)
      .toBe("zoo-006 is offline. Its tabs start when zoo-006 reconnects.");
    expect(sent("start_machine_project_cli")).toEqual([]);
  });

  it("offers to open again an open project that runs nowhere while its machines are connected", () => {
    seed({ instances: [] });
    report("open", [
      part(ZOO5, "zoo-005", "not_started", { reason: "The daemon did not answer the start." }),
      part(ZOO6, "zoo-006", "not_started"),
    ]);
    renderScreen();
    expect(stateChip()).toBe("Open");
    const sheet = openActions();
    fireEvent.click(within(sheet).getByRole("button", { name: "Open project again" }));
    expect(lines(screen.getByRole("dialog", { name: "Open mako again?" }))).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
  });

  it("shows the Open disabled, with why, while an open project's machines are all away", () => {
    seed({ instances: [], offline: [ZOO5, ZOO6] });
    report("open", [
      part(ZOO5, "zoo-005", "waiting_offline", { connected: false }),
      part(ZOO6, "zoo-006", "waiting_offline", { connected: false }),
    ]);
    renderScreen();
    expect(stateChip()).toBe("Open");
    const open = within(openActions()).getByRole("button", { name: "Open project" }) as HTMLButtonElement;
    expect(open.disabled).toBe(true);
    expect(open.title).toBe("No connected machine of this project can be started now");
    fireEvent.click(open);
    expect(screen.queryByRole("dialog", { name: /^Open mako/ })).toBeNull();
  });
});

describe("a project nobody has recorded, on the phone", () => {
  it("offers to keep a running one open, saying what that changes, and starts nothing by it", () => {
    seed();
    report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    renderScreen();
    // Shown from what runs.
    expect(stateChip()).toBe("Open");

    const sheet = openActions();
    expectOpenOnly(sheet);
    const keep = within(sheet).getByRole("button", { name: "Keep project open" });
    expect(keep.getAttribute("title")).toBe(KEEP_OPEN);
    fireEvent.click(keep);
    const confirm = screen.getByRole("dialog", { name: "Keep mako open?" });
    expect(lines(confirm)).toEqual(["This records the project as open. Nothing starts now.", KEEP_OPEN]);
    fireEvent.click(within(confirm).getByRole("button", { name: "Keep project open" }));
    expect(sent("start_machine_project_cli")).toEqual([OPEN_FRAME]);

    // Recorded: nothing is left to offer.
    report("open", [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "running")]);
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    expect(screen.queryByRole("button", { name: /open project|keep project open/i })).toBeNull();
  });

  it("offers Open project for one that runs on one of its two machines", () => {
    seed({ instances: [instance(ZOO5, "zoo-005", true)] });
    report(undefined, [part(ZOO5, "zoo-005", "running"), part(ZOO6, "zoo-006", "not_started")]);
    renderScreen();
    const sheet = openActions();
    expect(within(sheet).getByRole("status", { name: "Machines not following this project" }).textContent)
      .toBe("zoo-006 is not running this project.");
    fireEvent.click(within(sheet).getByRole("button", { name: "Open project" }));
    expect(lines(screen.getByRole("dialog", { name: "Open mako?" })))
      .toEqual(["zoo-005 already runs it.", "Its tabs start on zoo-006.", KEEP_OPEN]);
  });
});

describe("a new agent on a machine that is not running the project", () => {
  it("says choosing it opens the project and names the other machine whose tabs start", () => {
    seed({ instances: [] });
    report("closed", [part(ZOO5, "zoo-005", "not_started"), part(ZOO6, "zoo-006", "not_started")]);
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    const sheet = screen.getByRole("dialog", { name: "Add agent" });
    const step = within(sheet).getByRole("group", { name: "Run on" });
    const opens = within(step).getByRole("group", { name: "Opens the project" });
    expect(within(step).queryByRole("group", { name: "Starts the project first" })).toBeNull();
    expect(opens.textContent).toContain("opens the project, which also starts its tabs on zoo-006, then the tab opens");
  });
});
