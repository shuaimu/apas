import { create } from "zustand";
import {
  deleteSnapshot as deleteSnapshotIdb,
  loadAllSnapshots as loadAllSnapshotsIdb,
  saveSnapshot as saveSnapshotIdb,
} from "./sessionCacheDb";

// UUID generator with fallback for environments without crypto.randomUUID
function generateId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  // Fallback UUID v4 generator
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = Math.random() * 16 | 0;
    const v = c === 'x' ? r : (r & 0x3 | 0x8);
    return v.toString(16);
  });
}

export interface Message {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: Date;
  outputType?: OutputType;
}

export interface CliClient {
  id: string;
  name?: string;
  status: "online" | "offline" | "busy";
  version?: string;
  lastSeen?: string;
  activeSession?: string;
}

export interface SessionInfo {
  id: string;
  /** Stable project identity from `.apas`. Sidebar groups by this so moving
   * the project directory doesn't create a duplicate entry. Falls back to `id`
   * for legacy rows that pre-date the column. */
  projectId?: string;
  cliClientId?: string;
  workingDir?: string;
  hostname?: string;
  status: string;
  createdAt?: string;
  isShared?: boolean;
  ownerEmail?: string;
  shareRole?: "owner" | "admin" | "user";
  isActive?: boolean;
}

export interface UsageLimitWindow {
  utilization: number; // 0.0 to 1.0+
  resetsAt?: string; // ISO 8601 timestamp
}

export interface UsageLimits {
  fiveHour?: UsageLimitWindow;
  sevenDay?: UsageLimitWindow;
  fetchedAt?: string;
}

export type Provider = "claude" | "codex" | "minimax" | "glm" | "opencode" | "cursor-agent";

export type UsageLimitsByProvider = Partial<Record<Provider, UsageLimits>>;

export interface CliUsageLimits {
  cliClientId: string;
  limits: UsageLimits;
}

export interface MachineInfo {
  machineId: string;
  hostname: string;
  os: string;
  arch: string;
  daemonVersion?: string;
  minimaxBackend?: {
    apiBaseUrl?: string;
    apiKey?: string;
    apiKeyConfigured: boolean;
  };
  glmBackend?: {
    apiBaseUrl?: string;
    apiKey?: string;
    apiKeyConfigured: boolean;
  };
  lastSeen?: string;
}

export interface MachineProject {
  projectId: string;
  name?: string;
  path: string;
  isRunning: boolean;
  pid?: number;
  /** Resident-set size in KiB, reported by daemon in heartbeat. */
  memoryKb?: number;
  lastError?: string;
}

export interface MachineWithProjects {
  machine: MachineInfo;
  projects: MachineProject[];
}

// Map tool_use_id (e.g. "toolu_01Xwe...") to human-readable tool name (e.g. "Read", "Bash")
const toolNameMap = new Map<string, string>();

export type OutputType =
  | { type: "text" }
  | { type: "code"; language?: string }
  | { type: "tool_use"; tool: string; input: unknown; toolUseId?: string }
  | { type: "tool_result"; tool: string; success: boolean }
  | { type: "approval_request"; toolCallId: string; tool: string; description: string }
  | { type: "system" }
  | { type: "error" };

export type ToastKind = "success" | "info" | "error";

export interface Toast {
  id: string;
  kind: ToastKind;
  message: string;
}

/// Snapshot of the message-related state for a single session. Stored
/// in-memory keyed by session_id so tab switches restore last-seen
/// messages instantly while the fresh server fetch is still in flight.
export interface SessionCacheEntry {
  messages: Message[];
  paneMessages: Record<string, Message[]>;
  paneHasMore: Record<string, boolean>;
  paneConfigs: PaneConfig[];
  paneModes: Record<string, PaneType>;
  hasMoreMessages: boolean;
  isDualPane: boolean;
  answeredQuestions: Map<string, Record<string, string>>;
  cachedAt: number;
}

export type PaneType = "deadloop" | "interactive";

export interface PaneConfig {
  pane_id: number;
  provider: Provider;
  mode: "deadloop" | "interactive";
  session_id: string;
  is_paused: boolean;
  stop_requested?: boolean;
  prompt?: string;
  min_iteration_interval_minutes?: number;
  label?: string;
  model?: string;
  effort?: string;
}

// Legacy pane_id constants (must match shared::PANE_ID_DEADLOOP / PANE_ID_INTERACTIVE)
export const PANE_ID_DEADLOOP = 1;
export const PANE_ID_INTERACTIVE = 2;
const usageProviderHints = new Map<string, Provider>();

function normalizeProvider(raw: unknown): Provider | null {
  if (typeof raw === "string") {
    const normalized = raw.trim().toLowerCase();
    if (normalized === "minimax" || normalized === "mini_max") return "minimax";
    if (
      normalized === "claude" ||
      normalized === "anthropic" ||
      normalized === "claude-old" ||
      normalized === "claude_old"
    ) {
      // claude-old / claude_old are pre-streaming-switchover aliases — fold
      // them onto plain "claude" since the legacy --print interactive path
      // has been removed.
      return "claude";
    }
    if (normalized === "codex" || normalized === "openai" || normalized === "chatgpt") return "codex";
    if (normalized === "glm" || normalized === "zai" || normalized === "z.ai" || normalized === "zhipu") return "glm";
    if (normalized === "opencode") return "opencode";
    if (normalized === "cursor-agent" || normalized === "cursor_agent" || normalized === "cursor") return "cursor-agent";
  }
  return null;
}

// Map legacy pane_type string to numeric pane_id
function normalizePaneId(paneType: string | undefined, paneId: number | undefined): number | undefined {
  if (paneId != null) return paneId;
  if (!paneType) return undefined;
  const normalized = paneType.trim().toLowerCase();
  if (normalized === "deadloop" || normalized.includes("deadloop")) return PANE_ID_DEADLOOP;
  if (normalized === "interactive" || normalized.includes("interactive")) return PANE_ID_INTERACTIVE;
  // Try parsing as number (for stored messages with numeric string pane_type)
  const parsed = parseInt(normalized, 10);
  if (!isNaN(parsed)) return parsed;
  // Fallback: parse trailing numeric suffix from legacy composite formats.
  const suffixMatch = normalized.match(/(\d+)$/);
  if (suffixMatch) {
    const suffix = parseInt(suffixMatch[1], 10);
    if (!isNaN(suffix)) return suffix;
  }
  return undefined;
}

function normalizePaneModeHint(paneType: string | undefined): PaneType | undefined {
  if (!paneType) return undefined;
  const normalized = paneType.trim().toLowerCase();
  if (normalized === "deadloop" || normalized.includes("deadloop")) return "deadloop";
  if (normalized === "interactive" || normalized.includes("interactive")) return "interactive";
  return undefined;
}

function normalizeRawPaneId(rawPaneId: number | string | undefined): number | undefined {
  if (typeof rawPaneId === "number") {
    return isNaN(rawPaneId) ? undefined : rawPaneId;
  }
  if (typeof rawPaneId === "string") {
    const parsed = parseInt(rawPaneId, 10);
    return isNaN(parsed) ? undefined : parsed;
  }
  return undefined;
}

// Reverse map: numeric pane_id to legacy pane_type for wire compat
function legacyPaneType(paneId: number | undefined): string | undefined {
  if (paneId == null) return undefined;
  if (paneId === PANE_ID_DEADLOOP) return "deadloop";
  if (paneId === PANE_ID_INTERACTIVE) return "interactive";
  return undefined;
}

// Convert pane_id to string key for use in Record<string, ...>
export function paneKey(paneId: number): string {
  return String(paneId);
}

function collectPaneProviders(paneConfigs: PaneConfig[]): Set<Provider> {
  const seenProviders = new Set<Provider>();
  for (const pane of paneConfigs) {
    const normalized = normalizeProvider(pane.provider);
    if (normalized) seenProviders.add(normalized);
  }
  return seenProviders;
}

function inferUsageProvider(
  cliClientId: string,
  paneConfigs: PaneConfig[],
): Provider {
  const seenProviders = collectPaneProviders(paneConfigs);

  if (seenProviders.size === 1) {
    const provider = Array.from(seenProviders)[0];
    usageProviderHints.set(cliClientId, provider);
    return provider;
  }

  const hinted = usageProviderHints.get(cliClientId);
  if (hinted) return hinted;

  // Legacy servers may omit provider; in mixed-provider sessions those payloads
  // are typically Codex (last-write from periodic usage polling), so prefer Codex.
  if (seenProviders.size > 1) {
    if (seenProviders.has("codex")) {
      usageProviderHints.set(cliClientId, "codex");
      return "codex";
    }
    if (seenProviders.has("minimax")) {
      usageProviderHints.set(cliClientId, "minimax");
      return "minimax";
    }
    if (seenProviders.has("glm")) {
      usageProviderHints.set(cliClientId, "glm");
      return "glm";
    }
    usageProviderHints.set(cliClientId, "claude");
    return "claude";
  }

  return "claude";
}

interface AppState {
  // Auth state
  token: string | null;
  userId: string | null;
  userEmail: string | null;
  serverVersion: string | null;
  isAuthenticated: boolean;

  // Connection state
  connected: boolean;
  sessionId: string | null;
  cliClientId: string | null; // Current CLI client ID for per-project settings
  ws: WebSocket | null;
  refreshInterval: NodeJS.Timeout | null;
  isAttached: boolean; // Whether we're attached to an active session
  reconnectAttempts: number;
  reconnectTimeout: NodeJS.Timeout | null;
  visibilityHandler: (() => void) | null;

  // CLI clients
  cliClients: CliClient[];

  // Persisted sessions
  sessions: SessionInfo[];

  // Messages (single pane mode / fallback)
  messages: Message[];
  hasMoreMessages: boolean;
  isLoadingMore: boolean;

  // Dynamic pane state
  isDualPane: boolean;
  paneConfigs: PaneConfig[];
  paneMessages: Record<string, Message[]>;
  paneHasMore: Record<string, boolean>;
  paneStatuses: Record<string, string | null>;
  paneModes: Record<string, PaneType>;
  pausedPanes: number[]; // pane_ids that are paused
  loadingMorePane: number | null;

  // tool_use_id -> answers map. The AskUserQuestionCard reads this so the
  // form flips to the read-only submitted state immediately after the user
  // clicks Submit, instead of waiting for the round-trip tool_result.
  answeredQuestions: Map<string, Record<string, string>>;

  // Transient toast notifications surfaced at the top of the viewport.
  // Auto-dismiss is the Toaster component's job (timer on mount).
  toasts: Toast[];

  // Per-session message snapshot keyed by session_id. Lets tab switches
  // restore the last-seen messages instantly while we re-fetch fresh data
  // from the server in the background. Snapshots are written on
  // attachSession just before swapping to a new session.
  sessionCache: Map<string, SessionCacheEntry>;

  // Legacy compat (derived from dynamic state)
  deadloopMessages: Message[];
  interactiveMessages: Message[];
  hasMoreDeadloop: boolean;
  hasMoreInteractive: boolean;
  isDeadloopPaused: boolean;
  interactiveStatus: string | null;
  deadloopStatus: string | null;

  // Usage limits per CLI client
  usageLimits: Map<string, UsageLimitsByProvider>;

  // Daemon-reported machines
  machines: MachineWithProjects[];

  // Auth actions
  login: (token: string, userId: string, userEmail: string) => void;
  setUserEmail: (userEmail: string) => void;
  logout: () => void;

  // Actions
  connect: () => void;
  disconnect: () => void;
  sendMessage: (text: string) => void;
  addMessage: (message: Message) => void;
  approve: (toolCallId: string) => void;
  reject: (toolCallId: string) => void;
  answerQuestion: (toolUseId: string, answers: Record<string, string>) => void;
  showToast: (message: string, kind?: ToastKind) => void;
  dismissToast: (id: string) => void;
  clearMessages: () => void;
  startSession: (cliClientId?: string) => void;
  attachSession: (sessionId: string, forceReload?: boolean) => void;
  refreshCliClients: () => void;
  listMachines: () => void;
  startMachineProjectCli: (machineId: string, projectId: string) => void;
  stopMachineProjectCli: (machineId: string, projectId: string) => void;
  setMachineMiniMaxConfig: (
    machineId: string,
    apiKey?: string,
    clearApiKey?: boolean,
  ) => void;
  setMachineGlmConfig: (
    machineId: string,
    apiKey?: string,
    clearApiKey?: boolean,
  ) => void;
  listSessions: () => void;
  loadSessionMessages: (sessionId: string) => void;
  loadMoreMessages: (pane?: PaneType | number) => void;
  prependMessages: (messages: Message[], hasMore: boolean) => void;
  sendMessageToPane: (text: string, pane: PaneType | number) => { success: boolean; error?: string };
  addMessageToPane: (message: Message, pane: PaneType | number) => void;
  startAutoRefresh: () => void;
  stopAutoRefresh: () => void;
  pauseDeadloop: () => void;
  resumeDeadloop: () => void;
  pausePane: (paneId: number) => void;
  resumePane: (paneId: number) => void;
  addPane: (
    provider: string,
    mode: string,
    label?: string,
    prompt?: string,
    model?: string
  ) => { success: boolean; error?: string };
  removePane: (paneId: number) => void;
  updatePaneLabel: (paneId: number, label: string) => void;
  updatePaneEffort: (paneId: number, effort: string | null) => void;
  interruptPane: (paneId: number) => void;
  reorderPanes: (paneIds: number[]) => void;
  startBot: (
    paneId: number,
    prompt?: string,
    minIterationIntervalMinutes?: number,
    effort?: string,
  ) => void;
  stopBot: (paneId: number) => void;
  rebootCli: () => void;
  downloadSession: () => void;
}

const WS_URL = process.env.NEXT_PUBLIC_WS_URL || "ws://apas.mpaxos.com:8080";
const MINIMAX_API_BASE_URL = "https://api.minimax.io/anthropic";
const GLM_API_BASE_URL = "https://api.z.ai/api/anthropic";

export const useStore = create<AppState>((set, get) => ({
  // Auth state - initialize from localStorage if available
  token: typeof window !== 'undefined' ? localStorage.getItem("apas_token") : null,
  userId: typeof window !== 'undefined' ? localStorage.getItem("apas_user_id") : null,
  userEmail: typeof window !== 'undefined' ? localStorage.getItem("apas_user_email") : null,
  serverVersion: null,
  isAuthenticated: false,

  connected: false,
  // Restore sessionId from localStorage to persist across page refreshes
  sessionId: typeof window !== 'undefined' ? localStorage.getItem("apas_session_id") : null,
  // Restore cliClientId from localStorage for per-project settings
  cliClientId: typeof window !== 'undefined' ? localStorage.getItem("apas_cli_client_id") : null,
  ws: null,
  refreshInterval: null,
  isAttached: false,
  reconnectAttempts: 0,
  reconnectTimeout: null,
  visibilityHandler: null,
  cliClients: [],
  sessions: [],
  messages: [],
  hasMoreMessages: false,
  isLoadingMore: false,
  isDualPane: false,
  paneConfigs: [],
  paneMessages: {},
  paneHasMore: {},
  paneStatuses: {},
  paneModes: {},
  pausedPanes: [],
  answeredQuestions: new Map(),
  toasts: [],
  sessionCache: new Map(),
  loadingMorePane: null,
  // Legacy compat getters (populated from dynamic state)
  deadloopMessages: [],
  interactiveMessages: [],
  hasMoreDeadloop: false,
  hasMoreInteractive: false,
  isDeadloopPaused: false,
  interactiveStatus: null,
  deadloopStatus: null,
  usageLimits: new Map(),
  machines: [],

  login: (token: string, userId: string, userEmail: string) => {
    localStorage.setItem("apas_token", token);
    localStorage.setItem("apas_user_id", userId);
    localStorage.setItem("apas_user_email", userEmail);
    set({ token, userId, userEmail, isAuthenticated: true });
  },

  setUserEmail: (userEmail: string) => {
    localStorage.setItem("apas_user_email", userEmail);
    set({ userEmail });
  },

  logout: () => {
    localStorage.removeItem("apas_token");
    localStorage.removeItem("apas_user_id");
    localStorage.removeItem("apas_user_email");
    localStorage.removeItem("apas_session_id");
    const { ws, reconnectTimeout, visibilityHandler } = get();

    // Clear reconnect timeout
    if (reconnectTimeout) {
      clearTimeout(reconnectTimeout);
    }

    // Remove visibility handler
    if (visibilityHandler && typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', visibilityHandler);
    }

    if (ws) {
      ws.close(1000, "User logged out");
    }
    set({
      token: null,
      userId: null,
      userEmail: null,
      isAuthenticated: false,
      connected: false,
      ws: null,
      sessionId: null,
      serverVersion: null,
      cliClients: [],
      sessions: [],
      machines: [],
      paneModes: {},
      reconnectAttempts: 0,
      reconnectTimeout: null,
      visibilityHandler: null,
    });
  },

  connect: () => {
    const token = typeof window !== 'undefined' ? localStorage.getItem("apas_token") : null;
    if (!token) {
      console.log("No token found, cannot connect");
      return;
    }

    const { ws: currentWs, reconnectTimeout, visibilityHandler } = get();

    // Avoid opening duplicate sockets while one is already active/connecting.
    if (currentWs && (currentWs.readyState === WebSocket.OPEN || currentWs.readyState === WebSocket.CONNECTING)) {
      return;
    }

    // Clear any existing reconnect timeout
    if (reconnectTimeout) {
      clearTimeout(reconnectTimeout);
      set({ reconnectTimeout: null });
    }

    const ws = new WebSocket(`${WS_URL}/ws/web`);

    ws.onopen = () => {
      console.log("WebSocket connected, sending authentication...");
      // Reset reconnect attempts on successful connection
      set({ reconnectAttempts: 0 });
      // Send token for authentication
      ws.send(JSON.stringify({ type: "authenticate", token }));
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        handleServerMessage(data, set, get);
      } catch (e) {
        console.error("Failed to parse message:", e);
      }
    };

    ws.onclose = (event) => {
      console.log("WebSocket disconnected", event.code, event.reason);
      set({ connected: false, ws: null, cliClients: [], isAttached: false });

      // Auto-reconnect with exponential backoff (unless intentionally disconnected)
      if (event.code !== 1000) {
        const { reconnectAttempts } = get();
        const maxAttempts = 10;
        if (reconnectAttempts < maxAttempts) {
          const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000);
          console.log(`Scheduling reconnect attempt ${reconnectAttempts + 1} in ${delay}ms`);
          const timeout = setTimeout(() => {
            console.log(`Reconnect attempt ${reconnectAttempts + 1}`);
            set({ reconnectAttempts: reconnectAttempts + 1 });
            get().connect();
          }, delay);
          set({ reconnectTimeout: timeout });
        } else {
          console.log("Max reconnect attempts reached");
        }
      }
    };

    ws.onerror = (error) => {
      console.error("WebSocket error:", error);
    };

    // Add visibility change listener for mobile (only once)
    if (!visibilityHandler && typeof document !== 'undefined') {
      const handler = () => {
        if (document.visibilityState === 'visible') {
          const { ws, connected, sessionId, isAttached } = get();
          console.log("App became visible, checking connection...", { connected, isAttached, sessionId });
          if (!connected || !ws || ws.readyState !== WebSocket.OPEN) {
            console.log("Connection lost while in background, reconnecting...");
            set({ reconnectAttempts: 0 });
            get().connect();
          } else {
            console.log("Connection appears healthy, refreshing data...");
            get().refreshCliClients();
            get().listSessions();
            if (sessionId) {
              console.log("Refreshing attached session after foreground...");
              get().attachSession(sessionId, true);
            }
          }
        }
      };
      document.addEventListener('visibilitychange', handler);
      set({ visibilityHandler: handler });
    }

    set({ ws });
  },

  disconnect: () => {
    const { ws, reconnectTimeout, visibilityHandler } = get();
    get().stopAutoRefresh();

    if (reconnectTimeout) {
      clearTimeout(reconnectTimeout);
    }

    if (visibilityHandler && typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', visibilityHandler);
    }

    if (ws) {
      ws.close(1000, "User disconnected");
    }
    set({
      connected: false,
      ws: null,
      sessionId: null,
      serverVersion: null,
      cliClients: [],
      machines: [],
      paneModes: {},
      isAttached: false,
      reconnectAttempts: 0,
      reconnectTimeout: null,
      visibilityHandler: null,
    });
  },

  startSession: (cliClientId?: string) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    set({
      messages: [],
      sessionId: null,
      paneMessages: {},
      paneHasMore: {},
      paneStatuses: {},
      paneModes: {},
      pausedPanes: [],
      paneConfigs: [],
      answeredQuestions: new Map(),
      deadloopMessages: [],
      interactiveMessages: [],
      isDualPane: false,
      isDeadloopPaused: false,
      interactiveStatus: null,
      deadloopStatus: null,
    });

    ws.send(JSON.stringify({
      type: "start_session",
      cli_client_id: cliClientId || null
    }));
  },

  attachSession: (sessionId: string, forceReload = false) => {
    const state = get();
    const { ws, sessionId: currentSessionId } = state;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    localStorage.setItem("apas_session_id", sessionId);

    const isSameSession = currentSessionId === sessionId;

    const { cliClients, sessions } = state;
    const hasActiveClient = cliClients.some(c => c.activeSession === sessionId);

    let newCliClientId: string | null = null;
    const activeClient = cliClients.find(c => c.activeSession === sessionId);
    const sessionInfo = sessions.find(s => s.id === sessionId);
    // Prefer live active-session mapping over persisted session metadata.
    if (activeClient) {
      newCliClientId = activeClient.id;
    } else if (sessionInfo?.cliClientId) {
      newCliClientId = sessionInfo.cliClientId;
    }

    if (newCliClientId) {
      localStorage.setItem("apas_cli_client_id", newCliClientId);
    }

    if (!isSameSession || forceReload) {
      // Snapshot the session we're leaving so we can restore it instantly
      // next time the user comes back. Only snapshot if the session had
      // any messages — otherwise we'd cache an empty state and shadow a
      // fresh fetch on return.
      const sessionCache = new Map(state.sessionCache);
      if (
        currentSessionId &&
        currentSessionId !== sessionId &&
        (state.messages.length > 0 || Object.keys(state.paneMessages).length > 0)
      ) {
        const entry = {
          messages: state.messages,
          paneMessages: state.paneMessages,
          paneHasMore: state.paneHasMore,
          paneConfigs: state.paneConfigs,
          paneModes: state.paneModes,
          hasMoreMessages: state.hasMoreMessages,
          isDualPane: state.isDualPane,
          answeredQuestions: state.answeredQuestions,
          cachedAt: Date.now(),
        };
        sessionCache.set(currentSessionId, entry);
        // Mirror the snapshot to IndexedDB so it survives a reload —
        // fire and forget; failure just means next reload re-fetches.
        saveSnapshotIdb(currentSessionId, entry);
        // Cap the cache to prevent unbounded growth across long sessions
        // of project hopping. LRU by insertion order — Map preserves it.
        const MAX_CACHED_SESSIONS = 12;
        while (sessionCache.size > MAX_CACHED_SESSIONS) {
          const oldest = sessionCache.keys().next().value;
          if (oldest === undefined) break;
          sessionCache.delete(oldest);
          deleteSnapshotIdb(oldest);
        }
      }

      // Restore from cache if we have a snapshot — instant tab switch.
      // Server-side session_messages will still arrive and replace as the
      // authoritative state.
      const cached = forceReload ? undefined : sessionCache.get(sessionId);
      if (cached) {
        set({
          sessionId,
          cliClientId: newCliClientId,
          messages: cached.messages,
          paneMessages: cached.paneMessages,
          paneHasMore: cached.paneHasMore,
          paneConfigs: cached.paneConfigs,
          paneModes: cached.paneModes,
          hasMoreMessages: cached.hasMoreMessages,
          isDualPane: cached.isDualPane,
          answeredQuestions: cached.answeredQuestions,
          // Live state that doesn't survive a switch — refresh on attach.
          paneStatuses: {},
          pausedPanes: [],
          deadloopMessages: cached.paneMessages[paneKey(PANE_ID_DEADLOOP)] ?? [],
          interactiveMessages: cached.paneMessages[paneKey(PANE_ID_INTERACTIVE)] ?? [],
          isAttached: hasActiveClient,
          isDeadloopPaused: false,
          interactiveStatus: null,
          deadloopStatus: null,
          sessionCache,
        });
      } else {
        set({
          sessionId,
          cliClientId: newCliClientId,
          messages: [],
          paneMessages: {},
          paneHasMore: {},
          paneStatuses: {},
          paneModes: {},
          pausedPanes: [],
          paneConfigs: [],
          answeredQuestions: new Map(),
          deadloopMessages: [],
          interactiveMessages: [],
          isDualPane: false,
          isAttached: hasActiveClient,
          isDeadloopPaused: false,
          interactiveStatus: null,
          deadloopStatus: null,
          sessionCache,
        });
      }
    } else {
      set({ isAttached: hasActiveClient, cliClientId: newCliClientId });
    }

    ws.send(JSON.stringify({
      type: "attach_session",
      session_id: sessionId
    }));
  },

  refreshCliClients: () => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({ type: "list_cli_clients" }));
  },

  listMachines: () => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({ type: "list_machines" }));
  },

  startMachineProjectCli: (machineId: string, projectId: string) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({
      type: "start_machine_project_cli",
      machine_id: machineId,
      project_id: projectId,
    }));
  },

  stopMachineProjectCli: (machineId: string, projectId: string) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({
      type: "stop_machine_project_cli",
      machine_id: machineId,
      project_id: projectId,
    }));
  },

  setMachineMiniMaxConfig: (
    machineId: string,
    apiKey?: string,
    clearApiKey: boolean = false,
  ) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    const normalizedApiKey =
      apiKey && apiKey.trim().length > 0 ? apiKey.trim() : undefined;

    // Reflect the key change in UI immediately so users can verify what was saved.
    set((state) => ({
      machines: state.machines.map((entry) => {
        if (entry.machine.machineId !== machineId) return entry;
        const existingBackend = entry.machine.minimaxBackend;
        const nextApiKey = clearApiKey
          ? undefined
          : (normalizedApiKey ?? existingBackend?.apiKey);
        return {
          ...entry,
          machine: {
            ...entry.machine,
            minimaxBackend: {
              apiBaseUrl: MINIMAX_API_BASE_URL,
              apiKey: nextApiKey,
              apiKeyConfigured: Boolean(nextApiKey),
            },
          },
        };
      }),
    }));

    ws.send(JSON.stringify({
      type: "set_machine_mini_max_config",
      machine_id: machineId,
      api_base_url: MINIMAX_API_BASE_URL,
      api_key: normalizedApiKey,
      clear_api_key: clearApiKey,
    }));
  },

  setMachineGlmConfig: (
    machineId: string,
    apiKey?: string,
    clearApiKey: boolean = false,
  ) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    const normalizedApiKey =
      apiKey && apiKey.trim().length > 0 ? apiKey.trim() : undefined;

    set((state) => ({
      machines: state.machines.map((entry) => {
        if (entry.machine.machineId !== machineId) return entry;
        const existingBackend = entry.machine.glmBackend;
        const nextApiKey = clearApiKey
          ? undefined
          : (normalizedApiKey ?? existingBackend?.apiKey);
        return {
          ...entry,
          machine: {
            ...entry.machine,
            glmBackend: {
              apiBaseUrl: GLM_API_BASE_URL,
              apiKey: nextApiKey,
              apiKeyConfigured: Boolean(nextApiKey),
            },
          },
        };
      }),
    }));

    ws.send(JSON.stringify({
      type: "set_machine_glm_config",
      machine_id: machineId,
      api_base_url: GLM_API_BASE_URL,
      api_key: normalizedApiKey,
      clear_api_key: clearApiKey,
    }));
  },

  listSessions: () => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    ws.send(JSON.stringify({ type: "list_sessions" }));
  },

  loadSessionMessages: (sessionId: string) => {
    const { ws } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    localStorage.setItem("apas_session_id", sessionId);
    set({
      sessionId,
      messages: [],
      paneMessages: {},
      paneHasMore: {},
      paneStatuses: {},
      paneModes: {},
      pausedPanes: [],
      paneConfigs: [],
      answeredQuestions: new Map(),
      deadloopMessages: [],
      interactiveMessages: [],
      isDeadloopPaused: false,
      interactiveStatus: null,
      deadloopStatus: null,
      isDualPane: false,
      isAttached: false
    });
    ws.send(JSON.stringify({ type: "get_session_messages", session_id: sessionId }));
  },

  sendMessage: (text: string) => {
    const { ws, sessionId } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return;
    }

    const userMessage: Message = {
      id: generateId(),
      role: "user",
      content: text,
      timestamp: new Date(),
      outputType: { type: "text" },
    };
    set((state) => ({ messages: [...state.messages, userMessage] }));

    if (!sessionId) {
      ws.send(JSON.stringify({ type: "start_session" }));
    }

    ws.send(JSON.stringify({ type: "input", text }));
  },

  addMessage: (message: Message) => {
    set((state) => ({ messages: [...state.messages, message] }));
  },

  approve: (toolCallId: string) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "approve", tool_call_id: toolCallId }));
    }
  },

  reject: (toolCallId: string) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reject", tool_call_id: toolCallId }));
    }
  },

  answerQuestion: (toolUseId: string, answers: Record<string, string>) => {
    const { ws, showToast } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: "answer_question",
        tool_use_id: toolUseId,
        answers,
      }));
      showToast("Answer sent to Claude", "success");
    } else {
      showToast("Not connected — answer not sent", "error");
    }
    // Mark the question locally as answered so the card flips to the
    // submitted state immediately, even before the tool_result arrives.
    set((state) => {
      const next = new Map(state.answeredQuestions);
      next.set(toolUseId, answers);
      return { answeredQuestions: next };
    });
  },

  showToast: (message: string, kind: ToastKind = "info") => {
    const id = generateId();
    set((state) => ({ toasts: [...state.toasts, { id, kind, message }] }));
  },

  dismissToast: (id: string) => {
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },

  clearMessages: () => {
    set({ messages: [] });
  },

  loadMoreMessages: (pane?: PaneType | number) => {
    const { ws, sessionId, messages, paneMessages, isDualPane, loadingMorePane, hasMoreMessages, paneHasMore } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return;
    }
    if (!sessionId || loadingMorePane) {
      return;
    }

    const paneId = typeof pane === "number"
      ? pane
      : (pane ? normalizePaneId(pane, undefined) : undefined);
    const paneType = typeof pane === "string" ? pane : legacyPaneType(paneId) as PaneType | undefined;

    let targetMessages: Message[];
    let hasMore: boolean;

    if (isDualPane && paneId) {
      targetMessages = paneMessages[paneId] || [];
      hasMore = paneHasMore[paneId] || false;
    } else {
      targetMessages = messages;
      hasMore = hasMoreMessages;
    }

    if (!hasMore || targetMessages.length === 0) {
      return;
    }

    const oldestMessage = targetMessages.reduce((oldest, msg) =>
      msg.timestamp < oldest.timestamp ? msg : oldest
    );

    set({ loadingMorePane: paneId || null, isLoadingMore: true });

    ws.send(JSON.stringify({
      type: "get_session_messages",
      session_id: sessionId,
      limit: 50,
      before_id: oldestMessage.id,
      pane_type: paneType, // Legacy compat
      pane_id: paneId // New field
    }));
  },

  prependMessages: (newMessages: Message[], hasMore: boolean) => {
    set((state) => ({
      messages: [...newMessages, ...state.messages],
      hasMoreMessages: hasMore,
      isLoadingMore: false
    }));
  },

  sendMessageToPane: (text: string, pane: PaneType | number): { success: boolean; error?: string } => {
    const { ws, sessionId, isAttached } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      console.error("WebSocket not connected");
      return { success: false, error: "Not connected to server" };
    }

    if (!isAttached) {
      console.error("Session is not active");
      return { success: false, error: "Session is not active. Start the CLI to send messages." };
    }

    const paneId = typeof pane === "number" ? pane : normalizePaneId(pane, undefined);
    const paneType = legacyPaneType(paneId) || (typeof pane === "string" ? pane : undefined);

    ws.send(JSON.stringify({
      type: "input",
      text,
      pane_type: paneType, // Legacy compat: must be "deadloop" or "interactive"
      pane_id: paneId
    }));
    return { success: true };
  },

  addMessageToPane: (message: Message, pane: PaneType | number) => {
    const paneId = typeof pane === "number" ? pane : (normalizePaneId(pane, undefined) ?? 0);
    const key = paneKey(paneId);
    set((state) => {
      const current = state.paneMessages[key] || [];
      const newPaneMessages = { ...state.paneMessages, [key]: [...current, message] };
      return {
        paneMessages: newPaneMessages,
        // Legacy compat
        deadloopMessages: newPaneMessages[paneKey(PANE_ID_DEADLOOP)] || [],
        interactiveMessages: newPaneMessages[paneKey(PANE_ID_INTERACTIVE)] || [],
      };
    });
  },

  startAutoRefresh: () => {
    const { refreshInterval } = get();
    if (refreshInterval) return;

    const interval = setInterval(() => {
      const { ws, connected, sessionId, isAttached, cliClients } = get();

      if (ws && ws.readyState !== WebSocket.OPEN) {
        console.log("WebSocket not in OPEN state, triggering reconnect...");
        set({ connected: false, ws: null, isAttached: false, reconnectAttempts: 0 });
        get().connect();
        return;
      }

      if (!connected) return;

      get().refreshCliClients();
      get().listMachines();
      get().listSessions();

      if (sessionId && !isAttached) {
        const activeClient = cliClients.find(c => c.activeSession === sessionId);
        if (activeClient) {
          get().attachSession(sessionId);
        }
      }
    }, 3000);

    set({ refreshInterval: interval });
  },

  stopAutoRefresh: () => {
    const { refreshInterval } = get();
    if (refreshInterval) {
      clearInterval(refreshInterval);
      set({ refreshInterval: null });
    }
  },

  pauseDeadloop: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "pause_deadloop" }));
      // Also send new pane-specific pause
      ws.send(JSON.stringify({ type: "pause_pane", pane_id: PANE_ID_DEADLOOP }));
    }
  },

  resumeDeadloop: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "resume_deadloop" }));
      // Also send new pane-specific resume
      ws.send(JSON.stringify({ type: "resume_pane", pane_id: PANE_ID_DEADLOOP }));
    }
  },

  pausePane: (paneId: number) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "pause_pane", pane_id: paneId }));
      // Also send legacy message for backward compat
      if (paneId === PANE_ID_DEADLOOP) {
        ws.send(JSON.stringify({ type: "pause_deadloop" }));
      }
    }
  },

  resumePane: (paneId: number) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "resume_pane", pane_id: paneId }));
      if (paneId === PANE_ID_DEADLOOP) {
        ws.send(JSON.stringify({ type: "resume_deadloop" }));
      }
    }
  },

  addPane: (
    provider: string,
    mode: string,
    label?: string,
    prompt?: string,
    model?: string
  ) => {
    const { ws, sessionId, isAttached } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return { success: false, error: "Not connected to server" };
    }
    if (!sessionId) {
      return { success: false, error: "No session selected" };
    }
    if (!isAttached) {
      return { success: false, error: "Project is not running. Start the CLI client first." };
    }
    ws.send(JSON.stringify({
      type: "add_pane",
      provider,
      mode,
      label: label || undefined,
      prompt: prompt || undefined,
      model: model || undefined,
    }));
    return { success: true };
  },

  removePane: (paneId: number) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "remove_pane", pane_id: paneId }));
    }
  },

  updatePaneLabel: (paneId: number, label: string) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "update_pane_label", pane_id: paneId, label }));
    }
  },

  updatePaneEffort: (paneId: number, effort: string | null) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: "update_pane_effort",
        pane_id: paneId,
        effort: effort ?? null,
      }));
    }
  },

  interruptPane: (paneId: number) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "interrupt_pane", pane_id: paneId }));
    }
  },

  reorderPanes: (paneIds: number[]) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reorder_panes", pane_ids: paneIds }));
    }
  },

  startBot: (
    paneId: number,
    prompt?: string,
    minIterationIntervalMinutes?: number,
    effort?: string,
  ) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      const trimmedEffort = typeof effort === "string" ? effort.trim() : "";
      ws.send(JSON.stringify({
        type: "start_bot",
        pane_id: paneId,
        ...(prompt ? { prompt } : {}),
        ...(typeof minIterationIntervalMinutes === "number" && Number.isFinite(minIterationIntervalMinutes)
          ? { min_iteration_interval_minutes: Math.max(0, Math.floor(minIterationIntervalMinutes)) }
          : {}),
        ...(trimmedEffort ? { effort: trimmedEffort } : {}),
      }));
    }
  },

  stopBot: (paneId: number) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "stop_bot", pane_id: paneId }));
    }
  },

  rebootCli: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reboot_cli" }));
    }
  },

  downloadSession: () => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN && sessionId) {
      ws.send(JSON.stringify({ type: "download_session", session_id: sessionId }));
    }
  },
}));

// Hydrate the per-session message snapshots from IndexedDB on app boot.
// Runs once at module load; until it resolves, the in-memory cache is
// empty and tab switches fall back to the server fetch. Once hydrated,
// subsequent tab switches (and reloads, since the data survives) become
// instant. New entries written during the hydration window are merged
// rather than overwritten — the in-memory write wins on conflict, since
// it reflects the freshest state the user just saw.
if (typeof window !== "undefined") {
  loadAllSnapshotsIdb().then((diskCache) => {
    if (diskCache.size === 0) return;
    useStore.setState((state) => {
      const merged = new Map(diskCache);
      for (const [k, v] of state.sessionCache) {
        merged.set(k, v); // in-memory wins
      }
      return { sessionCache: merged };
    });
  });
}

// Helper function to route messages to correct array based on pane_id
function updatePaneModeHint(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
  rawPaneType: string | undefined,
  rawPaneId: number | string | undefined,
) {
  const modeHint = normalizePaneModeHint(rawPaneType);
  if (!modeHint) return;
  const paneId = normalizePaneId(rawPaneType, normalizeRawPaneId(rawPaneId));
  if (!paneId) return;
  const key = paneKey(paneId);
  if (get().paneModes[key] === modeHint) return;
  set((state) => ({ paneModes: { ...state.paneModes, [key]: modeHint } }));
}

function addMessageWithPaneRouting(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
  message: Message,
  rawPaneType: string | undefined,
  rawPaneId: number | string | undefined
) {
  const paneId = normalizePaneId(rawPaneType, normalizeRawPaneId(rawPaneId));
  let { isDualPane } = get();

  // Auto-detect dual pane mode when we receive a pane identifier
  if (paneId && !isDualPane) {
    set({ isDualPane: true });
    isDualPane = true;
  }

  if (isDualPane && paneId) {
    const key = paneKey(paneId);
    set((state) => {
      const current = state.paneMessages[key] || [];
      const newPaneMessages = { ...state.paneMessages, [key]: [...current, message] };
      return {
        paneMessages: newPaneMessages,
        // Legacy compat
        deadloopMessages: newPaneMessages[paneKey(PANE_ID_DEADLOOP)] || state.deadloopMessages,
        interactiveMessages: newPaneMessages[paneKey(PANE_ID_INTERACTIVE)] || state.interactiveMessages,
      };
    });
  } else {
    set((state) => ({ messages: [...state.messages, message] }));
  }
}

function handleServerMessage(
  data: Record<string, unknown>,
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState
) {
  switch (data.type) {
    case "authenticated":
      set({
        connected: true,
        isAuthenticated: true,
        userId: data.user_id as string,
        userEmail: (data.user_email as string | undefined) ?? get().userEmail,
        serverVersion: (data.server_version as string | undefined) ?? null,
      });
      if (data.user_email) {
        localStorage.setItem("apas_user_email", data.user_email as string);
      }
      console.log("Authenticated as user:", data.user_id);
      get().refreshCliClients();
      get().listMachines();
      get().listSessions();
      get().startAutoRefresh();
      {
        // Prefer the in-memory sessionId (what this tab is currently viewing)
        // over localStorage — otherwise a reconnect could hijack this tab to a
        // session another browser tab wrote to localStorage.
        const currentSessionId = get().sessionId;
        const sessionToRestore =
          currentSessionId || localStorage.getItem("apas_session_id");
        if (sessionToRestore) {
          console.log("Restoring session:", sessionToRestore);
          setTimeout(() => {
            get().attachSession(sessionToRestore, true);
          }, 500);
        }
      }
      break;

    case "authentication_failed":
      console.error("Authentication failed:", data.reason);
      localStorage.removeItem("apas_token");
      localStorage.removeItem("apas_user_id");
      localStorage.removeItem("apas_user_email");
      set({
        connected: false,
        isAuthenticated: false,
        token: null,
        userId: null,
        userEmail: null,
        serverVersion: null,
      });
      break;

    case "cli_clients": {
      const clients = (data.clients as Array<Record<string, unknown>>) || [];
      const parsedClients = clients.map((c) => ({
        id: c.id as string,
        name: c.name as string | undefined,
        status: (c.status as "online" | "offline" | "busy") || "offline",
        version: c.version as string | undefined,
        lastSeen: c.last_seen as string | undefined,
        activeSession: c.active_session as string | undefined,
      }));

      const { sessionId } = get();
      const activeClientForSession = sessionId
        ? parsedClients.find((c) => c.activeSession === sessionId)
        : undefined;
      const hasActiveClientInOurList = sessionId
        ? parsedClients.some(c => c.activeSession === sessionId)
        : false;

      set((state) => {
        const next: Partial<AppState> = {
          cliClients: parsedClients,
          ...(hasActiveClientInOurList ? { isAttached: true } : {}),
        };

        if (activeClientForSession && state.cliClientId !== activeClientForSession.id) {
          next.cliClientId = activeClientForSession.id;
          if (typeof window !== "undefined") {
            localStorage.setItem("apas_cli_client_id", activeClientForSession.id);
          }
        }

        return next;
      });
      break;
    }

    case "machines": {
      const machines = (data.machines as Array<Record<string, unknown>>) || [];
      const parsed: MachineWithProjects[] = machines.map((item) => {
        const machine = (item.machine as Record<string, unknown>) || {};
        const projects = (item.projects as Array<Record<string, unknown>>) || [];
        return {
          machine: {
            machineId: machine.machine_id as string,
            hostname: machine.hostname as string,
            os: machine.os as string,
            arch: machine.arch as string,
            daemonVersion: machine.daemon_version as string | undefined,
            minimaxBackend: machine.minimax_backend
              ? {
                  apiBaseUrl: ((machine.minimax_backend as Record<string, unknown>).api_base_url as string | undefined),
                  apiKey: ((machine.minimax_backend as Record<string, unknown>).api_key as string | undefined),
                  apiKeyConfigured: Boolean(
                    (machine.minimax_backend as Record<string, unknown>).api_key_configured
                  ),
                }
              : undefined,
            glmBackend: machine.glm_backend
              ? {
                  apiBaseUrl: ((machine.glm_backend as Record<string, unknown>).api_base_url as string | undefined),
                  apiKey: ((machine.glm_backend as Record<string, unknown>).api_key as string | undefined),
                  apiKeyConfigured: Boolean(
                    (machine.glm_backend as Record<string, unknown>).api_key_configured
                  ),
                }
              : undefined,
            lastSeen: machine.last_seen as string | undefined,
          },
          projects: projects.map((project) => ({
            projectId: project.project_id as string,
            name: project.name as string | undefined,
            path: project.path as string,
            isRunning: Boolean(project.is_running),
            pid: project.pid as number | undefined,
            memoryKb: project.memory_kb as number | undefined,
            lastError: project.last_error as string | undefined,
          })),
        };
      });

      set((state) => {
        const previousById = new Map(
          state.machines.map((entry) => [entry.machine.machineId, entry])
        );
        const merged = parsed.map((entry) => {
          const previous = previousById.get(entry.machine.machineId);
          const minimaxBackend = entry.machine.minimaxBackend;
          const previousMiniMaxKey = previous?.machine.minimaxBackend?.apiKey;
          const mergedMiniMax =
            minimaxBackend &&
            minimaxBackend.apiKey === undefined &&
            minimaxBackend.apiKeyConfigured &&
            previousMiniMaxKey
              ? {
                  ...minimaxBackend,
                  apiKey: previousMiniMaxKey,
                }
              : minimaxBackend;

          const glmBackend = entry.machine.glmBackend;
          const previousGlmKey = previous?.machine.glmBackend?.apiKey;
          const mergedGlm =
            glmBackend &&
            glmBackend.apiKey === undefined &&
            glmBackend.apiKeyConfigured &&
            previousGlmKey
              ? {
                  ...glmBackend,
                  apiKey: previousGlmKey,
                }
              : glmBackend;

          return {
            ...entry,
            machine: {
              ...entry.machine,
              minimaxBackend: mergedMiniMax,
              glmBackend: mergedGlm,
            },
          };
        });
        return { machines: merged };
      });
      break;
    }

    case "session_started": {
      const newSessionId = data.session_id as string;
      const existing = get().sessionId;
      // Only adopt the session if we don't already have one — otherwise this
      // is a stale response for a previous attach the user has since navigated
      // away from.
      if (!existing || existing === newSessionId) {
        set({ sessionId: newSessionId });
        console.log("Session started:", newSessionId);
      } else {
        console.log(
          "Ignoring stale session_started for",
          newSessionId,
          "(currently on",
          existing,
          ")",
        );
      }
      break;
    }

    case "session_status":
      console.log("Session status:", data.status);
      break;

    case "session_attached": {
      const hasActiveCli = data.has_active_cli as boolean;
      console.log("Session attached, has active CLI:", hasActiveCli);
      set({ isAttached: hasActiveCli });
      break;
    }

    case "pane_status": {
      const paneType = data.pane_type as string | undefined;
      const paneId = normalizePaneId(paneType, data.pane_id as number | undefined);
      const status = data.status as string | null;
      const modeHint = normalizePaneModeHint(paneType);

      if (paneId) {
        set((state) => ({
          paneStatuses: { ...state.paneStatuses, [paneId]: status },
          paneModes: modeHint
            ? { ...state.paneModes, [paneId]: modeHint }
            : state.paneModes,
          // Legacy compat
          interactiveStatus: paneId === PANE_ID_INTERACTIVE ? status : state.interactiveStatus,
          deadloopStatus: paneId === PANE_ID_DEADLOOP ? status : state.deadloopStatus,
        }));
      }
      break;
    }

    case "pane_list": {
      // Drop pane_list events for sessions we're not currently viewing.
      const msgSid = data.session_id as string | undefined;
      const curSid = get().sessionId;
      if (msgSid && curSid && msgSid !== curSid) {
        break;
      }
      const panes = ((data.panes as PaneConfig[]) || []).map((pane) => ({
        ...pane,
        provider: normalizeProvider(pane.provider) ?? "claude",
      }));
      const paneModes = panes.reduce<Record<string, PaneType>>((acc, pane) => {
        acc[paneKey(pane.pane_id)] = pane.mode;
        return acc;
      }, {});
      // Sync is_paused from pane configs to pausedPanes state
      const pausedPaneIds = panes.filter((p) => p.is_paused).map((p) => p.pane_id);
      set({
        paneConfigs: panes,
        paneModes,
        pausedPanes: pausedPaneIds,
        // Legacy compat
        isDeadloopPaused: pausedPaneIds.includes(PANE_ID_DEADLOOP),
      });
      break;
    }

    case "output": {
      const outputType = parseOutputType(data.output_type as Record<string, unknown> | undefined);
      const message: Message = {
        id: generateId(),
        role: "assistant",
        content: data.content as string,
        timestamp: new Date(),
        outputType,
      };
      const paneType = data.pane_type as string | undefined;
      const paneId = data.pane_id as number | string | undefined;
      updatePaneModeHint(set, get, paneType, paneId);
      addMessageWithPaneRouting(set, get, message, paneType, paneId);
      break;
    }

    case "error":
      console.error("Server error:", data.message);
      const errorMessage: Message = {
        id: generateId(),
        role: "system",
        content: data.message as string,
        timestamp: new Date(),
        outputType: { type: "error" },
      };
      set((state) => ({ messages: [...state.messages, errorMessage] }));
      break;

    case "sessions": {
      const sessions = (data.sessions as Array<Record<string, unknown>>) || [];
      const parsedSessions = sessions.map((s) => ({
        id: s.id as string,
        projectId: (s.project_id as string | undefined) ?? (s.id as string),
        cliClientId: s.cli_client_id as string | undefined,
        workingDir: s.working_dir as string | undefined,
        hostname: s.hostname as string | undefined,
        status: s.status as string,
        createdAt: s.created_at as string | undefined,
        isShared: s.is_shared as boolean | undefined,
        ownerEmail: s.owner_email as string | undefined,
        shareRole: s.share_role as "owner" | "admin" | "user" | undefined,
        isActive: s.is_active as boolean | undefined,
      }));

      set((state) => {
        const next: Partial<AppState> = { sessions: parsedSessions };
        if (state.sessionId) {
          const activeClient = state.cliClients.find((c) => c.activeSession === state.sessionId);
          const currentSession = parsedSessions.find((s) => s.id === state.sessionId);
          if (currentSession?.isActive != null) {
            // Keep attachment status aligned with server truth for this session.
            // This prevents stale "attached" UI state after a CLI crashes.
            next.isAttached = Boolean(currentSession.isActive);
          }
          const preferredCliClientId = activeClient?.id ?? currentSession?.cliClientId ?? null;

          if (preferredCliClientId && state.cliClientId !== preferredCliClientId) {
            next.cliClientId = preferredCliClientId;
            if (typeof window !== "undefined") {
              localStorage.setItem("apas_cli_client_id", preferredCliClientId);
            }
          }
        }
        return next;
      });
      break;
    }

    case "session_messages": {
      const messages = (data.messages as Array<Record<string, unknown>>) || [];
      const hasMore = data.has_more as boolean || false;

      // Drop stale responses — if the response is for a different session
      // than the one we're currently viewing, ignore it entirely (do NOT
      // overwrite sessionId or panes).
      const responseSessionId = data.session_id as string | undefined;
      const currentSessionId = get().sessionId;
      if (responseSessionId && currentSessionId && responseSessionId !== currentSessionId) {
        break;
      }

      const { isLoadingMore, loadingMorePane } = get();

      // Check if any messages have pane_type or pane_id - if so, enable dual pane
      const hasPaneType = messages.some((m) => m.pane_type || m.pane_id);
      if (hasPaneType) {
        set({ isDualPane: true });
      }

      const parsedMessages: Message[] = messages.map((m) => {
        const messageType = m.message_type as string || "text";
        const content = m.content as string;
        let outputType: OutputType;
        let displayContent = content;

        if (messageType === "tool_use") {
          try {
            const toolData = JSON.parse(content);
            outputType = {
              type: "tool_use",
              tool: toolData.name as string,
              input: toolData.input,
              toolUseId: toolData.id as string | undefined,
            };
            displayContent = `Using ${toolData.name}: ${JSON.stringify(toolData.input)}`;
            // Store id→name mapping so tool_result can look it up
            if (toolData.id && toolData.name) {
              toolNameMap.set(toolData.id as string, toolData.name as string);
            }
          } catch {
            outputType = { type: "text" };
          }
        } else if (messageType === "tool_result") {
          try {
            const resultData = JSON.parse(content);
            const toolUseId = resultData.tool_use_id as string;
            outputType = {
              type: "tool_result",
              tool: toolNameMap.get(toolUseId) || toolUseId,
              success: !resultData.is_error,
            };
            displayContent = resultData.content as string || content;
            // Recover AskUserQuestion's answers from the persisted
            // tool_use_result so the card shows its submitted state after
            // a page reload.
            if (
              toolUseId &&
              toolNameMap.get(toolUseId) === "AskUserQuestion" &&
              resultData.tool_use_result &&
              typeof resultData.tool_use_result === "object"
            ) {
              const answers = (resultData.tool_use_result as Record<string, unknown>)
                .answers as Record<string, string> | undefined;
              if (answers && typeof answers === "object") {
                set((state) => {
                  if (state.answeredQuestions.has(toolUseId)) {
                    return {};
                  }
                  const next = new Map(state.answeredQuestions);
                  next.set(toolUseId, answers);
                  return { answeredQuestions: next };
                });
              }
            }
          } catch {
            outputType = { type: "text" };
          }
        } else if (messageType === "result" || messageType === "system") {
          outputType = { type: "system" };
        } else {
          outputType = { type: "text" };
        }

        const rawRole = m.role as string;
        const role: "user" | "assistant" | "system" = rawRole === "tool" ? "assistant" : rawRole as "user" | "assistant" | "system";

        return {
          id: m.id as string,
          role,
          content: displayContent,
          timestamp: new Date(m.created_at as string || Date.now()),
          outputType,
        };
      });

      // Route messages to correct panes using pane_id
      const { isDualPane } = get();
      const paneMsgBuckets: Record<string, Message[]> = {};
      const mainMsgs: Message[] = [];
      const paneModeHints: Record<string, PaneType> = {};

      messages.forEach((m, i) => {
        const rawPaneType = m.pane_type as string | undefined;
        const paneId = normalizePaneId(rawPaneType, m.pane_id as number | undefined);
        const msg = parsedMessages[i];
        if (paneId) {
          if (!paneMsgBuckets[paneId]) paneMsgBuckets[paneId] = [];
          paneMsgBuckets[paneId].push(msg);
          const modeHint = normalizePaneModeHint(rawPaneType);
          if (modeHint) {
            paneModeHints[paneKey(paneId)] = modeHint;
          }
        } else {
          mainMsgs.push(msg);
        }
      });
      const hasPaneModeHints = Object.keys(paneModeHints).length > 0;

      if (isLoadingMore) {
        // Prepend older messages
        if (isDualPane || hasPaneType) {
          set((state) => {
            const newPaneMessages = { ...state.paneMessages };
            for (const [paneId, msgs] of Object.entries(paneMsgBuckets)) {
              newPaneMessages[paneId] = [...msgs, ...(newPaneMessages[paneId] || [])];
            }

            const updates: Partial<AppState> = {
              messages: [...mainMsgs, ...state.messages],
              paneMessages: newPaneMessages,
              deadloopMessages: newPaneMessages[paneKey(PANE_ID_DEADLOOP)] || state.deadloopMessages,
              interactiveMessages: newPaneMessages[paneKey(PANE_ID_INTERACTIVE)] || state.interactiveMessages,
              isLoadingMore: false,
              loadingMorePane: null,
            };

            if (hasPaneModeHints) {
              updates.paneModes = { ...state.paneModes, ...paneModeHints };
            }

            // Update the appropriate hasMore flag
            if (loadingMorePane) {
              updates.paneHasMore = { ...state.paneHasMore, [loadingMorePane]: hasMore };
              if (loadingMorePane === PANE_ID_DEADLOOP) updates.hasMoreDeadloop = hasMore;
              if (loadingMorePane === PANE_ID_INTERACTIVE) updates.hasMoreInteractive = hasMore;
            } else {
              updates.hasMoreMessages = hasMore;
            }

            return updates;
          });
        } else {
          if (hasPaneModeHints) {
            set((state) => ({ paneModes: { ...state.paneModes, ...paneModeHints } }));
          }
          get().prependMessages(parsedMessages, hasMore);
        }
      } else if (isDualPane || hasPaneType) {
        // Initial load - dual pane mode. Replace existing pane messages with
        // the server's authoritative snapshot (rather than prepending) — if
        // the user is returning to a cached session, the cached state and
        // the server's latest 50 overlap, and prepending would create
        // duplicates plus mis-order the timeline.
        const { paneModes: existingPaneModes } = get();
        const newPaneMessages: Record<string, Message[]> = {};
        const newPaneHasMore: Record<string, boolean> = {};
        for (const [paneId, msgs] of Object.entries(paneMsgBuckets)) {
          newPaneMessages[paneId] = msgs;
          newPaneHasMore[paneId] = hasMore;
        }
        set({
          sessionId: data.session_id as string,
          messages: mainMsgs,
          paneMessages: newPaneMessages,
          paneHasMore: newPaneHasMore,
          deadloopMessages: newPaneMessages[paneKey(PANE_ID_DEADLOOP)] || [],
          interactiveMessages: newPaneMessages[paneKey(PANE_ID_INTERACTIVE)] || [],
          hasMoreMessages: hasMore,
          hasMoreDeadloop: newPaneHasMore[paneKey(PANE_ID_DEADLOOP)] || false,
          hasMoreInteractive: newPaneHasMore[paneKey(PANE_ID_INTERACTIVE)] || false,
          paneModes: hasPaneModeHints
            ? { ...existingPaneModes, ...paneModeHints }
            : existingPaneModes,
          isDualPane: true,
        });
      } else {
        // Initial load - single pane mode
        set((state) => ({
          sessionId: data.session_id as string,
          messages: parsedMessages,
          hasMoreMessages: hasMore,
          paneModes: hasPaneModeHints
            ? { ...state.paneModes, ...paneModeHints }
            : state.paneModes,
        }));
      }
      break;
    }

    case "user_input": {
      const msgSessionId = data.session_id as string | undefined;
      const { sessionId: currentSessionId } = get();
      if (msgSessionId && currentSessionId && msgSessionId !== currentSessionId) {
        break;
      }

      const userMessage: Message = {
        id: generateId(),
        role: "user",
        content: data.text as string,
        timestamp: new Date(),
        outputType: { type: "text" },
      };
      const paneType = data.pane_type as string | undefined;
      const paneId = data.pane_id as number | string | undefined;
      updatePaneModeHint(set, get, paneType, paneId);
      addMessageWithPaneRouting(set, get, userMessage, paneType, paneId);
      break;
    }

    case "stream_message": {
      const msgSessionId = data.session_id as string | undefined;
      const { sessionId: currentSessionId } = get();
      if (msgSessionId && currentSessionId && msgSessionId !== currentSessionId) {
        break;
      }

      const msg = data.message as Record<string, unknown>;
      if (!msg) break;

      const paneType = data.pane_type as string | undefined;
      const paneId = data.pane_id as number | string | undefined;
      updatePaneModeHint(set, get, paneType, paneId);
      const msgType = msg.type as string;
      if (msgType === "assistant") {
        const message = msg.message as Record<string, unknown>;
        const content = message?.content as Array<Record<string, unknown>>;
        if (content) {
          for (const block of content) {
            if (block.type === "text") {
              const assistantMessage: Message = {
                id: generateId(),
                role: "assistant",
                content: block.text as string,
                timestamp: new Date(),
                outputType: { type: "text" },
              };
              addMessageWithPaneRouting(set, get, assistantMessage, paneType, paneId);
            } else if (block.type === "tool_use") {
              // Store id→name mapping so tool_result can look it up
              if (block.id && block.name) {
                toolNameMap.set(block.id as string, block.name as string);
              }
              const toolMessage: Message = {
                id: generateId(),
                role: "assistant",
                content: `Using ${block.name}: ${JSON.stringify(block.input)}`,
                timestamp: new Date(),
                outputType: {
                  type: "tool_use",
                  tool: block.name as string,
                  input: block.input,
                  toolUseId: block.id as string | undefined,
                },
              };
              addMessageWithPaneRouting(set, get, toolMessage, paneType, paneId);
            }
          }
        }
      } else if (msgType === "user") {
        const message = msg.message as Record<string, unknown>;
        const content = message?.content as Array<Record<string, unknown>>;
        // Claude tucks AskUserQuestion's structured answer payload into
        // `tool_use_result` at the top of the user stream message (not in
        // the tool_result content block). Hoist it so the card can flip to
        // its "answered" state and persist across reloads.
        const toolUseResult = msg.tool_use_result as Record<string, unknown> | undefined;
        if (toolUseResult && content) {
          const answers = toolUseResult.answers as Record<string, string> | undefined;
          if (answers && typeof answers === "object") {
            for (const block of content) {
              if (block.type === "tool_result") {
                const toolUseId = block.tool_use_id as string | undefined;
                if (toolUseId && toolNameMap.get(toolUseId) === "AskUserQuestion") {
                  set((state) => {
                    if (state.answeredQuestions.has(toolUseId)) {
                      return {};
                    }
                    const next = new Map(state.answeredQuestions);
                    next.set(toolUseId, answers);
                    return { answeredQuestions: next };
                  });
                }
              }
            }
          }
        }
        if (content) {
          for (const block of content) {
            if (block.type === "tool_result") {
              const toolUseId = block.tool_use_id as string;
              const toolResultMessage: Message = {
                id: generateId(),
                role: "assistant",
                content: block.content as string || "",
                timestamp: new Date(),
                outputType: {
                  type: "tool_result",
                  tool: toolNameMap.get(toolUseId) || toolUseId,
                  success: !(block.is_error as boolean),
                },
              };
              addMessageWithPaneRouting(set, get, toolResultMessage, paneType, paneId);
            }
          }
        }
      } else if (msgType === "result") {
        const subtype = msg.subtype as string || "result";
        const resultContent = msg.result as string | undefined;
        const cost = (msg.total_cost_usd as number || 0).toFixed(4);
        const duration = msg.duration_ms;

        // Threshold for "substantial" content that shouldn't be crammed into system metadata.
        // Multi-line content or content > 150 chars is treated as actual response content
        // (MiniMax sends full responses in the result field instead of via Assistant messages).
        const isSubstantialContent = resultContent && (
          resultContent.includes("\n") ||
          resultContent.length > 150
        );

        if (isSubstantialContent) {
          // Only create an assistant message if the text wasn't already streamed
          // via an earlier "assistant" event (Claude/Codex stream text blocks first,
          // but MiniMax only puts the full response in the result field).
          const normalizedPaneId = normalizePaneId(paneType, normalizeRawPaneId(paneId));
          const existing = normalizedPaneId
            ? (get().paneMessages[paneKey(normalizedPaneId)] || [])
            : get().messages;
          // Check recent messages (not just last — tool calls may interleave)
          const recentSlice = existing.slice(-10);
          const alreadyStreamed = recentSlice.some(
            (m) => m.role === "assistant"
              && m.outputType?.type === "text"
              && m.content === resultContent
          );

          if (!alreadyStreamed) {
            const assistantMessage: Message = {
              id: generateId(),
              role: "assistant",
              content: resultContent,
              timestamp: new Date(),
              outputType: { type: "text" },
            };
            addMessageWithPaneRouting(set, get, assistantMessage, paneType, paneId);
          }
        }

        // Always add a brief system message with metadata
        const systemMeta = `${subtype} - Cost: $${cost}, Duration: ${duration}ms`;
        const resultMessage: Message = {
          id: generateId(),
          role: "system",
          content: systemMeta,
          timestamp: new Date(),
          outputType: { type: "system" },
        };
        addMessageWithPaneRouting(set, get, resultMessage, paneType, paneId);
      }
      break;
    }

    case "deadloop_status": {
      const isPaused = data.is_paused as boolean;
      console.log("Deadloop status update:", isPaused ? "paused" : "running");
      set((state) => ({
        isDeadloopPaused: isPaused,
        pausedPanes: isPaused
          ? [...state.pausedPanes.filter(p => p !== PANE_ID_DEADLOOP), PANE_ID_DEADLOOP]
          : state.pausedPanes.filter(p => p !== PANE_ID_DEADLOOP),
      }));
      break;
    }

    case "pane_paused": {
      const paneId = data.pane_id as number;
      const isPaused = data.is_paused as boolean;
      console.log(`Pane ${paneId} ${isPaused ? "paused" : "resumed"}`);
      set((state) => ({
        pausedPanes: isPaused
          ? [...state.pausedPanes.filter(p => p !== paneId), paneId]
          : state.pausedPanes.filter(p => p !== paneId),
        // Legacy compat
        isDeadloopPaused: paneId === PANE_ID_DEADLOOP ? isPaused : state.isDeadloopPaused,
      }));
      break;
    }

    case "session_download": {
      const sessionId = data.session_id as string;
      const projectId = data.project_id as string | undefined;
      const messages = data.messages as Array<Record<string, unknown>> || [];
      const workingDir = data.working_dir as string | undefined;
      const hostname = data.hostname as string | undefined;
      const createdAt = data.created_at as string | undefined;

      const downloadData = {
        session_id: sessionId,
        project_id: projectId,
        working_dir: workingDir,
        hostname: hostname,
        created_at: createdAt,
        exported_at: new Date().toISOString(),
        message_count: messages.length,
        messages: messages,
      };

      const blob = new Blob([JSON.stringify(downloadData, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `apas-session-${sessionId.slice(0, 8)}-${new Date().toISOString().slice(0, 10)}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);

      console.log(`Downloaded session ${sessionId} with ${messages.length} messages`);
      break;
    }

    case "usage_limits": {
      const cliClientId = data.cli_client_id as string;
      const limits = data.limits as Record<string, unknown>;
      if (cliClientId && limits) {
        const directProvider = normalizeProvider((data as Record<string, unknown>).provider);
        const provider =
          directProvider ??
          inferUsageProvider(cliClientId, get().paneConfigs);

        if (directProvider) {
          usageProviderHints.set(cliClientId, directProvider);
        }

        if (!directProvider) {
          console.warn("Usage limits missing provider; inferred provider:", provider, data);
        }

        const toWindow = (raw: unknown) => {
          if (!raw || typeof raw !== "object") return undefined;
          const window = raw as Record<string, unknown>;
          const utilization = typeof window.utilization === "number" ? window.utilization : undefined;
          if (utilization === undefined) return undefined;
          const resetRaw =
            window.resets_at ??
            window.reset_at ??
            window.resetsAt ??
            window.resetAt;
          return {
            utilization,
            resetsAt: typeof resetRaw === "string" ? resetRaw : undefined,
          };
        };

        const parsedLimits: UsageLimits = {
          fiveHour: toWindow(limits.five_hour ?? limits.fiveHour),
          sevenDay: toWindow(limits.seven_day ?? limits.sevenDay),
          fetchedAt:
            (typeof limits.fetched_at === "string" ? limits.fetched_at : undefined) ??
            (typeof limits.fetchedAt === "string" ? limits.fetchedAt : undefined),
        };
        set((state) => {
          const newMap = new Map(state.usageLimits);
          const existing = newMap.get(cliClientId) ?? {};
          newMap.set(cliClientId, { ...existing, [provider]: parsedLimits });
          return { usageLimits: newMap };
        });
        console.log("Usage limits updated for CLI:", cliClientId, provider, parsedLimits);
      }
      break;
    }

    default:
      console.log("Unknown message type:", data.type);
  }
}

function parseOutputType(data: Record<string, unknown> | undefined): OutputType {
  if (!data) return { type: "text" };

  switch (data.type || Object.keys(data)[0]) {
    case "text":
      return { type: "text" };
    case "code":
      return { type: "code", language: data.language as string | undefined };
    case "tool_use":
      return {
        type: "tool_use",
        tool: data.tool as string,
        input: data.input,
        toolUseId: data.tool_use_id as string | undefined,
      };
    case "tool_result":
      return { type: "tool_result", tool: data.tool as string, success: data.success as boolean };
    case "approval_request":
      return {
        type: "approval_request",
        toolCallId: data.tool_call_id as string,
        tool: data.tool as string,
        description: data.description as string,
      };
    case "system":
      return { type: "system" };
    case "error":
      return { type: "error" };
    default:
      return { type: "text" };
  }
}
