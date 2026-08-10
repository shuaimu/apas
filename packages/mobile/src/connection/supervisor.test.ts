import type { MobileBootstrapResponse } from "@apas/protocol";

import { ConnectionSupervisor, type SocketLike } from "./supervisor";

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
    const supervisor = new ConnectionSupervisor({
      createSocket: () => { const socket = new FakeSocket(); sockets.push(socket); return socket; },
      accessToken: async () => "access",
      bootstrap: async () => emptyBootstrap,
      persistBootstrap: async () => undefined,
      setPhase: (phase) => phases.push(phase),
      setMutationsAllowed: jest.fn(),
      applyBootstrap: jest.fn(),
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
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(phases).toEqual(expect.arrayContaining(["connecting", "authenticating", "synchronizing", "ready"]));
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
    socket.open();
    await Promise.resolve();
    socket.receive({ type: "protocol_incompatible", minimum_version: 2, maximum_version: 3, read_only: true, message: "Upgrade" });
    await Promise.resolve();
    await Promise.resolve();
    expect(mutations).toHaveBeenLastCalledWith(false);
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
