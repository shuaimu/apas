import type { MobileBootstrapResponse, ServerToWeb, WebToServer } from "@apas/protocol";
import { validateServerMessage } from "@apas/protocol";

import { MOBILE_CAPABILITIES, MOBILE_PROTOCOL_VERSION } from "@/config/endpoints";
import type { ConnectionPhase } from "@/state/store";

export interface SocketLike {
  readyState: number;
  onopen: (() => void) | null;
  onmessage: ((event: { data: string }) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
  send(data: string): void;
  close(): void;
}

export interface ConnectionDependencies {
  createSocket: () => SocketLike;
  accessToken: (forceRefresh?: boolean) => Promise<string>;
  bootstrap: () => Promise<MobileBootstrapResponse>;
  persistBootstrap: (value: MobileBootstrapResponse) => Promise<void>;
  setPhase: (phase: ConnectionPhase) => void;
  setMutationsAllowed: (allowed: boolean) => void;
  setNegotiatedCapabilities?: (capabilities: string[]) => void;
  applyBootstrap: (value: MobileBootstrapResponse) => void;
  onSynchronized?: () => void;
  onMessage?: (message: ServerToWeb) => void;
  isAuthenticationLoss?: (error: unknown) => boolean;
  onAuthenticationLost?: () => void;
  random?: () => number;
  setTimeout?: typeof globalThis.setTimeout;
  clearTimeout?: typeof globalThis.clearTimeout;
}

const SOCKET_OPEN = 1;
const HEARTBEAT_INTERVAL_MS = 25_000;
const HEARTBEAT_TIMEOUT_MS = 15_000;
const MAX_RETRY_MS = 30_000;
const MUTATION_ACK_TIMEOUT_MS = 3_000;
const MUTATION_ATTEMPTS = 3;

interface PendingAcknowledgement {
  message: WebToServer;
  attempts: number;
  timer: ReturnType<typeof globalThis.setTimeout>;
  resolve: (message: ServerToWeb) => void;
  reject: (error: Error) => void;
}

export class ConnectionSupervisor {
  private socket: SocketLike | null = null;
  private generation = 0;
  private attempts = 0;
  private foreground = true;
  private online = true;
  private running = false;
  private retryTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private heartbeatTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  private heartbeatTimeout: ReturnType<typeof globalThis.setTimeout> | null = null;
  private readonly pendingAcknowledgements = new Map<string, PendingAcknowledgement>();
  private readonly random: () => number;
  private readonly setTimer: typeof globalThis.setTimeout;
  private readonly clearTimer: typeof globalThis.clearTimeout;

  constructor(private readonly dependencies: ConnectionDependencies) {
    this.random = dependencies.random ?? Math.random;
    this.setTimer = dependencies.setTimeout ?? globalThis.setTimeout;
    this.clearTimer = dependencies.clearTimeout ?? globalThis.clearTimeout;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.connect();
  }

  stop(): void {
    this.running = false;
    this.generation += 1;
    this.clearTimers();
    this.closeSocket();
    this.rejectPendingAcknowledgements("The connection closed before the server acknowledged the action");
    this.dependencies.setMutationsAllowed(false);
    this.dependencies.setPhase("offline");
  }

  setNetworkOnline(online: boolean): void {
    this.online = online;
    this.reconcileAvailability();
  }

  setForeground(foreground: boolean): void {
    this.foreground = foreground;
    this.reconcileAvailability();
  }

  send(message: WebToServer): boolean {
    if (this.socket?.readyState !== SOCKET_OPEN) return false;
    this.socket.send(JSON.stringify(message));
    return true;
  }

  sendAcknowledged(message: WebToServer, requestId: string): Promise<ServerToWeb> {
    if (this.pendingAcknowledgements.has(requestId)) {
      return Promise.reject(new Error("This action is already awaiting acknowledgement"));
    }
    if (this.socket?.readyState !== SOCKET_OPEN) {
      return Promise.reject(new Error("Reconnect before sending this action"));
    }
    return new Promise((resolve, reject) => {
      const timer = this.setTimer(() => this.retryAcknowledgement(requestId), MUTATION_ACK_TIMEOUT_MS);
      this.pendingAcknowledgements.set(requestId, {
        message,
        attempts: 1,
        timer,
        resolve,
        reject,
      });
      this.socket?.send(JSON.stringify(message));
    });
  }

  private reconcileAvailability(): void {
    if (!this.running) return;
    if (!this.online || !this.foreground) {
      this.generation += 1;
      this.clearTimers();
      this.closeSocket();
      this.rejectPendingAcknowledgements("The app went offline before the server acknowledged the action");
      this.dependencies.setMutationsAllowed(false);
      this.dependencies.setPhase("offline");
      return;
    }
    if (!this.socket && !this.retryTimer) {
      this.attempts = 0;
      this.connect();
    }
  }

  private connect(): void {
    if (!this.running || !this.online || !this.foreground || this.socket) return;
    const generation = ++this.generation;
    this.dependencies.setMutationsAllowed(false);
    this.dependencies.setPhase("connecting");
    const socket = this.dependencies.createSocket();
    this.socket = socket;
    socket.onopen = () => void this.authenticate(generation, socket);
    socket.onmessage = (event) => void this.handleMessage(generation, socket, event.data);
    socket.onerror = () => this.failSocket(generation, socket);
    socket.onclose = () => this.failSocket(generation, socket);
  }

  private async authenticate(generation: number, socket: SocketLike): Promise<void> {
    if (!this.current(generation, socket)) return;
    this.dependencies.setPhase("authenticating");
    try {
      const token = await this.dependencies.accessToken(false);
      if (!this.current(generation, socket)) return;
      const auth: WebToServer = {
        type: "authenticate",
        token,
        client_kind: "mobile",
        app_version: "0.1.0",
        protocol_version: MOBILE_PROTOCOL_VERSION,
        capabilities: [...MOBILE_CAPABILITIES],
      };
      socket.send(JSON.stringify(auth));
      this.armHeartbeat(generation, socket);
    } catch (error) {
      if (this.dependencies.isAuthenticationLoss?.(error)) {
        this.dependencies.onAuthenticationLost?.();
        this.stop();
        return;
      }
      this.failSocket(generation, socket);
    }
  }

  private async handleMessage(
    generation: number,
    socket: SocketLike,
    rawData: string,
  ): Promise<void> {
    if (!this.current(generation, socket)) return;
    this.touchHeartbeat(generation, socket);
    let message: ServerToWeb;
    try {
      const parsed: unknown = JSON.parse(rawData);
      if (!validateServerMessage(parsed).valid) throw new Error("Invalid server message");
      message = parsed as ServerToWeb;
    } catch {
      this.failSocket(generation, socket);
      return;
    }

    if (message.type === "authenticated") {
      this.dependencies.setMutationsAllowed(message.mutations_allowed !== false);
      this.dependencies.setNegotiatedCapabilities?.(message.negotiated_capabilities ?? []);
      await this.synchronize(generation, socket);
      return;
    }
    if (message.type === "protocol_incompatible") {
      this.dependencies.setMutationsAllowed(false);
      this.dependencies.setNegotiatedCapabilities?.([]);
      await this.synchronize(generation, socket);
      return;
    }
    if (message.type === "authentication_failed") {
      try {
        const token = await this.dependencies.accessToken(true);
        if (!this.current(generation, socket)) return;
        socket.send(
          JSON.stringify({
            type: "authenticate",
            token,
            client_kind: "mobile",
            app_version: "0.1.0",
            protocol_version: MOBILE_PROTOCOL_VERSION,
            capabilities: [...MOBILE_CAPABILITIES],
          } satisfies WebToServer),
        );
      } catch (error) {
        if (this.dependencies.isAuthenticationLoss?.(error)) {
          this.dependencies.onAuthenticationLost?.();
          this.stop();
          return;
        }
        this.failSocket(generation, socket);
      }
      return;
    }
    if (message.type === "project_access_changed") {
      await this.synchronize(generation, socket);
    }
    const acknowledgementId = message.type === "mutation_ack"
      ? message.request_id
      : message.type === "user_input"
        ? message.client_msg_id
        : null;
    if (acknowledgementId) {
      const pending = this.pendingAcknowledgements.get(acknowledgementId);
      if (pending) {
        this.clearTimer(pending.timer);
        this.pendingAcknowledgements.delete(acknowledgementId);
        if (message.type === "mutation_ack" && !message.accepted) {
          pending.reject(new Error(message.error ?? "The server rejected this action"));
        } else {
          pending.resolve(message);
        }
      }
    }
    this.dependencies.onMessage?.(message);
  }

  private async synchronize(generation: number, socket: SocketLike): Promise<void> {
    if (!this.current(generation, socket)) return;
    this.dependencies.setPhase("synchronizing");
    try {
      const value = await this.dependencies.bootstrap();
      if (!this.current(generation, socket)) return;
      await this.dependencies.persistBootstrap(value);
      if (!this.current(generation, socket)) return;
      this.dependencies.applyBootstrap(value);
      for (const session of value.sessions.filter((item) => item.is_active)) {
        socket.send(JSON.stringify({ type: "attach_session", session_id: session.id } satisfies WebToServer));
      }
      this.attempts = 0;
      this.dependencies.setPhase("ready");
      this.dependencies.onSynchronized?.();
    } catch (error) {
      if (this.dependencies.isAuthenticationLoss?.(error)) {
        this.dependencies.onAuthenticationLost?.();
        this.stop();
        return;
      }
      this.failSocket(generation, socket);
    }
  }

  private armHeartbeat(generation: number, socket: SocketLike): void {
    if (!this.current(generation, socket)) return;
    if (this.heartbeatTimer) this.clearTimer(this.heartbeatTimer);
    this.heartbeatTimer = this.setTimer(() => {
      if (!this.current(generation, socket)) return;
      socket.send(JSON.stringify({ type: "heartbeat" } satisfies WebToServer));
      this.heartbeatTimeout = this.setTimer(
        () => this.failSocket(generation, socket),
        HEARTBEAT_TIMEOUT_MS,
      );
    }, HEARTBEAT_INTERVAL_MS);
  }

  private touchHeartbeat(generation: number, socket: SocketLike): void {
    if (this.heartbeatTimeout) {
      this.clearTimer(this.heartbeatTimeout);
      this.heartbeatTimeout = null;
    }
    this.armHeartbeat(generation, socket);
  }

  private failSocket(generation: number, socket: SocketLike): void {
    if (!this.current(generation, socket)) return;
    this.generation += 1;
    this.clearTimers();
    this.closeSocket();
    this.rejectPendingAcknowledgements("The connection was interrupted before acknowledgement; retry safely with the same action");
    this.dependencies.setMutationsAllowed(false);
    if (!this.running || !this.online || !this.foreground) {
      this.dependencies.setPhase("offline");
      return;
    }
    this.dependencies.setPhase("connecting");
    const base = Math.min(1000 * 2 ** this.attempts++, MAX_RETRY_MS);
    const delay = Math.round(base * (0.75 + this.random() * 0.5));
    this.retryTimer = this.setTimer(() => {
      this.retryTimer = null;
      this.connect();
    }, delay);
  }

  private retryAcknowledgement(requestId: string): void {
    const pending = this.pendingAcknowledgements.get(requestId);
    if (!pending) return;
    if (this.socket?.readyState !== SOCKET_OPEN) {
      this.pendingAcknowledgements.delete(requestId);
      pending.reject(new Error("The connection was interrupted before acknowledgement"));
      return;
    }
    if (pending.attempts >= MUTATION_ATTEMPTS) {
      this.pendingAcknowledgements.delete(requestId);
      pending.reject(new Error("The server did not acknowledge the action; retry with the same action identifier"));
      return;
    }
    pending.attempts += 1;
    this.socket.send(JSON.stringify(pending.message));
    pending.timer = this.setTimer(
      () => this.retryAcknowledgement(requestId),
      MUTATION_ACK_TIMEOUT_MS,
    );
  }

  private rejectPendingAcknowledgements(message: string): void {
    for (const pending of this.pendingAcknowledgements.values()) {
      this.clearTimer(pending.timer);
      pending.reject(new Error(message));
    }
    this.pendingAcknowledgements.clear();
  }

  private current(generation: number, socket: SocketLike): boolean {
    return this.generation === generation && this.socket === socket;
  }

  private closeSocket(): void {
    const socket = this.socket;
    this.socket = null;
    if (socket) {
      socket.onclose = null;
      socket.onerror = null;
      socket.close();
    }
  }

  private clearTimers(): void {
    for (const timer of [this.retryTimer, this.heartbeatTimer, this.heartbeatTimeout]) {
      if (timer) this.clearTimer(timer);
    }
    this.retryTimer = null;
    this.heartbeatTimer = null;
    this.heartbeatTimeout = null;
  }
}
