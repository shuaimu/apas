import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage } from "@apas/protocol";
import {
  restoreAddPaneRequests,
  selectCliLifecycleInventory,
  useStore,
  type EffectiveProjectPolicy,
  type MachineWithProjects,
  type SessionInfo,
} from "./store";
import {
  LEGACY_MACHINE_KEY,
  PROJECT_INSTANCES_CAPABILITY,
  selectSessionInstances,
  type SessionInstance,
} from "./projectInstances";
import { ompQuestionKey } from "./ompQuestions";
import { refusedDeliveries, unconfirmedDeliveries } from "./terminalDelivery";

/**
 * One project, several machines. These drive the real socket handler with the
 * protocol's golden frames, and check every frame the store sends against the
 * protocol schema the server parses.
 */

const SID = "20000000-0000-4000-8000-000000000001";
const PROJECT = "30000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000001";
const ZOO6 = "50000000-0000-4000-8000-000000000002";
const LAPTOP = "50000000-0000-4000-8000-000000000003";

type MockWs = {
  send: Mock;
  onmessage?: (event: MessageEvent) => void;
  onclose?: (event: CloseEvent) => void;
};

/** Where a browser tab keeps its launches across a reload. */
const ADD_PANE_REQUESTS_KEY = "apas_add_pane_requests";

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8"));
}

/** Open a fresh socket through the global WebSocket mock. */
async function openSocket(): Promise<MockWs> {
  localStorage.setItem("apas_token", "test-token");
  useStore.setState({ ws: null });
  useStore.getState().connect();
  await new Promise((done) => setTimeout(done, 10));
  return useStore.getState().ws as unknown as MockWs;
}

function deliver(ws: MockWs, message: Record<string, unknown>) {
  ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify(message) }));
}

function sentFrames(ws: MockWs, type?: string): Record<string, unknown>[] {
  return ws.send.mock.calls
    .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
    .filter((frame) => !type || frame.type === type);
}

function closeSocket(ws: MockWs) {
  ws.onclose?.(new CloseEvent("close", { code: 1000 }));
}

function expectValidFrame(frame: Record<string, unknown>) {
  const result = validateClientMessage(frame);
  expect(result.errors).toEqual([]);
  expect(result.valid).toBe(true);
}

function negotiate(capable = true) {
  useStore.setState({
    negotiatedCapabilities: new Set(capable ? [PROJECT_INSTANCES_CAPABILITY] : []),
  });
}

const policy: EffectiveProjectPolicy = {
  teamAvailable: false,
  allowedLaunchProfiles: ["terminal:claude:official:default", "terminal:codex:official:default"],
  version: 3,
  projectSuspended: false,
  noncompliantPaneIds: [],
};

function owned(machineId: string, hostname: string, flags: Partial<MachineWithProjects> & { registered?: boolean } = {}): MachineWithProjects {
  const { registered = false, ...rest } = flags;
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: registered ? [{ projectId: PROJECT, path: "/workspace/project", isRunning: false }] : [],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
    ...rest,
  };
}

function instance(machineId: string | undefined, hostname: string, overrides: Partial<SessionInstance> = {}): SessionInstance {
  return { machineId, hostname, primary: false, ...overrides };
}

function project(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: SID,
    projectId: PROJECT,
    status: "active",
    gitRemote: "github.com/example/project",
    ...overrides,
  };
}

function addTerminal(machineId?: string) {
  return useStore.getState().addPane(
    "claude", "interactive", "Claude 3", undefined, undefined, undefined, undefined, false, "terminal", machineId,
  );
}

function launch(requestId: string | undefined) {
  return requestId ? useStore.getState().addPaneRequests[requestId] : undefined;
}

let showToast: Mock;
let listMachines: Mock;

beforeEach(() => {
  showToast = vi.fn();
  listMachines = vi.fn();
  localStorage.removeItem("apas_session_id");
  useStore.setState({
    sessionId: SID,
    isAttached: true,
    sessions: [project()],
    sessionInstances: {},
    addPaneRequests: {},
    paneConfigs: [],
    paneListReceived: false,
    projectPolicies: { [SID]: policy },
    machines: [],
    pendingInstances: {},
    cliLifecycleInventories: {},
    cliLifecycleOperations: {},
    cliLifecycleLatestBySession: {},
    negotiatedCapabilities: new Set(),
    showToast: showToast as never,
    listMachines: listMachines as never,
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("negotiating instances", () => {
  it("advertises project instances when authenticating", async () => {
    const ws = await openSocket();
    const [authenticate] = sentFrames(ws, "authenticate");
    expect(authenticate.capabilities).toContain(PROJECT_INSTANCES_CAPABILITY);
    expectValidFrame(authenticate);
  });

  it("changes nothing for a refused input's acknowledgement it is not tracking", async () => {
    // A negotiated viewer gets a mutation_ack for typed input the server
    // refused, beside the Error toast the web already shows. It settles only
    // a send this page is tracking under that id (see "refused input"), and
    // its request id is the input's client_msg_id, so it must not settle
    // anything else, even an OMP answer that happens to share the id.
    const refusal = fixture("server-mutation-ack-input.json");
    const omp = {
      sessionId: refusal.session_id as string,
      paneId: refusal.pane_id as number,
      toolUseId: "toolu-omp-4",
      dialogId: "dialog-4",
      requestId: refusal.request_id as string,
      phase: "submitting" as const,
    };
    useStore.setState({ ompQuestions: new Map([[ompQuestionKey(omp.sessionId, omp.paneId, omp.toolUseId), omp]]) });
    const ws = await openSocket();
    negotiate();
    const before = useStore.getState();
    deliver(ws, refusal);
    const after = useStore.getState();
    expect(after.ompQuestions).toBe(before.ompQuestions);
    expect(after.addPaneRequests).toBe(before.addPaneRequests);
    expect(after.pendingSends).toBe(before.pendingSends);
    expect(showToast).not.toHaveBeenCalled();
  });
});

describe("session instances", () => {
  it("applies each pushed set to the session it names, alongside the polled list", async () => {
    const ws = await openSocket();
    deliver(ws, fixture("server-session-instances.json"));
    const pushed = selectSessionInstances(useStore.getState(), SID);
    expect(pushed.map((item) => [item.hostname, item.primary, item.runtimeAccess]))
      .toEqual([["zoo-005", true, true], ["zoo-006", false, false]]);
    expect(useStore.getState().sessions[0].instances).toBe(pushed);

    // The list is polled; whichever of it and a push arrived last describes
    // the session.
    deliver(ws, {
      type: "sessions",
      sessions: [{
        id: SID, project_id: PROJECT, status: "active", is_active: true,
        instances: [{ machine_id: ZOO6, hostname: "zoo-006", primary: true }],
      }],
    });
    expect(selectSessionInstances(useStore.getState(), SID).map((item) => item.hostname)).toEqual(["zoo-006"]);

    deliver(ws, fixture("server-session-instances-legacy.json"));
    expect(selectSessionInstances(useStore.getState(), SID))
      .toEqual([{ machineId: undefined, hostname: "laptop", workingDir: "/workspace/project", primary: true, runtimeAccess: undefined }]);

    // A list without instances (an older server, or nothing attached) leaves
    // the session's recorded host as the description.
    deliver(ws, { type: "sessions", sessions: [{ id: SID, project_id: PROJECT, status: "offline", hostname: "zoo-005" }] });
    expect(selectSessionInstances(useStore.getState(), SID)).toEqual([]);
  });

  it("keeps a pushed set for a session the list does not hold yet, until the socket closes", async () => {
    const other = "20000000-0000-4000-8000-000000000009";
    const ws = await openSocket();
    deliver(ws, { type: "session_instances", session_id: other, instances: [{ machine_id: ZOO5, hostname: "zoo-005", primary: true }] });
    expect(selectSessionInstances(useStore.getState(), other).map((item) => item.machineId)).toEqual([ZOO5]);
    closeSocket(ws);
    expect(selectSessionInstances(useStore.getState(), other)).toEqual([]);
  });

  it("keeps machine tags on panes from session lists and pane lists", async () => {
    const ws = await openSocket();
    deliver(ws, fixture("server-sessions-instances.json"));
    const listed = useStore.getState().sessions[0];
    expect(listed.panes?.map((pane) => [pane.pane_id, pane.machine_id, pane.hostname]))
      .toEqual([[3, ZOO5, "zoo-005"], [4, ZOO6, "zoo-006"]]);
    expect(listed.instances?.map((item) => item.machineId)).toEqual([ZOO5, ZOO6]);

    deliver(ws, fixture("server-pane-list-instances.json"));
    expect(useStore.getState().paneConfigs.map((pane) => [pane.pane_id, pane.machine_id, pane.hostname]))
      .toEqual([[3, ZOO5, "zoo-005"], [4, ZOO6, "zoo-006"]]);
  });

  it("forgets a project's instances and launches with the project", () => {
    useStore.setState({
      sessionInstances: { [SID]: [instance(ZOO5, "zoo-005", { primary: true })] },
      addPaneRequests: {
        launch: {
          requestId: "launch", sessionId: SID, status: "failed", provider: "claude", kind: "terminal",
          startedAt: 1, updatedAt: 1,
        },
      },
    });
    useStore.getState().forgetProject(PROJECT);
    expect(useStore.getState().sessionInstances).toEqual({});
    expect(useStore.getState().addPaneRequests).toEqual({});
  });
});

describe("machines", () => {
  it("reads each project's remote and whether a machine can run another instance", async () => {
    const ws = await openSocket();
    deliver(ws, fixture("server-machines-instances.json"));
    expect(useStore.getState().machines[0]).toMatchObject({
      daemonConnected: true,
      projectInstancesAvailable: true,
      projects: [{ projectId: PROJECT, gitRemote: "github.com/example/project" }],
    });
    deliver(ws, {
      type: "machines",
      machines: [{ machine: { machine_id: ZOO5, hostname: "zoo-005", os: "linux", arch: "x64" }, projects: [{ project_id: PROJECT, path: "/p", is_running: true }] }],
    });
    expect(useStore.getState().machines[0]).toMatchObject({ daemonConnected: false, projectInstancesAvailable: false });
    expect(useStore.getState().machines[0].projects[0].gitRemote).toBeUndefined();
  });
});

describe("project-wide start and stop", () => {
  it("fans out only when the project view asks", async () => {
    const ws = await openSocket();
    useStore.getState().startMachineProjectCli(ZOO5, PROJECT);
    useStore.getState().stopMachineProjectCli(ZOO5, PROJECT);
    useStore.getState().startMachineProjectCli(ZOO5, PROJECT, { allInstances: true });
    useStore.getState().stopMachineProjectCli(ZOO5, PROJECT, { allInstances: true });
    const [start, stop, startAll, stopAll] = sentFrames(ws).filter((frame) =>
      frame.type === "start_machine_project_cli" || frame.type === "stop_machine_project_cli");
    // The Machines page's per-machine controls must never start or stop
    // another machine.
    expect(start).toEqual({ type: "start_machine_project_cli", machine_id: ZOO5, project_id: PROJECT });
    expect(stop).toEqual({ type: "stop_machine_project_cli", machine_id: ZOO5, project_id: PROJECT });
    expect(startAll).toEqual(fixture("web-start-machine-project-cli-all.json"));
    expect(stopAll).toEqual(fixture("web-stop-machine-project-cli-all.json"));
    for (const frame of [start, stop, startAll, stopAll]) expectValidFrame(frame);
  });
});

describe("forgetting a machine's tabs", () => {
  it("sends the request and leaves the roster to the server", async () => {
    const ws = await openSocket();
    useStore.setState({ paneConfigs: [{ pane_id: 4, provider: "codex", mode: "interactive", session_id: "p4", is_paused: false, machine_id: ZOO6 }] });
    expect(useStore.getState().forgetMachinePanes(SID, ZOO6)).toBe(true);
    const [frame] = sentFrames(ws, "forget_machine_panes");
    expect(frame).toEqual(fixture("web-forget-machine-panes.json"));
    expectValidFrame(frame);
    expect(useStore.getState().paneConfigs).toHaveLength(1);
  });

  it("reports that nothing was sent while disconnected", () => {
    useStore.setState({ ws: null });
    expect(useStore.getState().forgetMachinePanes(SID, ZOO6)).toBe(false);
    expect(showToast).toHaveBeenCalledWith(expect.stringContaining("Not connected"), "error");
  });
});

describe("correlated new tabs", () => {
  it("stays fire-and-forget against a server without instances, and refuses a machine", async () => {
    const ws = await openSocket();
    expect(addTerminal()).toEqual({ success: true });
    const [frame] = sentFrames(ws, "add_pane");
    expect(frame).not.toHaveProperty("request_id");
    expect(frame).not.toHaveProperty("machine_id");
    expect(useStore.getState().addPaneRequests).toEqual({});

    // An older server would ignore the machine and run the tab on the
    // primary, a machine the user did not pick.
    const targeted = addTerminal(ZOO6);
    expect(targeted.success).toBe(false);
    expect(targeted.error).toMatch(/newer APAS server/);
    expect(sentFrames(ws, "add_pane")).toHaveLength(1);
  });

  it("tracks a tab on the primary until its instance lists it", async () => {
    const ws = await openSocket();
    negotiate();
    const result = addTerminal();
    expect(result.success).toBe(true);
    const [frame] = sentFrames(ws, "add_pane");
    expectValidFrame(frame);
    expect(frame).toMatchObject({ session_id: SID, kind: "terminal", request_id: result.requestId });
    expect(frame).not.toHaveProperty("machine_id");
    expect(launch(result.requestId)).toMatchObject({ status: "pending", sessionId: SID, label: "Claude 3", kind: "terminal" });

    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: result.requestId, status: "routed", machine_id: ZOO5, pane_id: 9 });
    expect(launch(result.requestId)).toMatchObject({ status: "routed", paneId: 9, machineId: ZOO5 });

    deliver(ws, {
      type: "pane_list",
      session_id: SID,
      panes: [{ pane_id: 9, provider: "claude", mode: "interactive", kind: "terminal", session_id: "p9", machine_id: ZOO5, hostname: "zoo-005" }],
    });
    expect(useStore.getState().addPaneRequests).toEqual({});
  });

  it("drops a routed launch at once when its tab is already listed", async () => {
    const ws = await openSocket();
    negotiate();
    const result = addTerminal();
    useStore.setState({ paneConfigs: [{ pane_id: 9, provider: "claude", mode: "interactive", session_id: "p9", is_paused: false }] });
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: result.requestId, status: "routed", pane_id: 9 });
    expect(useStore.getState().addPaneRequests).toEqual({});
  });

  it("starts a registered machine's instance first, even when nothing runs the project", async () => {
    useStore.setState({ isAttached: false, machines: [owned(ZOO6, "zoo-006", { registered: true })] });
    const ws = await openSocket();
    negotiate();
    // A tab for the primary still needs the project running.
    expect(addTerminal().error).toMatch(/not running/);

    const result = addTerminal(ZOO6);
    expect(result.success).toBe(true);
    const [frame] = sentFrames(ws, "add_pane");
    expectValidFrame(frame);
    expect(frame).toMatchObject({ machine_id: ZOO6, request_id: result.requestId });
    expect(launch(result.requestId)).toMatchObject({ status: "pending", machineId: ZOO6, hostname: "zoo-006" });

    deliver(ws, { ...fixture("server-add-pane-starting.json"), request_id: result.requestId });
    expect(launch(result.requestId)?.status).toBe("starting");
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: result.requestId, status: "routed", machine_id: ZOO6, pane_id: 12 });
    expect(launch(result.requestId)).toMatchObject({ status: "routed", paneId: 12 });
  });

  it("reports the server's refusal and keeps it until dismissed", async () => {
    useStore.setState({ machines: [owned(ZOO6, "zoo-006", { registered: true })] });
    const ws = await openSocket();
    negotiate();
    const result = addTerminal(ZOO6);
    const failed = fixture("server-add-pane-failed.json");
    // Another session's answer with the same id is not this launch's.
    deliver(ws, { ...failed, session_id: "20000000-0000-4000-8000-000000000009", request_id: result.requestId });
    expect(launch(result.requestId)?.status).toBe("pending");

    deliver(ws, { ...failed, request_id: result.requestId });
    expect(launch(result.requestId)).toMatchObject({ status: "failed", error: failed.error });
    expect(showToast).toHaveBeenCalledWith(failed.error, "error");

    // A late duplicate cannot reopen it.
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: result.requestId, status: "routed", pane_id: 3 });
    expect(launch(result.requestId)?.status).toBe("failed");
    useStore.getState().dismissAddPaneRequest(result.requestId!);
    expect(useStore.getState().addPaneRequests).toEqual({});
  });

  it("will not offer a machine this viewer may not use", async () => {
    useStore.setState({ sessions: [project({ instances: [instance(ZOO6, "zoo-006", { primary: true, runtimeAccess: false })] })] });
    const ws = await openSocket();
    negotiate();
    expect(addTerminal(ZOO6).error).toBe("Access denied: you cannot use zoo-006, which runs this project instance");
    expect(sentFrames(ws, "add_pane")).toHaveLength(0);
  });

  it("reports launches as unconfirmed when the connection drops, and never resends them", async () => {
    useStore.setState({ machines: [owned(ZOO6, "zoo-006", { registered: true })] });
    const ws = await openSocket();
    negotiate();
    const pending = addTerminal();
    const starting = addTerminal(ZOO6);
    deliver(ws, { ...fixture("server-add-pane-starting.json"), request_id: starting.requestId });
    closeSocket(ws);

    expect(launch(pending.requestId)).toMatchObject({ status: "unconfirmed", error: expect.stringMatching(/not resent/) });
    expect(launch(starting.requestId)?.status).toBe("unconfirmed");
    expect(showToast).toHaveBeenCalledTimes(1);

    const next = await openSocket();
    negotiate();
    expect(sentFrames(next, "add_pane")).toHaveLength(0);
    // A late answer cannot revive a launch already reported unconfirmed.
    deliver(next, { type: "add_pane_result", session_id: SID, request_id: pending.requestId, status: "routed", pane_id: 9 });
    expect(launch(pending.requestId)?.status).toBe("unconfirmed");
    // Final launches are dismissable; in-flight ones are not.
    useStore.setState({ isAttached: true });
    const inFlight = addTerminal();
    expect(inFlight.success).toBe(true);
    useStore.getState().dismissAddPaneRequest(inFlight.requestId!);
    expect(launch(inFlight.requestId)?.status).toBe("pending");
    useStore.getState().dismissAddPaneRequest(pending.requestId!);
    expect(launch(pending.requestId)).toBeUndefined();
  });

  it("does not let a replaced socket's late close touch its successor's launches", async () => {
    const first = await openSocket();
    negotiate();
    const second = await openSocket();
    const result = addTerminal();
    expect(sentFrames(second, "add_pane")).toHaveLength(1);
    closeSocket(first);
    expect(launch(result.requestId)?.status).toBe("pending");
    closeSocket(second);
    expect(launch(result.requestId)?.status).toBe("unconfirmed");
  });

  it("reports launches from an earlier socket when a new one authenticates", async () => {
    await openSocket();
    negotiate();
    const result = addTerminal();
    // The first socket is abandoned without a close event.
    const next = await openSocket();
    useStore.setState({
      sessionId: null,
      refreshCliClients: vi.fn(),
      listSessions: vi.fn(),
      startAutoRefresh: vi.fn(),
    });
    deliver(next, { type: "authenticated", user_id: "user-1", negotiated_capabilities: [PROJECT_INSTANCES_CAPABILITY] });
    expect(launch(result.requestId)?.status).toBe("unconfirmed");
    expect(sentFrames(next, "add_pane")).toHaveLength(0);
  });

  it("reports a launch nobody answers as unconfirmed rather than waiting forever", async () => {
    useStore.setState({ machines: [owned(ZOO6, "zoo-006", { registered: true })] });
    const ws = await openSocket();
    negotiate();
    vi.useFakeTimers();
    const unanswered = addTerminal();
    vi.advanceTimersByTime(29_999);
    expect(launch(unanswered.requestId)?.status).toBe("pending");
    vi.advanceTimersByTime(1);
    expect(launch(unanswered.requestId)?.status).toBe("unconfirmed");

    // Starting an instance gets the server's start bound and then some.
    const starting = addTerminal(ZOO6);
    deliver(ws, { ...fixture("server-add-pane-starting.json"), request_id: starting.requestId });
    vi.advanceTimersByTime(60_000);
    expect(launch(starting.requestId)?.status).toBe("starting");
    vi.advanceTimersByTime(90_000);
    expect(launch(starting.requestId)).toMatchObject({ status: "unconfirmed", error: expect.stringContaining("zoo-006") });

    // A routed tab its instance never lists is let go.
    const routed = addTerminal();
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: routed.requestId, status: "routed", pane_id: 40 });
    vi.advanceTimersByTime(30_000);
    expect(launch(routed.requestId)).toBeUndefined();
    expect(sentFrames(ws, "add_pane")).toHaveLength(3);
  });
});

describe("clone and add", () => {
  const target = {
    machineId: LAPTOP,
    gitRemote: "github.com/example/project",
    instanceName: "project",
    cloneUrl: "https://github.com/example/project.git",
  };
  const tab = { provider: "claude", mode: "interactive", label: "Claude 3", kind: "terminal" as const };

  beforeEach(() => {
    useStore.setState({
      sessions: [project({ instances: [instance(ZOO5, "zoo-005", { primary: true })] })],
      machines: [owned(ZOO5, "zoo-005", { registered: true }), owned(LAPTOP, "laptop")],
    });
  });

  it("clones onto an owned machine, then adds the tab there as one launch", async () => {
    const ws = await openSocket();
    negotiate();
    const result = useStore.getState().cloneAndAddPane(target, tab);
    expect(result.success).toBe(true);
    const [clone] = sentFrames(ws, "create_project_instance");
    expect(clone).toEqual({ ...fixture("web-create-project-instance-join.json"), request_id: result.requestId });
    expectValidFrame(clone);
    expect(sentFrames(ws, "add_pane")).toHaveLength(0);
    expect(launch(result.requestId)).toMatchObject({ status: "cloning", machineId: LAPTOP, hostname: "laptop" });

    // Another machine's acknowledgement is not this clone's.
    deliver(ws, { type: "project_instance_created", machine_id: ZOO5, request_id: result.requestId, project_id: PROJECT });
    expect(launch(result.requestId)?.status).toBe("cloning");

    deliver(ws, { type: "project_instance_created", machine_id: LAPTOP, request_id: result.requestId, project_id: PROJECT });
    expect(listMachines).toHaveBeenCalled();
    const [add] = sentFrames(ws, "add_pane");
    expectValidFrame(add);
    expect(add).toMatchObject({ session_id: SID, machine_id: LAPTOP, kind: "terminal", label: "Claude 3" });
    expect(add.request_id).not.toBe(result.requestId);
    expect(Object.keys(useStore.getState().addPaneRequests)).toEqual([result.requestId]);
    expect(launch(result.requestId)).toMatchObject({ status: "pending", addRequestId: add.request_id, cloneRequestId: result.requestId });

    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: add.request_id, status: "starting", machine_id: LAPTOP });
    expect(launch(result.requestId)?.status).toBe("starting");
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: add.request_id, status: "routed", machine_id: LAPTOP, pane_id: 5 });
    expect(launch(result.requestId)).toMatchObject({ status: "routed", paneId: 5 });
    // None of this was a new project.
    expect(useStore.getState().pendingInstances).toEqual({});
  });

  it("adds no tab when the clone fails or does not join this project", async () => {
    const ws = await openSocket();
    negotiate();
    const failing = useStore.getState().cloneAndAddPane(target, tab);
    deliver(ws, { type: "project_instance_created", machine_id: LAPTOP, request_id: failing.requestId, error: "clone failed: auth" });
    expect(launch(failing.requestId)).toMatchObject({ status: "failed", error: "Cloning onto laptop failed: clone failed: auth" });

    // An older daemon or server would make a separate project instead.
    const separate = useStore.getState().cloneAndAddPane(target, tab);
    deliver(ws, { type: "project_instance_created", machine_id: LAPTOP, request_id: separate.requestId, project_id: "30000000-0000-4000-8000-0000000000ff" });
    expect(launch(separate.requestId)).toMatchObject({ status: "failed", error: expect.stringMatching(/separate project/) });
    expect(sentFrames(ws, "add_pane")).toHaveLength(0);
  });

  it("reports a clone whose answer was lost, and adds nothing when it arrives late", async () => {
    const ws = await openSocket();
    negotiate();
    const result = useStore.getState().cloneAndAddPane(target, tab);
    closeSocket(ws);
    expect(launch(result.requestId)).toMatchObject({ status: "unconfirmed", error: expect.stringMatching(/checkout may exist/) });

    // The daemon's acknowledgement reaches every one of the owner's clients.
    const next = await openSocket();
    negotiate();
    deliver(next, { type: "project_instance_created", machine_id: LAPTOP, request_id: result.requestId, project_id: PROJECT });
    expect(sentFrames(next, "add_pane")).toHaveLength(0);
    expect(sentFrames(next, "create_project_instance")).toHaveLength(0);
    expect(launch(result.requestId)).toMatchObject({
      status: "unconfirmed",
      error: "The clone onto laptop finished. If its tab does not appear in the tab bar, add one there from the new-tab menu.",
    });
  });

  it("never tells a duplicated browser tab that the clone its original carries on added no tab", async () => {
    // The browser copies a tab's session storage into a duplicate, launches
    // included, while the original goes on to add the tab. The clone's
    // acknowledgement reaches both pages.
    const ws = await openSocket();
    negotiate();
    const clone = useStore.getState().cloneAndAddPane(target, tab);

    vi.resetModules();
    const duplicate = (await import("./store")).useStore;
    expect(duplicate.getState().addPaneRequests[clone.requestId!]).toMatchObject({
      status: "unconfirmed",
      cloneRequestId: clone.requestId,
    });
    duplicate.setState({ showToast: vi.fn(), listMachines: vi.fn(), negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY]) });
    duplicate.getState().connect();
    await new Promise((done) => setTimeout(done, 10));
    const copy = duplicate.getState().ws as unknown as MockWs;

    const ack = { type: "project_instance_created", machine_id: LAPTOP, request_id: clone.requestId, project_id: PROJECT };
    deliver(ws, ack);
    deliver(copy, ack);

    // The original adds the tab, once.
    expect(sentFrames(ws, "add_pane")).toEqual([expect.objectContaining({ session_id: SID, machine_id: LAPTOP })]);
    // The duplicate adds nothing, and does not claim that nothing was added.
    expect(sentFrames(copy, "add_pane")).toHaveLength(0);
    const row = duplicate.getState().addPaneRequests[clone.requestId!];
    expect(row).toMatchObject({ status: "unconfirmed" });
    expect(row.error).not.toMatch(/no tab was added/);
    expect(row.error).toMatch(/If its tab does not appear in the tab bar, add one there/);
    duplicate.getState().disconnect();
  });

  it("refuses machines where a clone could not join", async () => {
    const ws = await openSocket();
    const attempt = () => useStore.getState().cloneAndAddPane(target, tab).error;
    expect(attempt()).toMatch(/newer APAS server/);
    negotiate();
    useStore.setState({ machines: [owned(LAPTOP, "laptop", { clusterAccess: "member" })] });
    expect(attempt()).toMatch(/your own machines/);
    useStore.setState({ machines: [owned(LAPTOP, "laptop", { daemonConnected: false, projectInstancesAvailable: false })] });
    expect(attempt()).toMatch(/laptop is offline/);
    useStore.setState({ machines: [owned(LAPTOP, "laptop", { projectInstancesAvailable: false })] });
    expect(attempt()).toMatch(/Reboot laptop to update/);
    useStore.setState({ machines: [owned(LAPTOP, "laptop", { registered: true })] });
    expect(attempt()).toMatch(/already has this project/);
    expect(sentFrames(ws, "create_project_instance")).toHaveLength(0);
    expect(useStore.getState().addPaneRequests).toEqual({});
  });

  it("keeps its launches across a reload of the tab, reporting the cut-off ones unconfirmed", async () => {
    const ws = await openSocket();
    negotiate();
    const clone = useStore.getState().cloneAndAddPane(target, tab);
    const pending = addTerminal();
    const failed = addTerminal();
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: failed.requestId, status: "failed", error: "zoo-005 refused the tab" });
    const routed = addTerminal();
    deliver(ws, { type: "add_pane_result", session_id: SID, request_id: routed.requestId, status: "routed", pane_id: 41 });

    // Per tab: the reloaded page is the same tab, and no other tab's launches
    // are its to report.
    const stored = JSON.parse(sessionStorage.getItem(ADD_PANE_REQUESTS_KEY) ?? "[]") as { requestId: string; status: string }[];
    expect(Object.fromEntries(stored.map((request) => [request.requestId, request.status]))).toEqual({
      [clone.requestId!]: "cloning",
      [pending.requestId!]: "pending",
      [failed.requestId!]: "failed",
    });

    // The reload: a fresh store, from this tab's storage alone.
    vi.resetModules();
    const reloaded = (await import("./store")).useStore;
    const restored = reloaded.getState().addPaneRequests;
    // Their answers went only to the connection the reload ended.
    expect(restored[clone.requestId!]).toMatchObject({
      status: "unconfirmed",
      machineId: LAPTOP,
      hostname: "laptop",
      cloneRequestId: clone.requestId,
      error: expect.stringMatching(/checkout may exist there without a tab\. Nothing was resent/),
    });
    expect(restored[pending.requestId!]).toMatchObject({ status: "unconfirmed", error: expect.stringMatching(/It was not resent/) });
    expect(restored[failed.requestId!]).toMatchObject({ status: "failed", error: "zoo-005 refused the tab" });
    expect(restored[routed.requestId!]).toBeUndefined();

    // The clone's acknowledgement reaches every connection of its owner, the
    // reloaded page's included: it says where the checkout went, and still
    // nothing is added or sent again.
    reloaded.setState({ showToast: vi.fn(), listMachines: vi.fn(), negotiatedCapabilities: new Set([PROJECT_INSTANCES_CAPABILITY]) });
    reloaded.getState().connect();
    await new Promise((done) => setTimeout(done, 10));
    const next = reloaded.getState().ws as unknown as MockWs;
    deliver(next, { type: "project_instance_created", machine_id: LAPTOP, request_id: clone.requestId, project_id: PROJECT });
    expect(reloaded.getState().addPaneRequests[clone.requestId!]).toMatchObject({
      status: "unconfirmed",
      error: expect.stringMatching(/The clone onto laptop finished\. If its tab does not appear in the tab bar, add one there/),
    });
    expect(sentFrames(next, "add_pane")).toHaveLength(0);
    expect(sentFrames(next, "create_project_instance")).toHaveLength(0);

    // Dismissed is gone for good, reloads included.
    reloaded.getState().dismissAddPaneRequest(failed.requestId!);
    expect((JSON.parse(sessionStorage.getItem(ADD_PANE_REQUESTS_KEY) ?? "[]") as { requestId: string }[])
      .map((request) => request.requestId)).not.toContain(failed.requestId);
    reloaded.getState().disconnect();
    expect(sessionStorage.getItem(ADD_PANE_REQUESTS_KEY)).toBeNull();
  });

  it("restores only well-formed launches from storage", () => {
    const base = { requestId: "a", sessionId: SID, provider: "claude", kind: "terminal", startedAt: 1, updatedAt: 2 };
    expect(restoreAddPaneRequests(null)).toEqual({});
    expect(restoreAddPaneRequests({ a: base })).toEqual({});
    expect(restoreAddPaneRequests([
      { ...base, status: "starting", hostname: "zoo-006" },
      { ...base, requestId: "b", status: "teleporting" },
      { ...base, requestId: "c", status: "failed", kind: "spaceship" },
      { ...base, requestId: "d", status: "failed", paneId: 1.5 },
      { ...base, requestId: "", status: "failed" },
      "junk",
    ])).toEqual({
      a: {
        ...base,
        kind: "terminal",
        status: "unconfirmed",
        hostname: "zoo-006",
        machineId: undefined,
        addRequestId: undefined,
        cloneRequestId: undefined,
        label: undefined,
        paneId: undefined,
        error: "Connection lost before the server confirmed this tab. It was not resent; check the tab bar before trying again.",
      },
    });
  });
});

describe("lifecycle per instance", () => {
  it("keeps one inventory per instance and reboots the chosen machine", async () => {
    useStore.setState({
      sessions: [project({ instances: [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")] })],
    });
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-cli-lifecycle-inventory-instance.json"));
    deliver(ws, {
      type: "cli_lifecycle_inventory", session_id: SID, machine_id: ZOO5,
      inventory: { persistent_terminal_hosting: false, panes: [] },
    });
    const state = useStore.getState();
    expect(selectCliLifecycleInventory(state, SID, ZOO6)).toMatchObject({ persistent_terminal_hosting: true, panes: [{ pane_id: 4 }] });
    // Naming no machine means the primary's.
    expect(selectCliLifecycleInventory(state, SID)).toMatchObject({ persistent_terminal_hosting: false });

    const requestId = useStore.getState().rebootCli(ZOO6);
    const [request] = sentFrames(ws, "cli_lifecycle_request");
    expect(request).toEqual({ ...fixture("web-cli-lifecycle-request-targeted.json"), request_id: requestId });
    expectValidFrame(request);
    expect(useStore.getState().cliLifecycleOperations[requestId!]).toMatchObject({ machineId: ZOO6, inventory: { persistent_terminal_hosting: true } });

    // Progress carrying an inventory replaces only that machine's.
    deliver(ws, {
      ...fixture("server-cli-lifecycle-status-instance.json"),
      request_id: requestId,
      inventory: { persistent_terminal_hosting: true, panes: [] },
    });
    expect(selectCliLifecycleInventory(useStore.getState(), SID, ZOO6)?.panes).toEqual([]);
    expect(selectCliLifecycleInventory(useStore.getState(), SID, ZOO5)).toMatchObject({ persistent_terminal_hosting: false });

    useStore.getState().rebootCli();
    expect(sentFrames(ws, "cli_lifecycle_request")[1]).not.toHaveProperty("machine_id");
  });

  it("drops the inventory of an instance that left", async () => {
    const ws = await openSocket();
    deliver(ws, fixture("server-cli-lifecycle-inventory-instance.json"));
    deliver(ws, { type: "cli_lifecycle_inventory", session_id: SID, machine_id: ZOO5, inventory: { persistent_terminal_hosting: false, panes: [] } });
    deliver(ws, { type: "session_instances", session_id: SID, instances: [{ machine_id: ZOO5, hostname: "zoo-005", primary: true }] });
    expect(Object.keys(useStore.getState().cliLifecycleInventories[SID])).toEqual([ZOO5]);
  });

  it("files an older server's inventory as the session's only one", async () => {
    const ws = await openSocket();
    deliver(ws, { type: "cli_lifecycle_inventory", session_id: SID, inventory: { persistent_terminal_hosting: true, panes: [] } });
    expect(Object.keys(useStore.getState().cliLifecycleInventories[SID])).toEqual([LEGACY_MACHINE_KEY]);
    expect(selectCliLifecycleInventory(useStore.getState(), SID)).toMatchObject({ persistent_terminal_hosting: true });

    // A machine cannot be named to a server that would reboot the primary.
    expect(useStore.getState().rebootCli(ZOO6)).toBeNull();
    expect(sentFrames(ws, "cli_lifecycle_request")).toHaveLength(0);
    useStore.getState().rebootCli();
    expect(sentFrames(ws, "cli_lifecycle_request")[0]).not.toHaveProperty("machine_id");
  });

  it("checks only the rebooted instance's panes against policy", async () => {
    useStore.setState({
      sessions: [project({ instances: [instance(ZOO5, "zoo-005", { primary: true }), instance(ZOO6, "zoo-006")] })],
      paneConfigs: [
        { pane_id: 3, provider: "claude", mode: "interactive", kind: "terminal", session_id: "p3", is_paused: false, machine_id: ZOO5 },
        // Outside the allowlist, but on zoo-006.
        { pane_id: 4, provider: "opencode", mode: "interactive", kind: "terminal", session_id: "p4", is_paused: false, machine_id: ZOO6 },
      ],
    });
    const ws = await openSocket();
    negotiate();
    deliver(ws, { type: "cli_lifecycle_inventory", session_id: SID, machine_id: ZOO5, inventory: { persistent_terminal_hosting: true, panes: [] } });
    deliver(ws, { type: "cli_lifecycle_inventory", session_id: SID, machine_id: ZOO6, inventory: { persistent_terminal_hosting: true, panes: [] } });
    expect(useStore.getState().rebootCli(ZOO5)).not.toBeNull();
    expect(useStore.getState().rebootCli(ZOO6)).toBeNull();
    expect(showToast).toHaveBeenCalledWith(expect.stringContaining("outside cluster policy"), "error");
  });
});

describe("the session's CLI identity", () => {
  // Every attached instance reports the project's session as its own, and
  // the server lists them in no fixed order. The view's layout is keyed by
  // the one this picks, so it must not follow whichever is listed first.
  const PRIMARY_CLI = "cli-zoo-005";
  const SECONDARY_CLI = "cli-zoo-006";

  function cli(id: string, activeSession: string | null = SID) {
    return { id, name: id, status: "online", version: "26.11.0", last_seen: null, active_session: activeSession };
  }

  function polledSession(cliClientId: string | null) {
    return { id: SID, project_id: PROJECT, cli_client_id: cliClientId, status: "active", is_active: true };
  }

  beforeEach(() => {
    useStore.setState({ cliClientId: PRIMARY_CLI, cliClients: [], sessions: [project({ cliClientId: PRIMARY_CLI })] });
  });

  it("stays on the primary's connection while another machine's instance attaches and leaves", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, { type: "cli_clients", clients: [cli(SECONDARY_CLI), cli(PRIMARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(PRIMARY_CLI);
    deliver(ws, { type: "sessions", sessions: [polledSession(PRIMARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(PRIMARY_CLI);
    deliver(ws, { type: "cli_clients", clients: [cli(PRIMARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(PRIMARY_CLI);
  });

  it("keeps the connection in use while the session row names none that is live", async () => {
    useStore.setState({ sessions: [project()] });
    const ws = await openSocket();
    negotiate();
    deliver(ws, { type: "cli_clients", clients: [cli(SECONDARY_CLI), cli(PRIMARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(PRIMARY_CLI);
  });

  it("moves to the new primary only once the old one's connection is gone", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, { type: "cli_clients", clients: [cli(SECONDARY_CLI), cli(PRIMARY_CLI)] });
    // zoo-005 stops: zoo-006 is the only connection left, before the polled
    // row has caught up with the handover.
    deliver(ws, { type: "cli_clients", clients: [cli(SECONDARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(SECONDARY_CLI);
    deliver(ws, { type: "sessions", sessions: [polledSession(SECONDARY_CLI)] });
    // zoo-005 comes back as a secondary on a new connection.
    deliver(ws, { type: "cli_clients", clients: [cli("cli-zoo-005-again"), cli(SECONDARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(SECONDARY_CLI);
    // Nothing live: the poll falls back to the row, as it always did.
    deliver(ws, { type: "cli_clients", clients: [] });
    deliver(ws, { type: "sessions", sessions: [polledSession(SECONDARY_CLI)] });
    expect(useStore.getState().cliClientId).toBe(SECONDARY_CLI);
  });

  it("picks the primary's connection when attaching to a project", () => {
    useStore.setState({
      cliClientId: "cli-of-another-project",
      cliClients: [
        { id: SECONDARY_CLI, status: "online", activeSession: SID },
        { id: PRIMARY_CLI, status: "online", activeSession: SID },
      ],
    });
    useStore.getState().commitSessionAttachment(SID, true);
    expect(useStore.getState().cliClientId).toBe(PRIMARY_CLI);
  });

  it("takes the one live connection an older server lists, as before", async () => {
    useStore.setState({ cliClientId: null, sessions: [project({ cliClientId: "cli-stale" })] });
    const ws = await openSocket();
    deliver(ws, { type: "cli_clients", clients: [cli("cli-other", "30000000-0000-4000-8000-0000000000aa"), cli("cli-current")] });
    expect(useStore.getState().cliClientId).toBe("cli-current");
  });
});

describe("refused input", () => {
  // A negotiated viewer's typed input that the server refuses comes back as
  // a mutation_ack carrying the input's client_msg_id, after the Error that
  // already raised the notice. The send is settled: never sent again under
  // that id, its reason shown where it is shown, its text kept.
  const PANE = 4;
  const REFUSAL = "zoo-005 is not running this project. Start it on zoo-005 first.";

  function refusalFor(clientMsgId: unknown): Record<string, unknown> {
    const ack = fixture("server-mutation-ack-input.json");
    expect(ack).toMatchObject({ mutation: "input", accepted: false, error: REFUSAL });
    return { ...ack, request_id: clientMsgId, session_id: SID, pane_id: PANE };
  }

  function bubble(clientMsgId: unknown) {
    return useStore.getState().paneMessages[String(PANE)]
      ?.find((message) => message.id === `optimistic-${clientMsgId}` || message.id === clientMsgId);
  }

  beforeEach(() => {
    localStorage.removeItem("apas_pending_sends");
    localStorage.removeItem("apas_terminal_deliveries");
    useStore.setState({
      isDualPane: true,
      paneMessages: {},
      pendingSends: [],
      pendingTerminalDeliveries: [],
      pendingSessionAttachment: null,
    });
  });

  it("is neither retransmitted nor replayed, and its bubble keeps the text with the reason", async () => {
    const ws = await openSocket();
    negotiate();
    vi.useFakeTimers();
    expect(useStore.getState().sendMessageToPane("Run the suite.", PANE)).toEqual({ success: true });
    const [input] = sentFrames(ws, "input");
    deliver(ws, refusalFor(input.client_msg_id));

    expect(useStore.getState().pendingSends).toEqual([]);
    expect(localStorage.getItem("apas_pending_sends")).toBeNull();
    expect(bubble(input.client_msg_id)).toMatchObject({ content: "Run the suite.", deliveryError: REFUSAL });
    // The Error frame is the notice; the acknowledgement adds none.
    expect(showToast).not.toHaveBeenCalled();

    // No retransmission when the 3 s retry comes due...
    vi.advanceTimersByTime(20_000);
    // ...and no replay once the session is attached again.
    useStore.setState({ pendingSessionAttachment: { sessionId: SID, forceReload: false, requestedFromSessionId: SID } });
    deliver(ws, { type: "session_attached", session_id: SID, has_active_cli: true });
    expect(sentFrames(ws, "input")).toEqual([input]);

    // A second acknowledgement for the settled id changes nothing.
    const settled = useStore.getState();
    deliver(ws, refusalFor(input.client_msg_id));
    expect(useStore.getState().paneMessages).toBe(settled.paneMessages);
    expect(useStore.getState().pendingSends).toBe(settled.pendingSends);
  });

  it("stands corrected by an echo of an attempt that was delivered after all", async () => {
    const ws = await openSocket();
    negotiate();
    useStore.getState().sendMessageToPane("Run the suite.", PANE);
    const [input] = sentFrames(ws, "input");
    deliver(ws, refusalFor(input.client_msg_id));
    deliver(ws, { type: "user_input", session_id: SID, pane_id: PANE, text: "Run the suite.", client_msg_id: input.client_msg_id });
    const delivered = useStore.getState().paneMessages[String(PANE)];
    expect(delivered).toHaveLength(1);
    expect(delivered[0]).toMatchObject({ id: input.client_msg_id, content: "Run the suite." });
    expect(delivered[0].deliveryError).toBeUndefined();
  });

  it("settles a terminal delivery with the server's reason instead of leaving it unconfirmed", async () => {
    const ws = await openSocket();
    negotiate();
    expect(useStore.getState().sendTerminalConversationMessage(PANE, "Run the suite.")).toEqual({ success: true });
    const [input] = sentFrames(ws, "terminal_conversation_input");
    deliver(ws, refusalFor(input.client_msg_id));

    const [delivery] = useStore.getState().pendingTerminalDeliveries;
    expect(delivery).toMatchObject({ id: input.client_msg_id, text: "Run the suite.", error: REFUSAL });
    // Stored for a reload, reason included.
    expect(JSON.parse(localStorage.getItem("apas_terminal_deliveries") ?? "[]")).toEqual([delivery]);
    expect(unconfirmedDeliveries([delivery], delivery.sentAt + 60_000)).toEqual([]);
    expect(refusedDeliveries([delivery])).toEqual([delivery]);
    expect(bubble(input.client_msg_id)).toMatchObject({ deliveryError: REFUSAL });
    expect(showToast).not.toHaveBeenCalled();

    // Recorded after all: the transcript's acknowledgement still clears it.
    deliver(ws, { type: "terminal_conversation_recorded", session_id: SID, pane_id: PANE, client_msg_id: input.client_msg_id });
    expect(useStore.getState().pendingTerminalDeliveries).toEqual([]);
  });

  it("leaves another session's send alone", async () => {
    const ws = await openSocket();
    negotiate();
    useStore.getState().sendMessageToPane("Run the suite.", PANE);
    const [input] = sentFrames(ws, "input");
    deliver(ws, { ...refusalFor(input.client_msg_id), session_id: "20000000-0000-4000-8000-0000000000ff" });
    expect(useStore.getState().pendingSends).toHaveLength(1);
    expect(bubble(input.client_msg_id)?.deliveryError).toBeUndefined();
  });
});
