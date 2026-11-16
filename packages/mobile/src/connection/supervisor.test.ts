import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { MobileBootstrapResponse, ServerToWeb } from "@apas/protocol";

import { MOBILE_CAPABILITIES } from "@/config/endpoints";
import { ConnectionSupervisor, MutationRefusedError, type SocketLike } from "./supervisor";

// The server frames added for multi-machine project instances, as the shared
// crate's golden fixtures spell them.
const PROJECT_INSTANCE_FRAMES = [
  "server-session-instances.json",
  "server-session-instances-legacy.json",
  "server-pane-list-instances.json",
  "server-sessions-instances.json",
  "server-machines-instances.json",
  "server-cli-lifecycle-inventory-instance.json",
  "server-cli-lifecycle-status-instance.json",
  "server-add-pane-starting.json",
  "server-add-pane-failed.json",
  // Sent only to a connection that negotiated project instances; an older
  // bundled schema, without the "input" kind, would drop the socket.
  "server-mutation-ack-input.json",
];

function fixture(name: string): ServerToWeb {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8")) as ServerToWeb;
}

class FakeSocket implements SocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  sent: string[] = [];
  closed = false;
  send(data: string) { this.sent.push(data); }
  close() { this.closed = true; this.readyState = 3; }
  open() { this.readyState = 1; this.onopen?.(); }
  receive(value: object) { this.onmessage?.({ data: JSON.stringify(value) }); }
}

const emptyBootstrap = {
  user_id: "1027ac43-3c54-467a-90e6-e50a170d4882",
  user_email: "mobile@example.test",
  cluster_role: "user",
  account_status: "active",
  protocol_min_version: 1,
  protocol_max_version: 1,
  features: {},
  sessions: [],
  machines: [],
  launch_targets: [],
} satisfies MobileBootstrapResponse;

describe("ConnectionSupervisor", () => {
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => jest.useRealTimers());

  it("connects, authenticates, synchronizes, and becomes ready", async () => {
    const sockets: FakeSocket[] = [];
    const phases: string[] = [];
    const setNegotiatedCapabilities = jest.fn();
    const onSynchronized = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: (phase) => phases.push(phase),
      setMutationsAllowed: jest.fn(),
      setNegotiatedCapabilities,
      applyBootstrap: jest.fn(),
      onSynchronized,
      random: () => 0.5,
    });
    supervisor.start();
    sockets[0].open();
    await Promise.resolve();
    expect(JSON.parse(sockets[0].sent[0])).toMatchObject({ type: "authenticate", client_kind: "mobile" });
    sockets[0].receive({
      type: "authenticated",
      user_id: emptyBootstrap.user_id,
      mutations_allowed: true,
      negotiated_capabilities: ["terminal", "pane_work_summary_v1"],
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(phases).toEqual(expect.arrayContaining(["connecting", "authenticating", "synchronizing", "ready"]));
    expect(setNegotiatedCapabilities).toHaveBeenCalledWith(["terminal", "pane_work_summary_v1"]);
    expect(onSynchronized).toHaveBeenCalledTimes(1);
  });

  it("advertises project instances and keeps the socket open for every frame they bring", async () => {
    const sockets: FakeSocket[] = [];
    const onMessage = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
      onMessage,
      random: () => 0.5,
    });
    supervisor.start();
    sockets[0].open();
    await Promise.resolve();
    expect(JSON.parse(sockets[0].sent[0]).capabilities).toContain("project_instances_v1");
    expect(JSON.parse(sockets[0].sent[0]).capabilities).toContain("autonomous_workers_v1");
    sockets[0].receive({
      type: "authenticated",
      user_id: emptyBootstrap.user_id,
      mutations_allowed: true,
      negotiated_capabilities: ["project_instances_v1"],
    });
    await Promise.resolve();
    await Promise.resolve();

    const frames = PROJECT_INSTANCE_FRAMES.map(fixture);
    for (const frame of frames) sockets[0].receive(frame);
    expect(onMessage.mock.calls.map(([message]) => message)).toEqual(frames);
    expect(sockets[0].closed).toBe(false);

    // The hazard the server's capability gate exists for: a frame the bundled
    // schema does not list drops the socket.
    sockets[0].receive({ type: "session_instances_v2", session_id: emptyBootstrap.user_id });
    expect(sockets[0].closed).toBe(true);
    expect(onMessage).toHaveBeenCalledTimes(frames.length);
  });

  it("asks for no open-state frame and stays connected while a project is closed and opened", async () => {
    // This app shows no open state and has no control for it, so it leaves
    // project_open_state_v1 out. The server then sends it no frame of that
    // kind; what an open and a close do reach it as the frames it already
    // reads, some of whose entries carry the new optional fields.
    const sessionsFrame = fixture("server-sessions-open-state.json") as Extract<ServerToWeb, { type: "sessions" }>;
    const machinesFrame = fixture("server-machines-open-state.json") as Extract<ServerToWeb, { type: "machines" }>;
    const instancesFrame = fixture("server-session-instances.json") as Extract<ServerToWeb, { type: "session_instances" }>;
    const sessionId = instancesFrame.session_id;
    expect(sessionsFrame.sessions.map((session) => session.open_state)).toEqual(["open", "closed", undefined]);
    expect(machinesFrame.machines[0].projects[0]).toMatchObject({ open_state: "open", part: "not_started" });
    expect(machinesFrame.machines[0].projects[0].part_reason).toBeTruthy();
    expect(machinesFrame.machines[0].projects[0].other_hosts).toHaveLength(2);
    // The bootstrap's session entries flatten the same entry, and its machine
    // list is the same list.
    const bootstrapValue: MobileBootstrapResponse = {
      ...emptyBootstrap,
      sessions: sessionsFrame.sessions.map((session) => ({ ...session, project_name: "project", attention_count: 0 })),
      machines: machinesFrame.machines,
    };

    const sockets: FakeSocket[] = [];
    const phases: string[] = [];
    const onMessage = jest.fn();
    const applyBootstrap = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => bootstrapValue,
      persistBootstrap: async () => undefined,
      setPhase: (phase) => phases.push(phase),
      setMutationsAllowed: jest.fn(),
      applyBootstrap,
      onMessage,
      random: () => 0.5,
    });
    supervisor.start();
    sockets[0].open();
    await Promise.resolve();
    expect(MOBILE_CAPABILITIES).not.toContain("project_open_state_v1");
    expect(JSON.parse(sockets[0].sent[0]).capabilities).toEqual([...MOBILE_CAPABILITIES]);
    expect(JSON.parse(sockets[0].sent[0]).capabilities).not.toContain("project_open_state_v1");
    sockets[0].receive({
      type: "authenticated",
      user_id: emptyBootstrap.user_id,
      mutations_allowed: true,
      negotiated_capabilities: ["project_instances_v1"],
    });
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(applyBootstrap).toHaveBeenCalledWith(bootstrapValue);
    expect(phases.at(-1)).toBe("ready");
    const attached = sockets[0].sent.map((value) => JSON.parse(value)).filter((value) => value.type === "attach_session");
    // Attachment still follows what runs, not the recorded state: the closed
    // project is not attached, and the open and the not-recorded one are.
    expect(attached.map((value) => value.session_id)).toEqual(
      sessionsFrame.sessions.filter((session) => session.is_active).map((session) => session.id),
    );

    const [zoo005, zoo006] = instancesFrame.instances;
    const frames: ServerToWeb[] = [
      // Sent to every viewer once it has authenticated.
      machinesFrame,
      sessionsFrame,
      // A close: each machine's instance leaves, then nothing is attached.
      { type: "session_instances", session_id: sessionId, instances: [zoo005] },
      { type: "session_instances", session_id: sessionId, instances: [] },
      { type: "session_attached", session_id: sessionId, has_active_cli: false },
      { type: "sessions", sessions: sessionsFrame.sessions.map((session) => session.id === sessionId ? { ...session, is_active: false, open_state: "closed" } : session) },
      // An open: the tab owners attach again, one after the other.
      { type: "session_attached", session_id: sessionId, has_active_cli: true },
      { type: "session_instances", session_id: sessionId, instances: [zoo005] },
      { type: "session_instances", session_id: sessionId, instances: [zoo005, zoo006] },
      fixture("server-pane-list-instances.json"),
      sessionsFrame,
      machinesFrame,
    ];
    for (const frame of frames) sockets[0].receive(frame);
    expect(onMessage.mock.calls.map(([message]) => message)).toEqual(frames);
    expect(onMessage.mock.calls.some(([message]) => (message as ServerToWeb).type === "project_open_state")).toBe(false);
    expect(sockets[0].closed).toBe(false);
    expect(sockets).toHaveLength(1);
    expect(phases.at(-1)).toBe("ready");
    supervisor.stop();
  });

  it("carries negotiated worker read subscriptions without acknowledged mutation replay", async () => {
    const socket = new FakeSocket();
    const onMessage = jest.fn();
    const mutations = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket, accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap, persistBootstrap: async () => undefined,
      setPhase: jest.fn(), setMutationsAllowed: mutations, applyBootstrap: jest.fn(), onMessage,
    });
    supervisor.start(); socket.open();
    await Promise.resolve();
    socket.receive({ type: "authenticated", user_id: emptyBootstrap.user_id, mutations_allowed: false, negotiated_capabilities: ["autonomous_workers_v1"] });
    await Promise.resolve(); await Promise.resolve();
    const subscription = { type: "subscribe_worker" as const, project_id: "project", worker_id: emptyBootstrap.user_id, after_seq: 7 };
    expect(supervisor.send(subscription)).toBe(true);
    socket.receive({ type: "worker_inbox_changed" });
    expect(onMessage).toHaveBeenCalledWith({ type: "worker_inbox_changed" });
    expect(mutations).toHaveBeenLastCalledWith(false);
    expect(socket.sent.map((value) => JSON.parse(value)).filter((value) => value.type === "subscribe_worker")).toEqual([subscription]);
    supervisor.stop();
  });

  it("makes the socket unusable immediately in the background", () => {
    const socket = new FakeSocket();
    const mutations = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: mutations,
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    supervisor.setForeground(false);
    expect(socket.closed).toBe(true);
    expect(supervisor.send({ type: "heartbeat" })).toBe(false);
    expect(mutations).toHaveBeenLastCalledWith(false);
  });

  it("reconnects with bounded jitter after a failure", () => {
    const sockets: FakeSocket[] = [];
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
      random: () => 0.5,
    });
    supervisor.start();
    sockets[0].onerror?.();
    jest.advanceTimersByTime(999);
    expect(sockets).toHaveLength(1);
    jest.advanceTimersByTime(1);
    expect(sockets).toHaveLength(2);
  });

  it("retries an idempotent mutation and resolves only on its acknowledgement", async () => {
    const socket = new FakeSocket();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    const requestId = "mutation-1";
    const acknowledged = supervisor.sendAcknowledged({
      type: "interrupt_pane",
      session_id: emptyBootstrap.user_id,
      pane_id: 3,
      request_id: requestId,
    }, requestId);
    const sendsAfterFirstAttempt = socket.sent.length;
    jest.advanceTimersByTime(3_000);
    expect(socket.sent).toHaveLength(sendsAfterFirstAttempt + 1);
    socket.receive({
      type: "mutation_ack",
      request_id: requestId,
      session_id: emptyBootstrap.user_id,
      pane_id: 3,
      mutation: "interrupt",
      accepted: true,
    });
    await expect(acknowledged).resolves.toMatchObject({ type: "mutation_ack", accepted: true });
  });

  it("surfaces a stale decision rejection without claiming success", async () => {
    const socket = new FakeSocket();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    const requestId = "decision-1";
    const acknowledged = supervisor.sendAcknowledged({
      type: "plan_review_answer",
      session_id: emptyBootstrap.user_id,
      pane_id: 3,
      tool_use_id: "tool-1",
      approve: true,
      request_id: requestId,
    }, requestId);
    socket.receive({
      type: "mutation_ack",
      request_id: requestId,
      session_id: emptyBootstrap.user_id,
      pane_id: 3,
      mutation: "plan_review",
      accepted: false,
      error: "already resolved",
    });
    await expect(acknowledged).rejects.toThrow("already resolved");
  });

  it("settles typed input the server refused with the server's reason, not a retry", async () => {
    const socket = new FakeSocket();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    const refusal = fixture("server-mutation-ack-input.json");
    if (refusal.type !== "mutation_ack") throw new Error("unexpected fixture");
    // Accepted input is acknowledged by its user_input echo; a refusal by a
    // mutation_ack carrying the input's client_msg_id.
    const acknowledged = supervisor.sendAcknowledged({
      type: "input",
      session_id: refusal.session_id,
      pane_id: refusal.pane_id,
      text: "Run the suite.",
      client_msg_id: refusal.request_id,
    }, refusal.request_id);
    const sends = socket.sent.length;
    socket.receive(refusal);
    await expect(acknowledged).rejects.toThrow(refusal.error!);
    // A refusal, not a lost acknowledgement: its id is spent.
    await expect(acknowledged).rejects.toBeInstanceOf(MutationRefusedError);
    // Settled: the timer that would resend it is gone with it.
    jest.advanceTimersByTime(10_000);
    expect(socket.sent).toHaveLength(sends);
    expect(socket.closed).toBe(false);
  });

  it("tells a missing acknowledgement apart from a refusal", async () => {
    const socket = new FakeSocket();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    const acknowledged = supervisor.sendAcknowledged({
      type: "input",
      session_id: emptyBootstrap.user_id,
      pane_id: 3,
      text: "Run the suite.",
      client_msg_id: "msg-unanswered",
    }, "msg-unanswered");
    const rejected = acknowledged.catch((error: unknown) => error);
    jest.advanceTimersByTime(9_000);
    const error = await rejected;
    // Its outcome is unknown, so a retry keeps the same id.
    expect(error).toBeInstanceOf(Error);
    expect(error).not.toBeInstanceOf(MutationRefusedError);
  });

  it("drops a silent socket after the heartbeat response deadline", async () => {
    const sockets: FakeSocket[] = [];
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
      random: () => 0.5,
    });
    supervisor.start();
    sockets[0].open();
    await Promise.resolve();
    sockets[0].receive({ type: "authenticated", user_id: emptyBootstrap.user_id, mutations_allowed: true });
    await Promise.resolve();
    await Promise.resolve();
    jest.advanceTimersByTime(25_000);
    expect(JSON.parse(sockets[0].sent.at(-1) ?? "{}")).toMatchObject({ type: "heartbeat" });
    jest.advanceTimersByTime(15_000);
    expect(sockets[0].closed).toBe(true);
    jest.advanceTimersByTime(1_000);
    expect(sockets).toHaveLength(2);
  });

  it("refreshes an expired access token before continuing authentication", async () => {
    const socket = new FakeSocket();
    const accessToken = jest.fn(async (force?: boolean) => force ? "refreshed" : "expired");
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken,
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    socket.receive({ type: "authentication_failed", reason: "expired" });
    await Promise.resolve();
    expect(accessToken).toHaveBeenLastCalledWith(true);
    expect(JSON.parse(socket.sent.at(-1) ?? "{}")).toMatchObject({ type: "authenticate", token: "refreshed" });
  });

  it("does not multiply retries during a reconnect storm", () => {
    const sockets: FakeSocket[] = [];
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
      random: () => 0.5,
    });
    supervisor.start();
    const failure = sockets[0].onerror;
    failure?.();
    failure?.();
    jest.advanceTimersByTime(1_000);
    expect(sockets).toHaveLength(2);
  });

  it("ignores a bootstrap result that races suspension", async () => {
    const socket = new FakeSocket();
    let resolveBootstrap: ((value: MobileBootstrapResponse) => void) | undefined;
    const bootstrap = new Promise<MobileBootstrapResponse>((resolve) => { resolveBootstrap = resolve; });
    const applyBootstrap = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: () => bootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap,
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    socket.receive({ type: "authenticated", user_id: emptyBootstrap.user_id, mutations_allowed: true });
    await Promise.resolve();
    supervisor.setForeground(false);
    resolveBootstrap?.(emptyBootstrap);
    await Promise.resolve();
    await Promise.resolve();
    expect(applyBootstrap).not.toHaveBeenCalled();
  });

  it("refreshes authorization after a project access change", async () => {
    const socket = new FakeSocket();
    const bootstrap = jest.fn(async () => emptyBootstrap);
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    socket.receive({ type: "authenticated", user_id: emptyBootstrap.user_id, mutations_allowed: true });
    await Promise.resolve();
    await Promise.resolve();
    socket.receive({ type: "project_access_changed", project_id: "gone", change: "revoked" });
    await Promise.resolve();
    await Promise.resolve();
    expect(bootstrap).toHaveBeenCalledTimes(2);
  });

  it("keeps a protocol downgrade read-only after synchronization", async () => {
    const socket = new FakeSocket();
    const mutations = jest.fn();
    const setNegotiatedCapabilities = jest.fn();
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: mutations,
      setNegotiatedCapabilities,
      applyBootstrap: jest.fn(),
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    socket.receive({ type: "protocol_incompatible", minimum_version: 2, maximum_version: 3, read_only: true, message: "Upgrade" });
    await Promise.resolve();
    await Promise.resolve();
    expect(mutations).toHaveBeenLastCalledWith(false);
    expect(setNegotiatedCapabilities).toHaveBeenLastCalledWith([]);
  });

  it("wipes authentication state when refresh proves the device is revoked", async () => {
    const socket = new FakeSocket();
    const onAuthenticationLost = jest.fn();
    const revoked = new Error("revoked");
    const supervisor = new ConnectionSupervisor({
      createSocket: () => socket,
      accessToken: async () => { throw revoked; },
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: jest.fn(),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
      isAuthenticationLoss: (error) => error === revoked,
      onAuthenticationLost,
    });
    supervisor.start();
    socket.open();
    await Promise.resolve();
    await Promise.resolve();
    expect(onAuthenticationLost).toHaveBeenCalledTimes(1);
    expect(socket.closed).toBe(true);
  });
});
