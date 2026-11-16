import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { MobileBootstrapResponse, ServerToWeb } from "@apas/protocol";

import { bootstrap } from "@/api/client";
import { MOBILE_CAPABILITIES } from "@/config/endpoints";
import { acceptEvents } from "@/storage/cache";
import { tabUnavailableReason } from "@/state/projectInstances";
import { useMobileStore } from "@/state/store";
import { startConnectionRuntime, stopConnectionRuntime } from "./runtime";

// The whole native connection, from the socket to the store, with only the
// device's storage, credentials and network sensors replaced.
jest.mock("@react-native-community/netinfo", () => ({
  __esModule: true,
  default: { addEventListener: jest.fn(() => jest.fn()) },
}));
jest.mock("@/api/client", () => ({
  ApiError: class extends Error { status = 0; },
  bootstrap: jest.fn(),
  validAccessToken: jest.fn(async () => "access"),
}));
jest.mock("@/storage/cache", () => ({
  acceptEvents: jest.fn(async () => undefined),
  removeInaccessibleSessions: jest.fn(async () => undefined),
  replaceSessionSummaries: jest.fn(async () => undefined),
  wipeCache: jest.fn(async () => undefined),
  writePaneWorkSummarySnapshot: jest.fn(async () => undefined),
}));
jest.mock("@/security/credentials", () => ({ clearCredentials: jest.fn(async () => undefined) }));
jest.mock("@/notifications", () => ({
  startNotificationAndLinkRuntime: jest.fn(),
  stopNotificationAndLinkRuntime: jest.fn(),
}));
jest.mock("@/jobs/runtime", () => ({
  acceptJobsBootstrap: jest.fn(async () => undefined),
  setJobEnvironment: jest.fn(),
  startJobsRuntime: jest.fn(),
  stopJobsRuntime: jest.fn(async () => undefined),
  purgeJobAccess: jest.fn(async () => undefined),
}));
jest.mock("@/query/client", () => ({ queryClient: { clear: jest.fn(), invalidateQueries: jest.fn() } }));
jest.mock("@/workers/runtime", () => ({ bindWorkerSocket: jest.fn(), handleWorkerMessage: jest.fn(() => false) }));

type Frame<T extends ServerToWeb["type"]> = Extract<ServerToWeb, { type: T }>;

function fixture<T extends ServerToWeb["type"]>(name: string): Frame<T> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8")) as Frame<T>;
}

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  sent: { type: string; [key: string]: unknown }[] = [];
  closed = false;
  constructor(readonly url: string) { FakeWebSocket.instances.push(this); }
  send(data: string) { this.sent.push(JSON.parse(data)); }
  close() { this.closed = true; this.readyState = 3; }
  open() { this.readyState = 1; this.onopen?.(); }
  receive(value: object) { this.onmessage?.({ data: JSON.stringify(value) }); }
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 30; turn += 1) await Promise.resolve();
}

const sessionsFrame = fixture<"sessions">("server-sessions-open-state.json");
const machinesFrame = fixture<"machines">("server-machines-open-state.json");
const instancesFrame = fixture<"session_instances">("server-session-instances.json");
const paneListFrame = fixture<"pane_list">("server-pane-list-instances.json");
// The fixture's viewer may not use zoo-006; this one may.
const zoo005 = instancesFrame.instances[0];
const zoo006 = { ...instancesFrame.instances[1], runtime_access: true };
const bothMachines: Frame<"session_instances"> = { ...instancesFrame, instances: [zoo005, zoo006] };
const [openSession, closedSession, notRecordedSession] = sessionsFrame.sessions;
const zoo006Pane = paneListFrame.panes.find((pane) => pane.machine_id === zoo006.machine_id);

// The bootstrap's session entries flatten the session entry of a live list,
// and its machine list is the list a `machines` frame carries.
const bootstrapValue: MobileBootstrapResponse = {
  user_id: "1027ac43-3c54-467a-90e6-e50a170d4882",
  user_email: "mobile@example.test",
  cluster_role: "user",
  account_status: "active",
  protocol_min_version: 1,
  protocol_max_version: 1,
  features: {},
  sessions: sessionsFrame.sessions.map((session) => ({ ...session, project_name: "project", attention_count: 0 })),
  machines: machinesFrame.machines,
  launch_targets: [],
};

describe("native connection while a project is closed and opened", () => {
  const originalWebSocket = globalThis.WebSocket;

  beforeEach(() => {
    jest.useFakeTimers();
    FakeWebSocket.instances = [];
    globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
    jest.mocked(bootstrap).mockReset().mockResolvedValue(bootstrapValue);
    jest.mocked(acceptEvents).mockClear();
    useMobileStore.getState().reset();
  });

  afterEach(() => {
    stopConnectionRuntime();
    globalThis.WebSocket = originalWebSocket;
    jest.useRealTimers();
  });

  it("does not ask for the open state, reads entries that carry it, and stays connected", async () => {
    expect(sessionsFrame.sessions.map((session) => session.open_state)).toEqual(["open", "closed", undefined]);
    expect(Object.keys(machinesFrame.machines[0].projects[0])).toEqual(
      expect.arrayContaining(["open_state", "part", "part_reason", "other_hosts"]),
    );

    startConnectionRuntime();
    const [socket] = FakeWebSocket.instances;
    socket.open();
    await settle();
    expect(socket.sent[0]).toMatchObject({ type: "authenticate", client_kind: "mobile" });
    expect(socket.sent[0].capabilities).toEqual([...MOBILE_CAPABILITIES]);
    expect(socket.sent[0].capabilities).not.toContain("project_open_state_v1");

    // The server confirms only what was asked for, so not the open state.
    socket.receive({
      type: "authenticated",
      user_id: bootstrapValue.user_id,
      mutations_allowed: true,
      negotiated_capabilities: ["project_instances_v1"],
    });
    await settle();
    expect(useMobileStore.getState().connection).toBe("ready");
    expect(useMobileStore.getState().negotiatedCapabilities).not.toContain("project_open_state_v1");
    // Sessions that carry the field are kept as they came, and what the app
    // shows as running still follows `is_active`.
    expect(useMobileStore.getState().sessions).toEqual(bootstrapValue.sessions);
    expect(useMobileStore.getState().sessions.map((session) => session.is_active)).toEqual([true, false, true]);
    expect(useMobileStore.getState().knownMachines).toEqual([
      { machine_id: zoo006.machine_id, hostname: "zoo-006", connected: true },
    ]);
    expect(socket.sent.filter((frame) => frame.type === "attach_session").map((frame) => frame.session_id))
      .toEqual([openSession.id, notRecordedSession.id]);

    const deliver = (frame: ServerToWeb) => {
      socket.receive(frame);
      expect(socket.closed).toBe(false);
      expect(useMobileStore.getState().connection).toBe("ready");
      expect(useMobileStore.getState().error).toBeNull();
    };
    const activity = () => Object.fromEntries(useMobileStore.getState().sessions.map((session) => [session.id, session.is_active]));

    // Lists every viewer is sent: nothing in this app reads them, and the
    // new fields on their entries change nothing.
    const before = useMobileStore.getState();
    deliver(machinesFrame);
    deliver(sessionsFrame);
    await settle();
    expect(useMobileStore.getState().sessions).toBe(before.sessions);
    expect(useMobileStore.getState().knownMachines).toBe(before.knownMachines);

    // The project runs on both machines.
    deliver(bothMachines);
    deliver(paneListFrame);
    expect(useMobileStore.getState().instancesBySession[openSession.id]).toEqual([zoo005, zoo006]);

    // A close: each machine's instance leaves, then nothing is attached.
    deliver({ type: "session_instances", session_id: openSession.id, instances: [zoo005] });
    // zoo-006's tab is refused in the words for a machine whose daemon is
    // connected, as the bootstrap listed it.
    expect(tabUnavailableReason(zoo006Pane, useMobileStore.getState().instancesBySession[openSession.id], true, useMobileStore.getState().knownMachines))
      .toBe("zoo-006 is not running this project. Open the project again to start it there.");
    deliver({ type: "session_instances", session_id: openSession.id, instances: [] });
    deliver({ type: "session_attached", session_id: openSession.id, has_active_cli: false });
    deliver({
      type: "sessions",
      sessions: [{ ...openSession, is_active: false, open_state: "closed" }, closedSession, notRecordedSession],
    });
    await settle();
    expect(activity()).toEqual({ [openSession.id]: false, [closedSession.id]: false, [notRecordedSession.id]: true });
    expect(useMobileStore.getState().instancesBySession[openSession.id]).toEqual([]);
    // The roster is kept: closing removes no tab.
    expect(useMobileStore.getState().panesBySession[openSession.id]).toEqual(paneListFrame.panes);

    // An open: the tab owners attach again, one after the other.
    deliver({ type: "session_attached", session_id: openSession.id, has_active_cli: true });
    deliver({ type: "session_instances", session_id: openSession.id, instances: [zoo005] });
    deliver(bothMachines);
    deliver(paneListFrame);
    deliver(sessionsFrame);
    deliver(machinesFrame);
    await settle();
    expect(activity()).toEqual({ [openSession.id]: true, [closedSession.id]: false, [notRecordedSession.id]: true });
    expect(useMobileStore.getState().instancesBySession[openSession.id]).toEqual([zoo005, zoo006]);
    expect(tabUnavailableReason(zoo006Pane, useMobileStore.getState().instancesBySession[openSession.id], true, useMobileStore.getState().knownMachines))
      .toBeNull();

    // None of it became a conversation event, and one socket carried it all.
    expect(useMobileStore.getState().eventsBySession).toEqual({});
    expect(acceptEvents).not.toHaveBeenCalled();
    expect(FakeWebSocket.instances).toHaveLength(1);
    expect(socket.sent.some((frame) => /open|close|start_machine|stop_machine/.test(frame.type))).toBe(false);
  });

  it("reads past an open-state frame it did not ask for", async () => {
    // The server sends the frame only to viewers that negotiated it. This
    // build's schema lists it, so one that arrived anyway would neither drop
    // the socket nor reach the store.
    startConnectionRuntime();
    const [socket] = FakeWebSocket.instances;
    socket.open();
    await settle();
    socket.receive({ type: "authenticated", user_id: bootstrapValue.user_id, mutations_allowed: true, negotiated_capabilities: [] });
    await settle();
    const before = useMobileStore.getState();

    for (const name of [
      "server-project-open-state.json",
      "server-project-open-state-closed.json",
      "server-project-open-state-not-recorded.json",
      "server-project-open-state-refused.json",
    ]) {
      socket.receive(fixture<"project_open_state">(name));
    }
    await settle();
    expect(socket.closed).toBe(false);
    const after = useMobileStore.getState();
    expect(after.connection).toBe("ready");
    expect(after.sessions).toBe(before.sessions);
    expect(after.eventsBySession).toBe(before.eventsBySession);
    expect(after.error).toBeNull();
  });
});
