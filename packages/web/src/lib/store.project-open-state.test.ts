import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, type Mock } from "vitest";
import { validateClientMessage, validateServerMessage } from "@apas/protocol";
import { useStore } from "./store";
import {
  buildProjectOpenModel,
  openStateNegotiated,
  PROJECT_OPEN_STATE_CAPABILITY,
  selectProjectOpenReport,
} from "./projectOpenState";

/**
 * A project's open state through the real socket handler, with the
 * protocol's golden frames. Every frame the store sends is checked against
 * the schema the server parses.
 */

const SID = "20000000-0000-4000-8000-000000000001";
const PROJECT = "30000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000001";
const ZOO6 = "50000000-0000-4000-8000-000000000002";

type MockWs = {
  send: Mock;
  onmessage?: (event: MessageEvent) => void;
  onclose?: (event: CloseEvent) => void;
};

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8"));
}

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

function negotiate(capable = true) {
  useStore.setState({ negotiatedCapabilities: new Set(capable ? [PROJECT_OPEN_STATE_CAPABILITY] : []) });
}

const initialState = useStore.getState();

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  useStore.setState(initialState, true);
  useStore.setState({ sessionId: SID, isAttached: true, toasts: [] });
});

afterEach(() => {
  useStore.getState().disconnect();
});

describe("negotiation", () => {
  it("advertises project_open_state_v1 and records whether the server echoed it", async () => {
    const ws = await openSocket();
    const [authenticate] = sentFrames(ws, "authenticate");
    expect(authenticate.capabilities).toContain("project_open_state_v1");
    expect(validateClientMessage(authenticate).valid).toBe(true);

    deliver(ws, { type: "authenticated", user_id: "u1", negotiated_capabilities: ["project_instances_v1"] });
    expect(openStateNegotiated(useStore.getState())).toBe(false);
    deliver(ws, { type: "authenticated", user_id: "u1", negotiated_capabilities: ["project_open_state_v1"] });
    expect(openStateNegotiated(useStore.getState())).toBe(true);
  });

  it("ignores a frame from a server that did not negotiate it", async () => {
    const ws = await openSocket();
    negotiate(false);
    deliver(ws, fixture("server-project-open-state.json"));
    expect(useStore.getState().projectOpenStates).toEqual({});
  });
});

describe("the project_open_state frame", () => {
  it("is held per project, and each frame replaces the last", async () => {
    const ws = await openSocket();
    negotiate();
    for (const name of [
      "server-project-open-state.json",
      "server-project-open-state-closed.json",
      "server-project-open-state-not-recorded.json",
      "server-project-open-state-refused.json",
    ]) {
      expect(validateServerMessage(fixture(name))).toMatchObject({ valid: true, errors: [] });
    }

    deliver(ws, fixture("server-project-open-state.json"));
    expect(selectProjectOpenReport(useStore.getState(), [PROJECT])).toMatchObject({
      projectId: PROJECT,
      state: "open",
      setBy: "owner@example.com",
      setKind: "request",
      setAt: "2026-11-16T05:00:00Z",
      machines: [
        { machineId: ZOO5, hostname: "zoo-005", part: "running", ownsTabs: true, connected: true },
        { machineId: ZOO6, hostname: "zoo-006", part: "waiting_offline", ownsTabs: true, connected: false },
      ],
    });

    deliver(ws, fixture("server-project-open-state-closed.json"));
    const closed = selectProjectOpenReport(useStore.getState(), [PROJECT]);
    expect(closed).toMatchObject({ state: "closed", setKind: "administrative" });
    expect(closed?.setBy).toBeUndefined();
    expect(closed?.machines.map((machine) => machine.part)).toEqual(["still_running", "not_started"]);

    // A frame without a state is a project nobody has recorded.
    deliver(ws, fixture("server-project-open-state-not-recorded.json"));
    const unrecorded = selectProjectOpenReport(useStore.getState(), [PROJECT]);
    expect(unrecorded?.state).toBeUndefined();
    expect(unrecorded?.machines).toHaveLength(2);
    expect(Object.keys(useStore.getState().projectOpenStates)).toEqual([PROJECT]);
  });

  it("updates the project's list entries at once, so the lists do not wait for a poll", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-sessions-open-state.json"));
    deliver(ws, fixture("server-machines-open-state.json"));
    const session = () => useStore.getState().sessions.find((item) => item.projectId === PROJECT);
    const entry = () => useStore.getState().machines[0].projects.find((item) => item.projectId === PROJECT);
    expect(session()?.openState).toBe("open");
    expect(entry()?.openState).toBe("open");
    const untouched = useStore.getState().sessions.find((item) => item.projectId !== PROJECT);

    deliver(ws, fixture("server-project-open-state-closed.json"));
    expect(session()?.openState).toBe("closed");
    expect(entry()?.openState).toBe("closed");
    // The machine's own part still comes from the machine list.
    expect(entry()?.part).toBe("not_started");
    expect(useStore.getState().sessions.find((item) => item.projectId !== PROJECT)).toBe(untouched);

    deliver(ws, fixture("server-project-open-state-not-recorded.json"));
    expect(session()?.openState).toBeUndefined();
    expect(entry()?.openState).toBeUndefined();
  });

  it("is dropped with the socket, while the lists keep the state they carried", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-sessions-open-state.json"));
    deliver(ws, fixture("server-project-open-state.json"));
    expect(useStore.getState().projectOpenStates[PROJECT]).toBeDefined();
    ws.onclose?.(new CloseEvent("close", { code: 1006 }));
    expect(useStore.getState().projectOpenStates).toEqual({});
    expect(useStore.getState().sessions.find((item) => item.projectId === PROJECT)?.openState).toBe("open");
  });

  it("is dropped when the viewer attaches to another project, and kept on a re-attach", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-sessions-open-state.json"));
    deliver(ws, fixture("server-project-open-state.json"));
    // The same session again (a reload of its history): the frame stays.
    useStore.getState().attachSession(SID, true);
    expect(useStore.getState().projectOpenStates[PROJECT]).toBeDefined();
    // Another session: nothing says the held frame is kept up to date while
    // this viewer is away. The lists still carry the state.
    useStore.getState().attachSession("20000000-0000-4000-8000-0000000000ff");
    expect(useStore.getState().projectOpenStates).toEqual({});
    expect(useStore.getState().sessions.find((item) => item.projectId === PROJECT)?.openState).toBe("open");
  });

  it("goes with a forgotten project", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-project-open-state.json"));
    useStore.getState().forgetProject(PROJECT);
    expect(useStore.getState().projectOpenStates).toEqual({});
  });
});

describe("the lists", () => {
  it("read open_state from session entries, absent meaning not recorded", async () => {
    const ws = await openSocket();
    deliver(ws, fixture("server-sessions-open-state.json"));
    expect(useStore.getState().sessions.map((session) => session.openState)).toEqual(["open", "closed", undefined]);
  });

  it("read the server-filled fields of machine entries", async () => {
    const ws = await openSocket();
    deliver(ws, fixture("server-machines-open-state.json"));
    const [open, closed, unrecorded, registered] = useStore.getState().machines[0].projects;
    expect(open).toMatchObject({
      openState: "open",
      part: "not_started",
      partReason: "zoo-006 could not start the project: the checkout is missing.",
      otherHosts: [
        { hostname: "zoo-005", connected: true, running: true, ownsTabs: true },
        { hostname: "laptop", connected: false, running: false, ownsTabs: true },
      ],
    });
    expect(closed).toMatchObject({ openState: "closed" });
    expect(closed.part).toBeUndefined();
    expect(closed.otherHosts).toBeUndefined();
    expect(unrecorded).toMatchObject({ part: "running", isRunning: true });
    expect(unrecorded.openState).toBeUndefined();
    expect(registered.openState).toBeUndefined();
    expect(registered.part).toBeUndefined();
  });

  it("ignore values outside the vocabulary", async () => {
    const ws = await openSocket();
    deliver(ws, {
      type: "machines",
      machines: [{
        machine: { machine_id: ZOO5, hostname: "zoo-005", os: "linux", arch: "x64" },
        projects: [{ project_id: PROJECT, path: "/p", is_running: false, open_state: "ajar", part: "paused" }],
      }],
    });
    const [entry] = useStore.getState().machines[0].projects;
    expect(entry.openState).toBeUndefined();
    expect(entry.part).toBeUndefined();
  });

  it("feed one model: the frame for a project in view, the machine list otherwise", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-sessions-open-state.json"));
    deliver(ws, fixture("server-machines-open-state.json"));
    const build = () => {
      const state = useStore.getState();
      return buildProjectOpenModel({
        supported: openStateNegotiated(state),
        projectIds: [PROJECT],
        report: selectProjectOpenReport(state, [PROJECT]),
        session: state.sessions.find((item) => item.projectId === PROJECT),
        machines: state.machines,
        target: { machineId: ZOO6, projectId: PROJECT },
      });
    };
    // No frame yet: the machine list names the other machines.
    expect(build().machines.map((machine) => [machine.hostname, machine.part])).toEqual([
      ["laptop", "waiting_offline"],
      ["zoo-005", "running"],
      ["zoo-006", "not_started"],
    ]);
    deliver(ws, fixture("server-project-open-state-refused.json"));
    expect(build().machines.map((machine) => [machine.hostname, machine.part])).toEqual([
      ["zoo-005", "starting"],
      ["zoo-006", "not_started"],
    ]);
  });
});

describe("openProject and closeProject", () => {
  it("send the two existing requests for the whole project, once", async () => {
    const ws = await openSocket();
    expect(useStore.getState().openProject(ZOO5, PROJECT)).toBe(true);
    expect(useStore.getState().closeProject(ZOO5, PROJECT)).toBe(true);
    const frames = sentFrames(ws).filter((frame) =>
      frame.type === "start_machine_project_cli" || frame.type === "stop_machine_project_cli");
    expect(frames).toEqual([
      fixture("web-start-machine-project-cli-all.json"),
      fixture("web-stop-machine-project-cli-all.json"),
    ]);
    for (const frame of frames) {
      expect(frame.all_instances).toBe(true);
      expect(validateClientMessage(frame)).toMatchObject({ valid: true, errors: [] });
    }
  });

  it("send the same requests to a server without the capability", async () => {
    const ws = await openSocket();
    negotiate(false);
    useStore.getState().openProject(ZOO5, PROJECT);
    useStore.getState().closeProject(ZOO5, PROJECT);
    expect(sentFrames(ws, "start_machine_project_cli")).toEqual([fixture("web-start-machine-project-cli-all.json")]);
    expect(sentFrames(ws, "stop_machine_project_cli")).toEqual([fixture("web-stop-machine-project-cli-all.json")]);
  });

  it("report that nothing was sent while disconnected, and send nothing later", async () => {
    const ws = await openSocket();
    useStore.setState({ ws: null });
    expect(useStore.getState().openProject(ZOO5, PROJECT)).toBe(false);
    expect(useStore.getState().closeProject(ZOO5, PROJECT)).toBe(false);
    useStore.setState({ ws: ws as never });
    expect(sentFrames(ws, "start_machine_project_cli")).toEqual([]);
    expect(sentFrames(ws, "stop_machine_project_cli")).toEqual([]);
  });
});

describe("no request names one machine alone", () => {
  /** Every source file of the web that is not a test. */
  function sourceFiles(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) return sourceFiles(path);
      return /\.(ts|tsx)$/.test(entry.name) && !/\.test\.(ts|tsx)$/.test(entry.name) ? [path] : [];
    });
  }

  it("the store has no per-machine start or stop", () => {
    const actions = useStore.getState() as unknown as Record<string, unknown>;
    expect(actions.startMachineProjectCli).toBeUndefined();
    expect(actions.stopMachineProjectCli).toBeUndefined();
  });

  it("only openProject and closeProject build the two requests, each with all_instances: true", () => {
    const root = resolve(__dirname, "..");
    const senders = sourceFiles(root).flatMap((path) => {
      const source = readFileSync(path, "utf8");
      // Each object literal that names one of the two request types, up to
      // its closing brace.
      const literals = source.match(/\{\s*type: "(?:start|stop)_machine_project_cli",[^}]*\}/g) ?? [];
      return literals.map((literal) => ({ path: path.slice(root.length + 1), literal }));
    });
    expect(senders.map((sender) => sender.path)).toEqual(["lib/store.ts", "lib/store.ts"]);
    for (const { literal } of senders) {
      expect(literal).toMatch(/\ball_instances: true,/);
      expect(literal).not.toMatch(/\.\.\./);
    }
    // No other spelling of the request or of its flag is left anywhere.
    for (const path of sourceFiles(root)) {
      const source = readFileSync(path, "utf8");
      const mentions = source.match(/"(?:start|stop)_machine_project_cli"/g) ?? [];
      expect([path.slice(root.length + 1), mentions.length]).toEqual(
        [path.slice(root.length + 1), path.endsWith(join("lib", "store.ts")) ? 2 : 0],
      );
      const otherSpelling = /all_instances:(?!\s*true\b)/.test(source) || /\ballInstances\b/.test(source);
      expect([path.slice(root.length + 1), otherSpelling]).toEqual([path.slice(root.length + 1), false]);
    }
  });
});
