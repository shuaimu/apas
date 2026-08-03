import { create } from "zustand";
import { decodeBase64, emitTerminal, encodeBase64 } from "./terminalBus";
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
  /** Canonical `host/owner/repo` of the project's git `origin` remote. The
   * sidebar groups projects that share this value under one repo header.
   * Undefined means "no remote" (its own sidebar group). */
  gitRemote?: string;
  /** Raw cloneable `origin` URL, used to prefill the clone URL when creating
   * a new instance under this repo. */
  gitRemoteUrl?: string;
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

export type Provider = "claude" | "codex" | "minimax" | "glm" | "deepseek" | "opencode" | "cursor-agent";

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
  deepseekBackend?: {
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
  /// Server's `created_at` for the most recent message represented in this
  /// snapshot. Persisted to IDB so that a tab restored from a previous
  /// browser session can fire a catchup with the right watermark instead
  /// of silently showing stale content forever (the in-memory
  /// `sessionLastCreatedAt` is fresh on every page load and cannot recover
  /// this on its own).
  lastCreatedAt?: string;
  /// Per-pane catchup watermarks (`String(paneId)` → server `created_at`)
  /// captured at snapshot time. Persisted so a page reload can fire the
  /// PRECISE per-pane catchup (`get_messages_per_pane_after`) instead of
  /// the session-level MIN cutoff. That MIN is dragged far into the past
  /// by any long-idle pane, and the server caps a single-cutoff catchup
  /// at 500 rows across ALL panes — so an actively-updating pane's newest
  /// messages can fall outside that window and never repaint after a
  /// reload (previously the user had to clear IndexedDB to see them).
  /// Per-pane watermarks fetch each pane's own tail, immune to both.
  paneLastCreatedAt?: Record<string, string>;
  /// Latest `team-todo.md` snapshot for this session. Persisted so the
  /// Overview's TODO panel renders the right content immediately on
  /// refresh instead of going through the fetch round-trip (which can
  /// silently fail when the CLI is briefly disconnected or slow).
  teamTodoState?: TeamTodoState;
  /// Latest `suggested-workers.md` snapshot for this session. Same
  /// reasoning as `teamTodoState`.
  suggestedWorkers?: SuggestedWorker[];
}

/// Wire shape of team-todo.md (mirrors shared::TeamTodoStateMsg).
/// Statuses are kept as strings on the wire so the web doesn't have to
/// re-derive the enum mapping.
export interface TeamTodoState {
  globals: TeamTodoGlobal[];
  workers: TeamTodoWorker[];
  /// Per-agent scratchpad cursor (RFC3339 timestamp of the last
  /// scratchpad record they acted on). `null` means the cursor file
  /// is missing — agent hasn't iterated yet (or was wiped).
  tech_lead_cursor?: string | null;
  reviewer_cursor?: string | null;
}

export interface TeamTodoGlobal {
  id: string;
  title: string;
  /// proposed | approved | in_progress | under_review | pr_open | done | rejected | withdrawn
  status: string;
  /// user | tech-lead
  origin: string;
  /// One PR per contributing worker pane. Empty until any worker's
  /// branch has been pushed and PR'd.
  prs: TeamTodoPaneTodoPr[];
  body: string;
}

export interface TeamTodoPaneTodoPr {
  pane_id: number;
  url: string;
  annotation?: string;
}

/// Wire shape of one entry in suggested-workers.md. Manager pane writes
/// these; Overview renders each as a card with Accept/Dismiss.
/// A user `input` we've sent but haven't yet seen the server echo back
/// as `user_input`. Persisted to localStorage so refresh / reconnect
/// can replay it. Removed once the matching echo arrives.
export interface PendingSend {
  /// Client-generated id. Matches the optimistic message's id (minus
  /// the `optimistic-` prefix) so ack handlers can link them.
  id: string;
  /// Target session — replay is only meaningful while the user is
  /// viewing this session.
  sessionId: string;
  /// Target pane within that session (null = no specific pane).
  paneId: number | null;
  paneType?: string;
  text: string;
  /// ms epoch when first enqueued.
  createdAt: number;
  /// How many times we've sent this. >1 means a reconnect retransmit.
  attempts: number;
}

export interface SuggestedWorker {
  id: string;
  label: string;
  role: string;
  goal: string;
  backstory: string;
  needs_worktree: boolean;
}

export interface TeamTodoWorker {
  pane_id: number;
  role_hint?: string | null;
  subtasks: TeamTodoSubtask[];
}

export interface TeamTodoSubtask {
  id: string;
  title: string;
  /// pending | in_progress | done | reviewing | revising | approved
  status: string;
  parent: string;
  body: string;
}

export type PaneType = "deadloop" | "interactive";

/** Aggregated usage counters for one time window. snake_case to match the
 *  server's ServerToWeb::ProjectUsageStats payload verbatim. */
export interface UsageCounters {
  prompts: number;
  responses: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  cost_usd: number;
}

export interface PaneUsageStats {
  pane_id: number;
  lifetime: UsageCounters;
  last_7d: UsageCounters;
  today: UsageCounters;
  last_active?: string;
}

export interface ProjectUsageStats {
  panes: PaneUsageStats[];
  lifetime: UsageCounters;
  last_7d: UsageCounters;
  today: UsageCounters;
  last_active?: string;
}

/** How a pane hosts its agent. Mirrors `shared::PaneKind`.
 *  - "agent": headless stream-json worker (default, everything today).
 *  - "terminal": the provider's real TUI on a pty, rendered by xterm.js.
 *    Terminal panes publish no stream events, so they have no usage
 *    stats, status, diffs, or Tech Lead delegation. */
export type PaneKind = "agent" | "terminal";

export interface PaneConfig {
  pane_id: number;
  provider: Provider;
  mode: "deadloop" | "interactive";
  /** Absent on panes persisted before terminal panes existed → "agent". */
  kind?: PaneKind;
  session_id: string;
  is_paused: boolean;
  stop_requested?: boolean;
  prompt?: string;
  min_iteration_interval_minutes?: number;
  label?: string;
  model?: string;
  effort?: string;
  worktree_path?: string;
  role?: string;
  goal?: string;
  backstory?: string;
  plan_review_mode?: PlanReviewMode;
  /** v3.2 — worker mode. false (default) = autonomous, available for
   *  Tech-Lead delegation. true = manual, only the user chats with it. */
  manual_mode?: boolean;
  /** v3.5 — true = part of the team (Tech Lead can delegate, shows on
   *  Overview Pane Grid). false (default for legacy / TabBar + side
   *  chats) = not part of the team queue. */
  managed?: boolean;
}

export type PaneCleanupAction = "discard" | "merge_and_remove" | "leave_as_branch";

// Legacy pane_id constants (must match shared::PANE_ID_DEADLOOP / PANE_ID_INTERACTIVE)
export const PANE_ID_DEADLOOP = 1;
export const PANE_ID_INTERACTIVE = 2;
const usageProviderHints = new Map<string, Provider>();

export function storeDebugLog(...args: Parameters<typeof console.log>) {
  if (process.env.NODE_ENV !== "production") {
    console.log(...args);
  }
}

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
    if (normalized === "deepseek" || normalized === "deep_seek" || normalized === "deep-seek") return "deepseek";
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
    if (seenProviders.has("deepseek")) {
      usageProviderHints.set(cliClientId, "deepseek");
      return "deepseek";
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

  // Sessions that have received `stream_message`/`user_input` events
  // while the user wasn't viewing them — used to drive the "new
  // activity" indicator in the sidebar. Cleared when the user attaches
  // to that session.
  unreadSessions: Set<string>;

  // Reconnect catchup: per-session high-water mark of server `created_at`
  // values seen via stream_message. After a WS drop we ask the server for
  // `after_created_at = sessionLastCreatedAt[sid]` so the missing tail gets
  // appended to live state instead of being silently dropped when
  // `route_to_web` couldn't reach the disconnected old connection_id.
  sessionLastCreatedAt: Map<string, string>;

  /// Per-pane "first fetch in flight" set. The attach response no
  /// longer carries every pane's tail (lazy-load mode) — each tab
  /// fetches its own messages on first open via
  /// `loadPaneMessagesIfNeeded`. Tracking in-flight prevents
  /// double-fetch when the user clicks the same tab twice quickly
  /// or when React effects re-fire.
  paneLoadingInitial: Set<number>;

  /// Per-pane watermark map (sessionId → paneId → server created_at).
  /// Used to derive `sessionLastCreatedAt[sid]` as the MIN across
  /// panes — that's the only safe catchup point. If a fast pane (e.g.
  /// the Tech Lead deadloop) keeps streaming, its created_at would
  /// dominate a session-wide MAX and a `after_created_at=MAX` catchup
  /// query would return nothing for slower panes whose last message
  /// is hours behind. The MIN semantics means catchup fetches all
  /// messages since the slowest pane last spoke — the client dedupes
  /// by id, so the extra payload is harmless.
  paneLastCreatedAt: Map<string, Map<number, string>>;

  // Frozen pre-reconnect watermark, snapshotted from `sessionLastCreatedAt`
  // the moment the WS reauthenticates. Catchup uses this in preference to
  // the live watermark so that a stream_message that arrives between
  // reconnect and the user clicking a background tab can't advance the
  // watermark past the disconnect-window messages still sitting on disk.
  // Cleared per-session once that session's catchup reply lands.
  reconnectWatermarks: Map<string, string>;

  /// Outgoing sends the server hasn't acked yet (no `user_input` echo
  /// received). Persisted to localStorage so a page refresh doesn't
  /// lose the typed input. Replayed on every WS authenticate.
  /// Removed when the matching `user_input` arrives.
  pendingSends: PendingSend[];

  /// Persisted queue of AskUserQuestion answers waiting for claude to
  /// actually process them. Replayed on every WS authenticate so a
  /// dropped submission (silently-stale WS, server rejected forward,
  /// CLI wasn't attached) eventually lands. Removed when the answered
  /// tool_result arrives with the {answers} payload in its
  /// tool_use_result.
  pendingAnswers: PendingAnswer[];

  /// Persisted queue of pane-label renames waiting for the CLI to ack
  /// (next PaneList arrives with the matching label). Prevents "reboot
  /// lost my rename" when the update_pane_label WS frame is dropped
  /// silently.
  pendingLabels: PendingLabel[];

  /** Per-session snapshot of suggested-workers.md. Pushed by the CLI on
   *  FetchSuggestedWorkers, after Dismiss mutations, and via the CLI's
   *  mtime-gated poller. Keyed by session_id so switching projects shows
   *  the right project's suggestions immediately (rather than the
   *  previous project's data lingering until the new CLI replies). A
   *  missing key = haven't fetched yet; `[]` = file empty / no
   *  suggestions. */
  suggestedWorkersBySession: Map<string, SuggestedWorker[]>;

  /** Per-session snapshot of team-todo.md. Same shape + reasoning as
   *  `suggestedWorkersBySession`. Pushed in response to fetchTeamTodo()
   *  and after TodoApproval / AddTodo mutations, plus the CLI's mtime
   *  poller. Missing key = not fetched yet. */
  teamTodoStates: Map<string, TeamTodoState>;

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
  createProjectInstance: (
    machineId: string,
    gitRemote: string,
    instanceName: string,
    branch: string,
    cloneUrl?: string,
    basePath?: string,
  ) => boolean;
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
  setMachineDeepseekConfig: (
    machineId: string,
    apiKey?: string,
    clearApiKey?: boolean,
  ) => void;
  listSessions: () => void;
  loadSessionMessages: (sessionId: string) => void;
  loadMoreMessages: (pane?: PaneType | number) => void;
  /** Lazy-load mode: fetch this pane's messages on first tab activation
   *  if we haven't already. Server's attach reply doesn't ship every
   *  pane's tail anymore; this action requests only the active pane
   *  via `get_session_messages` with `pane_id`. No-op if the pane is
   *  already loaded or a fetch is in flight. */
  loadPaneMessagesIfNeeded: (paneId: number) => void;
  /** Re-fetch a pane's newest contiguous slice and reconcile it as a
   *  sliding window (overwrites the recent range, keeps older history) so a
   *  reconnect/reload heals any hole left below the catchup watermark. */
  refreshPaneWindow: (paneId: number, limit?: number) => void;
  prependMessages: (messages: Message[], hasMore: boolean) => void;
  sendMessageToPane: (text: string, pane: PaneType | number) => { success: boolean; error?: string };
  addMessageToPane: (message: Message, pane: PaneType | number) => void;
  startAutoRefresh: () => void;
  stopAutoRefresh: () => void;
  pauseDeadloop: () => void;
  resumeDeadloop: () => void;
  pausePane: (paneId: number) => void;
  resumePane: (paneId: number) => void;
  rebootPane: (paneId: number) => void;
  addPane: (
    provider: string,
    mode: string,
    label?: string,
    prompt?: string,
    model?: string,
    isolatedWorktree?: boolean,
    initialRole?: {
      role?: string;
      goal?: string;
      backstory?: string;
      planReviewMode?: PlanReviewMode;
    },
    managed?: boolean,
    kind?: PaneKind,
  ) => { success: boolean; error?: string };
  removePane: (paneId: number, cleanupAction?: PaneCleanupAction) => void;
  updatePaneLabel: (paneId: number, label: string) => void;
  updatePaneEffort: (paneId: number, effort: string | null) => void;
  /** Switch a pane's agent backend (provider + model). Pass `provider`
   *  to swap the underlying CLI (claude / codex / cursor-agent /
   *  opencode / minimax / glm); pass `null` to keep current. Pass
   *  `model` to set / `null` to clear back to the provider's default.
   *  Kills the current agent child + respawns the worker on a fresh
   *  session id; chat history stays visible client-side but is NOT
   *  in the new agent's prompt. */
  updatePaneModel: (
    paneId: number,
    model: string | null,
    provider?: string | null,
  ) => void;
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
  requestPaneDiff: (paneId: number) => void;
  paneDiffs: Record<number, PaneDiff>;
  createPanePr: (paneId: number) => void;
  /** v3.1 — current project_goal.md content per session id, mirrored
   *  from the CLI's mtime poller. Used by ProjectGoalBar to hydrate the
   *  textbox when the user isn't editing. */
  projectGoals: Record<string, string>;
  /** Per-session usage stats (prompts/tokens/cost) for the Overview panel,
   *  keyed by session_id. Pushed live and replayed on attach. */
  usageStats: Record<string, ProjectUsageStats>;
  /** Tech-Lead autonomy flags per session, mirrored from the CLI's
   *  `.apas` poller. Toggled from the Overview. */
  projectFlags: Record<
    string,
    { autoApproveTodos: boolean; autoMergePrs: boolean; teamEnabled: boolean }
  >;
  /** Manager v2 — overwrite project_goal.md at the project root. */
  updateProjectGoal: (goal: string) => void;
  /** Push new Tech-Lead autonomy flags to the CLI. */
  updateProjectFlags: (flags: {
    autoApproveTodos: boolean;
    autoMergePrs: boolean;
    teamEnabled: boolean;
  }) => void;
  /** Spawn the default team panes for any role that isn't already
   *  present. Idempotent on the CLI side. Each role's `provider` /
   *  `model` come from the Team setup card; null falls back to the
   *  CLI default (Claude / unset). */
  startTeam: (specs: {
    manager: { provider: string; model: string | null };
    techLead: { provider: string; model: string | null };
    reviewer: { provider: string; model: string | null };
    developer: { provider: string; model: string | null };
  }) => void;
  updatePaneRole: (paneId: number, role?: string, goal?: string, backstory?: string) => void;
  /** Scratchpad records keyed by server session id. `teamRecords` is the
   *  active-session projection retained for existing components/tests. */
  teamRecordsBySession: Map<string, TeamRecord[]>;
  teamRecords: TeamRecord[];
  planReviewPending: PlanReviewPendingItem[];
  answerPlanReview: (toolUseId: string, approve: boolean) => void;
  updatePaneReviewMode: (paneId: number, mode: PlanReviewMode) => void;
  /** v3.2 — flip a worker between autonomous and manual modes. */
  updatePaneManualMode: (paneId: number, manualMode: boolean) => void;

  /** v3.5 — one-way promote: turn an unmanaged side-chat pane into
   *  a team member the Tech Lead can delegate to. There's no demote. */
  promotePaneToManaged: (paneId: number) => void;

  /** Terminal panes (PaneKind "terminal"). Frames themselves arrive via
   *  terminalBus, not through store state — see that module for why. */
  attachTerminal: (paneId: number) => void;
  sendTerminalInput: (paneId: number, data: string) => void;
  sendTerminalResize: (paneId: number, cols: number, rows: number) => void;

  /** Ask the server (which asks the CLI) for the current team-todo.md.
   *  Reply lands in `teamTodoState` via the team_todo_state handler. */
  fetchTeamTodo: () => void;
  /** Approve a proposed Global TODO (state machine: proposed → approved). */
  approveTodo: (todoId: string) => void;
  /** Reject a proposed Global TODO (state machine: proposed → rejected). */
  rejectTodo: (todoId: string) => void;
  /** Add a new Global TODO (status: approved, origin: user). CLI assigns the id. */
  addTodo: (title: string, body: string) => void;

  /** Ask the CLI for the current suggested-workers.md snapshot. Reply
   *  lands in `suggestedWorkers` via the suggested_workers_state handler. */
  fetchSuggestedWorkers: () => void;
  /** Spawn the suggested worker as a managed pane + drop the section. */
  acceptSuggestion: (suggestion: SuggestedWorker) => void;
  /** Drop the suggestion without spawning anything. */
  dismissSuggestion: (suggestionId: string) => void;
}

export interface PaneDiff {
  branch?: string;
  base?: string;
  diff?: string;
  error?: string;
  fetchedAt: number;
}

export interface TeamRecord {
  ts: string;
  pane_id?: number;
  tags: string[];
  kind: string;
  body: string;
}

export function selectActiveTeamRecords(state: {
  sessionId: string | null;
  teamRecordsBySession: Map<string, TeamRecord[]>;
  teamRecords: TeamRecord[];
}): TeamRecord[] {
  if (!state.sessionId) return state.teamRecords;
  return state.teamRecordsBySession.get(state.sessionId) ?? state.teamRecords;
}

export type PlanReviewMode = "always" | "risky_only" | "never";

export interface PlanReviewPendingItem {
  paneId: number;
  toolUseId: string;
  toolName: string;
  input: unknown;
  arrivedAt: number;
}

const WS_URL = process.env.NEXT_PUBLIC_WS_URL || "ws://apas.mpaxos.com";
const MINIMAX_API_BASE_URL = "https://api.minimax.io/anthropic";
const GLM_API_BASE_URL = "https://api.z.ai/api/anthropic";
const DEEPSEEK_API_BASE_URL = "https://api.deepseek.com/anthropic";

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
  paneDiffs: {},
  projectGoals: {},
  usageStats: {},
  projectFlags: {},
  teamRecordsBySession: new Map(),
  teamRecords: [],
  planReviewPending: [],
  answeredQuestions: loadAnsweredQuestions(),
  toasts: [],
  sessionCache: new Map(),
  unreadSessions: new Set(),
  sessionLastCreatedAt: new Map(),
  paneLastCreatedAt: new Map(),
  paneLoadingInitial: new Set(),
  reconnectWatermarks: new Map(),
  pendingSends: loadPendingSends(),
  pendingAnswers: loadPendingAnswers(),
  pendingLabels: loadPendingLabels(),
  teamTodoStates: new Map(),
  suggestedWorkersBySession: new Map(),
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
      teamRecordsBySession: new Map(),
      teamRecords: [],
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
      storeDebugLog("No token found, cannot connect");
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

    // Liveness loop: a silently-stale WS (mobile OS throttling, NAT
    // timeout, server-side TCP RST swallowed) keeps readyState === OPEN
    // forever — ws.send() never throws and ws.onclose never fires. The
    // browser thinks it's connected; the server has moved on; messages
    // get dropped in both directions. Counter that by tracking inbound
    // liveness: every tick, send a Heartbeat (which the server echoes)
    // and check that we've seen *any* frame in the last `livenessMs`.
    // If not, close — onclose drives the existing reconnect path.
    let lastIncomingAt = Date.now();
    const heartbeatMs = 5_000;
    // Liveness window has to be comfortably larger than the worst-case
    // single-frame download — for big projects (1+ GB messages.jsonl
    // with multi-MB tool_result dumps) a single session_messages reply
    // can take 10-20s to arrive on cellular. The browser only fires
    // onmessage once the full frame lands, so `lastIncomingAt` doesn't
    // tick during the download. 30s gives headroom; truly-dead WSes
    // still surface within that window because heartbeat pings keep
    // round-tripping under normal conditions.
    const livenessMs = 30_000;
    const heartbeatHandle = setInterval(() => {
      if (ws.readyState !== WebSocket.OPEN) return;
      try {
        ws.send(JSON.stringify({ type: "heartbeat" }));
      } catch {
        // send throws only on rare states (CLOSING); onclose will fire shortly.
      }
      if (Date.now() - lastIncomingAt > livenessMs) {
        console.warn(
          `[ws] no inbound frame in ${Math.round((Date.now() - lastIncomingAt) / 1000)}s — closing to force reconnect`,
        );
        try {
          ws.close();
        } catch {
          // ignore — onclose handler still runs
        }
      }
    }, heartbeatMs);

    ws.onopen = () => {
      storeDebugLog("WebSocket connected, sending authentication...");
      // Reset reconnect attempts on successful connection
      set({ reconnectAttempts: 0 });
      lastIncomingAt = Date.now();
      // Send token for authentication
      ws.send(JSON.stringify({ type: "authenticate", token }));
    };

    ws.onmessage = (event) => {
      lastIncomingAt = Date.now();
      try {
        const data = JSON.parse(event.data);
        handleServerMessage(data, set, get);
      } catch (e) {
        console.error("Failed to parse message:", e);
      }
    };

    ws.onclose = (event) => {
      storeDebugLog("WebSocket disconnected", event.code, event.reason);
      clearInterval(heartbeatHandle);
      set({ connected: false, ws: null, cliClients: [], isAttached: false });

      // Auto-reconnect with exponential backoff (unless intentionally disconnected)
      if (event.code !== 1000) {
        const { reconnectAttempts } = get();
        const maxAttempts = 10;
        if (reconnectAttempts < maxAttempts) {
          const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000);
          storeDebugLog(`Scheduling reconnect attempt ${reconnectAttempts + 1} in ${delay}ms`);
          const timeout = setTimeout(() => {
            storeDebugLog(`Reconnect attempt ${reconnectAttempts + 1}`);
            set({ reconnectAttempts: reconnectAttempts + 1 });
            get().connect();
          }, delay);
          set({ reconnectTimeout: timeout });
        } else {
          storeDebugLog("Max reconnect attempts reached");
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
          storeDebugLog("App became visible, checking connection...", { connected, isAttached, sessionId });
          if (!connected || !ws || ws.readyState !== WebSocket.OPEN) {
            storeDebugLog("Connection lost while in background, reconnecting...");
            set({ reconnectAttempts: 0 });
            get().connect();
          } else {
            // WS *looks* alive — but a silently-stale WS (mobile OS
            // throttling, power saving, sleep/wake cycles) can swallow
            // stream_messages without ever triggering onclose. Trusting
            // readyState alone left users staring at stale tabs after
            // unhiding. Refresh ancillary state AND fire a tail catchup
            // for the current session so any messages we missed while
            // backgrounded land before the user starts reading.
            storeDebugLog("Connection appears healthy, refreshing data...");
            get().refreshCliClients();
            get().listSessions();
            if (sessionId) {
              requestCatchupIfNeeded(get, sessionId);
            }
            // If sessionId is set but isAttached is false (server-side
            // attachment got dropped for some reason), reattach without
            // forceReload — cache-first so the user keeps their messages.
            if (sessionId && !isAttached) {
              storeDebugLog("Session was detached server-side; soft re-attach...");
              get().attachSession(sessionId, false);
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
      teamRecords: [],
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
      teamRecords: [],
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
        const entry: SessionCacheEntry = {
          messages: state.messages,
          paneMessages: state.paneMessages,
          paneHasMore: state.paneHasMore,
          paneConfigs: state.paneConfigs,
          paneModes: state.paneModes,
          hasMoreMessages: state.hasMoreMessages,
          isDualPane: state.isDualPane,
          answeredQuestions: state.answeredQuestions,
          cachedAt: Date.now(),
          // Carry the catchup watermark into the snapshot so a future
          // page load can ask the server for messages newer than this
          // instead of falsely trusting the stale cache forever.
          lastCreatedAt: state.sessionLastCreatedAt.get(currentSessionId),
          paneLastCreatedAt: paneWatermarksToRecord(
            state.paneLastCreatedAt.get(currentSessionId),
          ),
          teamTodoState: state.teamTodoStates.get(currentSessionId),
          suggestedWorkers: state.suggestedWorkersBySession.get(currentSessionId),
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
      // Drop the unread indicator for the session we're navigating into.
      const unreadSessions = state.unreadSessions.has(sessionId)
        ? (() => {
            const next = new Set(state.unreadSessions);
            next.delete(sessionId);
            return next;
          })()
        : state.unreadSessions;
      if (cached) {
        set({
          sessionId,
          cliClientId: newCliClientId,
          teamRecords: state.teamRecordsBySession.get(sessionId) ?? [],
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
          paneLoadingInitial: new Set(),
          sessionCache,
          unreadSessions,
        });
      } else {
        set({
          sessionId,
          cliClientId: newCliClientId,
          teamRecords: state.teamRecordsBySession.get(sessionId) ?? [],
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
          unreadSessions,
        });
      }
    } else {
      set((state) => ({
        isAttached: hasActiveClient,
        cliClientId: newCliClientId,
        teamRecords: state.teamRecordsBySession.get(sessionId) ?? [],
      }));
    }

    ws.send(JSON.stringify({
      type: "attach_session",
      session_id: sessionId
    }));

    // If we restored from cache, the live attach reply gets dedupe-skipped
    // ("live state wins") and the gap that landed while this tab was a
    // background session stays empty. Catchup fills it lazily — at most
    // one per-tab attach, naturally rate-limited by user clicks rather
    // than fired in parallel across every cached session on reconnect.
    if (!isSameSession && !forceReload && state.sessionCache.has(sessionId)) {
      requestCatchupIfNeeded(get, sessionId);
    }
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

  createProjectInstance: (
    machineId: string,
    gitRemote: string,
    instanceName: string,
    branch: string,
    cloneUrl?: string,
    basePath?: string,
  ): boolean => {
    const { ws, showToast } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      showToast("Not connected — try again in a moment", "error");
      return false;
    }
    const requestId =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `req-${Date.now()}`;
    ws.send(JSON.stringify({
      type: "create_project_instance",
      machine_id: machineId,
      git_remote: gitRemote,
      instance_name: instanceName,
      branch,
      clone_url: cloneUrl || undefined,
      base_path: basePath || undefined,
      request_id: requestId,
    }));
    return true;
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

  setMachineDeepseekConfig: (
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
        const existingBackend = entry.machine.deepseekBackend;
        const nextApiKey = clearApiKey
          ? undefined
          : (normalizedApiKey ?? existingBackend?.apiKey);
        return {
          ...entry,
          machine: {
            ...entry.machine,
            deepseekBackend: {
              apiBaseUrl: DEEPSEEK_API_BASE_URL,
              apiKey: nextApiKey,
              apiKeyConfigured: Boolean(nextApiKey),
            },
          },
        };
      }),
    }));

    ws.send(JSON.stringify({
      type: "set_machine_deepseek_config",
      machine_id: machineId,
      api_base_url: DEEPSEEK_API_BASE_URL,
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
    // Cap the initial all-panes load. The server returns up to `limit`
    // messages PER pane; with 6-8 managed panes the default of 100 means
    // 600-800 messages arrive in one frame and get parsed + markdown-
    // rendered synchronously on attach, freezing the tab on open. 30/pane
    // is plenty for the newest-message view; older history pages in on
    // scroll via loadMoreMessages.
    ws.send(JSON.stringify({ type: "get_session_messages", session_id: sessionId, limit: 30 }));
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

    // `session_id` is required for multi-attached connections: the server
    // routes pane-scoped messages on it instead of the connection's
    // last-attached session, which is non-deterministic. See store.ts:1623
    // (cached-session subscribe loop) and ws_web.rs:1328 (overwrites the
    // connection's tracked session on every attach).
    ws.send(JSON.stringify({ type: "input", session_id: sessionId, text }));
  },

  addMessage: (message: Message) => {
    set((state) => ({ messages: [...state.messages, message] }));
  },

  approve: (toolCallId: string) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "approve", session_id: sessionId, tool_call_id: toolCallId }));
    }
  },

  reject: (toolCallId: string) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reject", session_id: sessionId, tool_call_id: toolCallId }));
    }
  },

  answerQuestion: (toolUseId: string, answers: Record<string, string>) => {
    const { ws, sessionId, showToast } = get();
    const canSend = ws && ws.readyState === WebSocket.OPEN;
    if (canSend) {
      ws.send(
        JSON.stringify({
          type: "answer_question",
          // Route to the session this question belongs to (the one the
          // user is viewing). Without it the server falls back to the
          // connection's last-attached session, which the multi-session
          // fan-out drifts — sending the answer to the wrong project.
          session_id: sessionId,
          tool_use_id: toolUseId,
          answers,
        }),
      );
      showToast("Answer sent to Claude", "success");
    } else {
      showToast("Not connected — answer queued for retry", "info");
    }
    // Enqueue into the pending-answer queue AND persist immediately.
    // Reasons this needs to live in localStorage, not just in-memory:
    // (1) A page refresh right after submit was previously losing the
    //     submitted state — the card would ask again.
    // (2) A ws.send() on a "silently stale" socket (readyState OPEN but
    //     TCP dead) drops the frame. The pending-answer queue is
    //     flushed on every reconnect so the answer eventually lands.
    // Confirmed root cause on mako Claude-6: server messages.jsonl AND
    // claude's on-disk session jsonl both show a 16-min gap between
    // the AskUserQuestion tool_use and its cancel tool_result — the
    // answer never reached claude's stdin.
    set((state) => {
      const alreadyAnswered = new Map(state.answeredQuestions);
      alreadyAnswered.set(toolUseId, answers);
      saveAnsweredQuestions(alreadyAnswered);
      const filtered = state.pendingAnswers.filter((p) => p.toolUseId !== toolUseId);
      const next: PendingAnswer[] = [
        ...filtered,
        {
          toolUseId,
          answers,
          sessionId: sessionId ?? "",
          createdAt: Date.now(),
          attempts: canSend ? 1 : 0,
        },
      ];
      savePendingAnswers(next);
      return { answeredQuestions: alreadyAnswered, pendingAnswers: next };
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

  loadPaneMessagesIfNeeded: (paneId: number) => {
    const { ws, sessionId, paneMessages, paneLoadingInitial } = get();
    if (!sessionId) return;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    // `undefined` = never fetched. An empty array `[]` means the
    // pane was fetched and is genuinely empty — don't refetch.
    if (paneMessages[paneKey(paneId)] !== undefined) return;
    if (paneLoadingInitial.has(paneId)) return;
    // Reserve the in-flight slot AND seed the bucket as []. The seed
    // satisfies the "trust local" rule's `existing.length === 0`
    // accept-snapshot branch (so the reply still populates), and the
    // `!== undefined` check above blocks redundant re-fetches by
    // re-renders.
    set((state) => {
      const nextLoading = new Set(state.paneLoadingInitial);
      nextLoading.add(paneId);
      return {
        paneLoadingInitial: nextLoading,
        paneMessages: { ...state.paneMessages, [paneKey(paneId)]: [] },
      };
    });
    ws.send(
      JSON.stringify({
        type: "get_session_messages",
        session_id: sessionId,
        pane_id: paneId,
        limit: 30,
      }),
    );
    // Fallback: a pane that returns no messages won't appear in
    // paneMsgBuckets so the session_messages handler can't clear
    // its in-flight marker. Clear after 30s either way so a future
    // refresh attempt isn't blocked.
    const requestedSessionId = sessionId;
    setTimeout(() => {
      const cur = get();
      if (cur.sessionId !== requestedSessionId) return;
      if (!cur.paneLoadingInitial.has(paneId)) return;
      set((state) => {
        const next = new Set(state.paneLoadingInitial);
        next.delete(paneId);
        return { paneLoadingInitial: next };
      });
    }, 30_000);
  },

  refreshPaneWindow: (paneId: number, limit = 50) => {
    // Re-fetch the newest contiguous slice for a pane; the session_messages
    // handler reconciles it as a sliding window (see the isDualPane branch),
    // keeping older cached history and OVERWRITING the recent range. Unlike
    // loadPaneMessagesIfNeeded this fires even when the bucket is already
    // populated — it's how a reconnect/reload heals a hole left below the
    // watermark, which the after_created_at catchup can never backfill.
    const { ws, sessionId } = get();
    if (!sessionId) return;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (paneId < 0) return; // skip the Overview pseudo-tab sentinel
    ws.send(
      JSON.stringify({
        type: "get_session_messages",
        session_id: sessionId,
        pane_id: paneId,
        limit,
      }),
    );
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

    // One shared id for the optimistic placeholder + the pending-send
    // queue entry — lets the `user_input` ack handler strip the
    // optimistic prefix AND remove the pending entry in one shot.
    const sendId = generateId();

    // Optimistic local render: drop the message into the pane immediately so
    // the user sees it without waiting for the server's user_input bounce.
    // The "optimistic-" id prefix is the dedupe handshake — the user_input
    // handler claims the matching slot and strips the prefix instead of
    // pushing a duplicate.
    if (paneId != null) {
      const optimisticMsg: Message = {
        id: `optimistic-${sendId}`,
        role: "user",
        content: text,
        timestamp: new Date(),
        outputType: { type: "text" },
      };
      get().addMessageToPane(optimisticMsg, pane);
    }

    // Enqueue into the pending-send queue BEFORE the WS send. If the WS
    // is silently stale (readyState OPEN but TCP dead), ws.send drops
    // the frame and the heartbeat watchdog reconnects within ~35s —
    // flushPendingSends will retransmit on the new socket. Persisted to
    // localStorage so a page refresh during the dead window doesn't
    // also lose the typed input.
    if (sessionId) {
      const entry: PendingSend = {
        id: sendId,
        sessionId,
        paneId: typeof paneId === "number" ? paneId : null,
        paneType,
        text,
        createdAt: Date.now(),
        attempts: 1,
      };
      const nextPending = [...get().pendingSends, entry];
      savePendingSends(nextPending);
      set({ pendingSends: nextPending });

      // First-line retry: a server echo for a healthy WS lands in well
      // under a second. If 3s pass without one, retransmit on the same
      // WS — covers the rare case of a single dropped frame on an
      // otherwise alive connection. If the WS is genuinely dead, the
      // retry also goes to the void and the heartbeat watchdog
      // (5s/10s) will close + reconnect, at which point
      // flushPendingSends replays from localStorage.
      setTimeout(() => {
        const entry = get().pendingSends.find((p) => p.id === sendId);
        if (!entry || entry.attempts >= 3) return;
        const curWs = get().ws;
        if (!curWs || curWs.readyState !== WebSocket.OPEN) return;
        try {
          curWs.send(
            JSON.stringify({
              type: "input",
              session_id: entry.sessionId,
              text: entry.text,
              pane_type: entry.paneType,
              pane_id: entry.paneId,
              client_msg_id: entry.id,
            }),
          );
          console.warn(`[pending-send] no ack for ${sendId} in 3s — retransmit attempt ${entry.attempts + 1}`);
          set((state) => {
            const next = state.pendingSends.map((p) =>
              p.id === sendId ? { ...p, attempts: p.attempts + 1 } : p,
            );
            savePendingSends(next);
            return { pendingSends: next };
          });
        } catch {
          // ignore — onclose will trigger reconnect path
        }
      }, 3_000);

      // Secondary safety net: if neither the original send nor the +3s
      // retry got acked within 15s (e.g. dead WS that hadn't yet
      // tripped the watchdog when we sent, plus reconnect-and-replay
      // still slow), fire a tail catchup so any response sitting on
      // the server shows up.
      setTimeout(() => {
        const stillPending = get().pendingSends.some((p) => p.id === sendId);
        if (stillPending) {
          requestCatchupIfNeeded(get, sessionId);
        }
      }, 15_000);
    }

    ws.send(JSON.stringify({
      type: "input",
      session_id: sessionId,
      text,
      pane_type: paneType, // Legacy compat: must be "deadloop" or "interactive"
      pane_id: paneId,
      // Idempotency key: retransmits (3s retry / reconnect replay) carry
      // the same id so the server drops them instead of double-storing.
      client_msg_id: sendId,
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
        storeDebugLog("WebSocket not in OPEN state, triggering reconnect...");
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
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "pause_deadloop", session_id: get().sessionId }));
      // Also send new pane-specific pause
      ws.send(JSON.stringify({ type: "pause_pane", session_id: sessionId, pane_id: PANE_ID_DEADLOOP }));
    }
  },

  resumeDeadloop: () => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "resume_deadloop", session_id: get().sessionId }));
      // Also send new pane-specific resume
      ws.send(JSON.stringify({ type: "resume_pane", session_id: sessionId, pane_id: PANE_ID_DEADLOOP }));
    }
  },

  pausePane: (paneId: number) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "pause_pane", session_id: sessionId, pane_id: paneId }));
      // Also send legacy message for backward compat
      if (paneId === PANE_ID_DEADLOOP) {
        ws.send(JSON.stringify({ type: "pause_deadloop", session_id: get().sessionId }));
      }
    }
  },

  resumePane: (paneId: number) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "resume_pane", session_id: sessionId, pane_id: paneId }));
      if (paneId === PANE_ID_DEADLOOP) {
        ws.send(JSON.stringify({ type: "resume_deadloop", session_id: get().sessionId }));
      }
    }
  },

  rebootPane: (paneId: number) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reboot_pane", session_id: sessionId, pane_id: paneId }));
    }
  },

  addPane: (
    provider: string,
    mode: string,
    label?: string,
    prompt?: string,
    model?: string,
    isolatedWorktree?: boolean,
    initialRole?: {
      role?: string;
      goal?: string;
      backstory?: string;
      planReviewMode?: PlanReviewMode;
    },
    /** v3.5 — true when this pane is being added through the Overview's
     *  + Add Worker flow (joins the team / Tech Lead can delegate to it);
     *  false (default) when it's a TabBar + side chat / experiment. */
    managed: boolean = false,
    /** "terminal" hosts the provider's real TUI on a pty instead of the
     *  headless stream-json worker. Such panes are never managed — they
     *  publish no stream events, so the Tech Lead can't delegate to them. */
    kind: PaneKind = "agent",
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
      session_id: get().sessionId,
      provider,
      mode,
      label: label || undefined,
      prompt: prompt || undefined,
      model: model || undefined,
      isolated_worktree: isolatedWorktree === true ? true : undefined,
      role: initialRole?.role || undefined,
      goal: initialRole?.goal || undefined,
      backstory: initialRole?.backstory || undefined,
      plan_review_mode: initialRole?.planReviewMode || undefined,
      managed: kind === "terminal" ? false : managed,
      kind,
    }));
    return { success: true };
  },

  removePane: (paneId: number, cleanupAction?: PaneCleanupAction) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      const payload: Record<string, unknown> = { type: "remove_pane", session_id: get().sessionId, pane_id: paneId };
      if (cleanupAction) {
        payload.cleanup_action = cleanupAction;
      }
      ws.send(JSON.stringify(payload));
    }
  },

  requestPaneDiff: (paneId: number) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "request_pane_diff", session_id: get().sessionId, pane_id: paneId }));
    }
  },

  createPanePr: (paneId: number) => {
    const { ws, showToast } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "create_pr", session_id: get().sessionId, pane_id: paneId }));
      showToast("Pushing branch + creating PR…", "info");
    } else {
      showToast("Not connected — cannot create PR", "error");
    }
  },

  updateProjectGoal: (goal: string) => {
    const { ws, showToast } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "update_project_goal", session_id: get().sessionId, goal }));
    } else {
      showToast("Not connected — cannot save goal", "error");
    }
  },

  updateProjectFlags: (flags) => {
    const { ws, sessionId, showToast } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      showToast("Not connected — cannot update flags", "error");
      return;
    }
    ws.send(
      JSON.stringify({
        type: "update_project_flags",
        session_id: get().sessionId,
        auto_approve_todos: flags.autoApproveTodos,
        auto_merge_prs: flags.autoMergePrs,
        team_enabled: flags.teamEnabled,
      }),
    );
    // Optimistic local update so the toggle feels instant; the CLI
    // echo (~5s poll cycle) will reconcile if anything diverges.
    if (sessionId) {
      set((state) => ({
        projectFlags: { ...state.projectFlags, [sessionId]: flags },
      }));
    }
  },

  startTeam: (specs) => {
    const { ws, showToast } = get();
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      showToast("Not connected — cannot start team", "error");
      return;
    }
    const toSpec = (s: { provider: string; model: string | null }) => ({
      provider: s.provider,
      ...(s.model != null ? { model: s.model } : {}),
    });
    ws.send(
      JSON.stringify({
        type: "start_team",
        session_id: get().sessionId,
        manager: toSpec(specs.manager),
        tech_lead: toSpec(specs.techLead),
        reviewer: toSpec(specs.reviewer),
        developer: toSpec(specs.developer),
      }),
    );
    showToast("Spawning team panes…", "info");
  },

  fetchTeamTodo: () => {
    const { ws, sessionId } = get();
    if (!sessionId) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "fetch_team_todo", session_id: sessionId }));
    }
  },

  approveTodo: (todoId: string) => {
    const { ws, sessionId, showToast } = get();
    if (!sessionId) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "todo_approval",
          session_id: sessionId,
          todo_id: todoId,
          action: "approve",
        }),
      );
    } else {
      showToast("Not connected — cannot approve", "error");
    }
  },

  rejectTodo: (todoId: string) => {
    const { ws, sessionId, showToast } = get();
    if (!sessionId) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "todo_approval",
          session_id: sessionId,
          todo_id: todoId,
          action: "reject",
        }),
      );
    } else {
      showToast("Not connected — cannot reject", "error");
    }
  },

  addTodo: (title: string, body: string) => {
    const { ws, sessionId, showToast } = get();
    if (!sessionId) return;
    const t = title.trim();
    if (!t) {
      showToast("TODO title can't be empty", "error");
      return;
    }
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "add_todo",
          session_id: sessionId,
          title: t,
          body,
        }),
      );
    } else {
      showToast("Not connected — cannot add TODO", "error");
    }
  },

  fetchSuggestedWorkers: () => {
    const { ws, sessionId } = get();
    if (!sessionId) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "fetch_suggested_workers",
          session_id: sessionId,
        }),
      );
    }
  },

  acceptSuggestion: (suggestion: SuggestedWorker) => {
    const { showToast, addPane, dismissSuggestion } = get();
    // addPane handles the actual pane creation; we mark managed=true so
    // it lands in the Team box. Worktree comes from needs_worktree.
    const result = addPane(
      "claude",
      "interactive",
      suggestion.label || suggestion.role || "New worker",
      undefined,
      undefined,
      suggestion.needs_worktree,
      {
        role: suggestion.role || undefined,
        goal: suggestion.goal || undefined,
        backstory: suggestion.backstory || undefined,
      },
      true,
    );
    if (!result.success) {
      showToast(result.error ?? "Failed to add worker", "error");
      return;
    }
    // Drop the section from suggested-workers.md so it doesn't show
    // again next render — the CLI republishes the trimmed list.
    dismissSuggestion(suggestion.id);
    showToast(
      `Accepted ${suggestion.label || suggestion.role || "suggestion"} — added to the team`,
      "info",
    );
  },

  dismissSuggestion: (suggestionId: string) => {
    const { ws, sessionId, showToast } = get();
    if (!sessionId) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "dismiss_suggestion",
          session_id: sessionId,
          suggestion_id: suggestionId,
        }),
      );
    } else {
      showToast("Not connected — cannot dismiss", "error");
    }
  },

  updatePaneRole: (paneId: number, role?: string, goal?: string, backstory?: string) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      const payload: Record<string, unknown> = { type: "update_pane_role", session_id: get().sessionId, pane_id: paneId };
      if (role !== undefined) payload.role = role;
      if (goal !== undefined) payload.goal = goal;
      if (backstory !== undefined) payload.backstory = backstory;
      ws.send(JSON.stringify(payload));
    }
  },

  answerPlanReview: (toolUseId: string, approve: boolean) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "plan_review_answer", session_id: get().sessionId, tool_use_id: toolUseId, approve }));
    }
    // Optimistically drop the pending item — CLI will send the control_response
    // regardless, and re-rendering it would confuse the user.
    set((state) => ({
      planReviewPending: state.planReviewPending.filter((p) => p.toolUseId !== toolUseId),
    }));
  },

  updatePaneReviewMode: (paneId: number, mode: PlanReviewMode) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "update_pane_review_mode", session_id: get().sessionId, pane_id: paneId, mode }));
    }
  },

  updatePaneManualMode: (paneId: number, manualMode: boolean) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: "update_pane_manual_mode",
        session_id: get().sessionId,
        pane_id: paneId,
        manual_mode: manualMode,
      }));
    }
  },

  /** Ask the server to replay this terminal pane's scrollback. Answered
   *  from the server's ring buffer, so it lands even if the CLI is
   *  mid-reconnect. */
  attachTerminal: (paneId: number) => {
    const { ws, sessionId } = get();
    if (!sessionId || !ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({
      type: "terminal_attach",
      session_id: sessionId,
      pane_id: paneId,
    }));
  },

  /** Forward keystrokes to the pane's pty. */
  sendTerminalInput: (paneId: number, data: string) => {
    const { ws, sessionId } = get();
    if (!sessionId || !ws || ws.readyState !== WebSocket.OPEN) return;
    // xterm hands us a JS string whose code units are the input bytes
    // (it does its own UTF-8 encoding), so treat it as binary here.
    const bytes = new Uint8Array(data.length);
    for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
    ws.send(JSON.stringify({
      type: "terminal_input",
      session_id: sessionId,
      pane_id: paneId,
      data_b64: encodeBase64(bytes),
    }));
  },

  /** Tell the pty the viewport size so the hosted TUI re-lays-out. */
  sendTerminalResize: (paneId: number, cols: number, rows: number) => {
    const { ws, sessionId } = get();
    if (!sessionId || !ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({
      type: "terminal_resize",
      session_id: sessionId,
      pane_id: paneId,
      cols,
      rows,
    }));
  },

  promotePaneToManaged: (paneId: number) => {
    const { ws, sessionId, showToast } = get();
    if (!sessionId) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: "promote_pane_to_managed",
        session_id: sessionId,
        pane_id: paneId,
      }));
    } else {
      showToast("Not connected — cannot promote", "error");
    }
  },

  updatePaneLabel: (paneId: number, label: string) => {
    const trimmed = label.trim();
    if (!trimmed) return;
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "update_pane_label", session_id: get().sessionId, pane_id: paneId, label: trimmed }));
    }
    // Optimistic local update — the label sticks in the UI immediately
    // regardless of whether the WS frame lands. mako Claude-6 reported
    // "reboot lost my rename" — either the WS frame was dropped in
    // transit (same class of bug as the AskUserQuestion answer loss)
    // or the CLI acked but a later PaneList refresh clobbered the
    // display. Optimistic + a persistent retry queue means neither
    // path can make the user's rename disappear.
    set((state) => ({
      paneConfigs: state.paneConfigs.map((p) =>
        p.pane_id === paneId ? { ...p, label: trimmed } : p,
      ),
    }));
    // Enqueue for retry-on-reconnect. Mirror pendingSends / pendingAnswers.
    const sessionId = get().sessionId;
    if (sessionId) {
      set((state) => {
        const filtered = state.pendingLabels.filter(
          (p) => !(p.paneId === paneId && p.sessionId === sessionId),
        );
        const next = [
          ...filtered,
          { paneId, label: trimmed, sessionId, createdAt: Date.now(), attempts: 1 },
        ];
        savePendingLabels(next);
        return { pendingLabels: next };
      });
    }
  },

  updatePaneEffort: (paneId: number, effort: string | null) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: "update_pane_effort",
        session_id: sessionId,
        pane_id: paneId,
        effort: effort ?? null,
      }));
    }
  },

  updatePaneModel: (
    paneId: number,
    model: string | null,
    provider?: string | null,
  ) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "update_pane_model",
          // Carry session_id so the switch targets the pane the user is
          // viewing, not the connection's drifting last-attached session —
          // which misrouted model changes on mobile. See interruptPane.
          session_id: sessionId,
          pane_id: paneId,
          model: model ?? null,
          // null = keep current provider; only send when explicitly
          // changing (avoids surprise resets on a model-only swap).
          ...(provider !== undefined ? { provider } : {}),
        }),
      );
    }
  },

  interruptPane: (paneId: number) => {
    const { ws, sessionId } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      // Carry session_id so the server can validate/auto-attach this
      // connection to the target session — without it, an interrupt sent
      // right after a reconnect routes by the connection's loosely-tracked
      // "current" session (or is dropped). Matters for "Stop team".
      ws.send(JSON.stringify({ type: "interrupt_pane", session_id: sessionId, pane_id: paneId }));
    }
  },

  reorderPanes: (paneIds: number[]) => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reorder_panes", session_id: get().sessionId, pane_ids: paneIds }));
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
        session_id: get().sessionId,
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
      ws.send(JSON.stringify({ type: "stop_bot", session_id: get().sessionId, pane_id: paneId }));
    }
  },

  rebootCli: () => {
    const { ws } = get();
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "reboot_cli", session_id: get().sessionId }));
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
    let newKeys: string[] = [];
    useStore.setState((state) => {
      const merged = new Map(diskCache);
      for (const [k, v] of state.sessionCache) {
        merged.set(k, v); // in-memory wins
      }
      newKeys = Array.from(diskCache.keys()).filter((k) => !state.sessionCache.has(k));
      // Seed the catchup watermark from each persisted entry. Without
      // this, a hydrated tab has no `sessionLastCreatedAt[sid]`, so the
      // first attach-after-load skips its catchup query and the user
      // sees stale messages until they hit refresh. In-memory wins on
      // conflict (it can only be newer).
      const seededLast = new Map(state.sessionLastCreatedAt);
      for (const [k, v] of diskCache) {
        if (!v.lastCreatedAt) continue;
        const existing = seededLast.get(k);
        if (!existing || v.lastCreatedAt > existing) {
          seededLast.set(k, v.lastCreatedAt);
        }
      }
      // Reseed per-pane watermarks so the first post-reload catchup uses
      // the precise per-pane form (get_messages_per_pane_after) rather
      // than the session-level MIN cutoff — the MIN is dragged into the
      // past by any long-idle pane, and the server's single-cutoff catchup
      // is capped at 500 rows across all panes, so an active pane's newest
      // messages can fall outside it and never repaint. In-memory wins on
      // conflict (it's strictly fresher for panes seen this page load).
      const seededPane = new Map(state.paneLastCreatedAt);
      for (const [k, v] of diskCache) {
        if (!v.paneLastCreatedAt) continue;
        const mergedPane = new Map(seededPane.get(k) ?? new Map<number, string>());
        for (const [pidStr, ts] of Object.entries(v.paneLastCreatedAt)) {
          const pid = Number(pidStr);
          if (!Number.isFinite(pid)) continue;
          const prev = mergedPane.get(pid);
          if (!prev || ts > prev) mergedPane.set(pid, ts);
        }
        if (mergedPane.size > 0) seededPane.set(k, mergedPane);
      }
      // Seed per-session file snapshots (team-todo, suggested-workers)
      // so the Overview panels render immediately on refresh instead of
      // going through a fetch round-trip (which silently does nothing
      // when the CLI is briefly offline). In-memory wins on conflict.
      const seededTodos = new Map(state.teamTodoStates);
      const seededSuggested = new Map(state.suggestedWorkersBySession);
      for (const [k, v] of diskCache) {
        if (v.teamTodoState && !seededTodos.has(k)) {
          seededTodos.set(k, v.teamTodoState);
        }
        if (v.suggestedWorkers && !seededSuggested.has(k)) {
          seededSuggested.set(k, v.suggestedWorkers);
        }
      }
      return {
        sessionCache: merged,
        sessionLastCreatedAt: seededLast,
        paneLastCreatedAt: seededPane,
        teamTodoStates: seededTodos,
        suggestedWorkersBySession: seededSuggested,
      };
    });
    // Race fix: if `attachSession` already fired for the current
    // session before IDB hydration completed, it read an empty
    // sessionCache and set paneMessages / messages to []. Now that
    // the snapshot is loaded, restore it so the user sees their
    // data without having to switch tabs or refresh. Skip if the
    // user already typed / received something into this session
    // (don't clobber live state).
    {
      const state = useStore.getState();
      if (state.sessionId) {
        const cached = diskCache.get(state.sessionId);
        const isEmpty =
          state.messages.length === 0 &&
          Object.keys(state.paneMessages).length === 0;
        if (cached && isEmpty) {
          // The live PaneList may already have arrived even though
          // messages haven't (attach sends PaneList alongside the
          // SessionMessages read) — never overwrite fresh pane state
          // with the snapshot's, which can carry modes from a CLI
          // boot ago (stale "deadloop" rendering bot UI on a pane
          // the CLI restored as interactive).
          useStore.setState({
            messages: cached.messages,
            paneMessages: cached.paneMessages,
            paneHasMore: cached.paneHasMore,
            paneConfigs:
              state.paneConfigs.length > 0
                ? state.paneConfigs
                : cached.paneConfigs,
            paneModes:
              Object.keys(state.paneModes).length > 0
                ? state.paneModes
                : cached.paneModes,
            hasMoreMessages: cached.hasMoreMessages,
            isDualPane: cached.isDualPane,
            answeredQuestions: cached.answeredQuestions,
            deadloopMessages:
              cached.paneMessages[paneKey(PANE_ID_DEADLOOP)] ?? [],
            interactiveMessages:
              cached.paneMessages[paneKey(PANE_ID_INTERACTIVE)] ?? [],
          });
        }
        // Catchup gap fix: fire the current session's catchup once the
        // per-pane watermarks are (re)seeded above — whether or not the
        // isEmpty restore ran. Two reload orderings otherwise leave an
        // actively-updating pane frozen at its cached tail: (1) attach saw
        // an empty cache and skipped its own catchup, then a live
        // stream_message beat hydration so `isEmpty` is now false and the
        // restore+catchup above is skipped; (2) attach restored a stale
        // cache whose tail predates messages that landed while the tab was
        // closed. With per-pane watermarks now persisted the catchup is
        // precise (get_messages_per_pane_after) and dedupes by id, so
        // firing it here is safe even when nothing was missed.
        requestCatchupIfNeeded(useStore.getState, state.sessionId);
      }
    }
    // If the WS is already authenticated, subscribe to the freshly-
    // hydrated sessions so the server starts pushing for them too.
    // (If hydration beat auth, the "authenticated" handler will do
    // this pass itself.)
    const state = useStore.getState();
    const ws = state.ws;
    if (state.isAuthenticated && ws && ws.readyState === WebSocket.OPEN) {
      const currentSid = state.sessionId;
      for (const sid of newKeys) {
        if (sid === currentSid) continue;
        ws.send(JSON.stringify({ type: "attach_session", session_id: sid }));
      }
    }
  });

  // Auto-snapshot the *active* session to IDB whenever its data
  // changes. attachSession only snapshots on switch-away, so without
  // this the currently-viewed session is never persisted — and a
  // hard refresh comes back to an empty pane until the server's
  // session_messages reply lands. Debounced 1s so a burst of
  // stream_messages doesn't hammer IDB.
  let snapshotTimer: ReturnType<typeof setTimeout> | null = null;
  useStore.subscribe((state, prev) => {
    if (!state.sessionId) return;
    if (
      state.sessionId === prev.sessionId &&
      state.paneMessages === prev.paneMessages &&
      state.messages === prev.messages &&
      state.paneConfigs === prev.paneConfigs &&
      state.paneHasMore === prev.paneHasMore &&
      state.paneModes === prev.paneModes &&
      state.teamTodoStates === prev.teamTodoStates &&
      state.suggestedWorkersBySession === prev.suggestedWorkersBySession
    ) {
      return;
    }
    const hasAnyData =
      state.messages.length > 0 ||
      Object.keys(state.paneMessages).length > 0 ||
      state.teamTodoStates.has(state.sessionId) ||
      state.suggestedWorkersBySession.has(state.sessionId);
    if (!hasAnyData) return;
    if (snapshotTimer) clearTimeout(snapshotTimer);
    snapshotTimer = setTimeout(() => {
      snapshotTimer = null;
      const cur = useStore.getState();
      const sid = cur.sessionId;
      if (!sid) return;
      const entry: SessionCacheEntry = {
        messages: cur.messages,
        paneMessages: cur.paneMessages,
        paneHasMore: cur.paneHasMore,
        paneConfigs: cur.paneConfigs,
        paneModes: cur.paneModes,
        hasMoreMessages: cur.hasMoreMessages,
        isDualPane: cur.isDualPane,
        answeredQuestions: cur.answeredQuestions,
        cachedAt: Date.now(),
        lastCreatedAt: cur.sessionLastCreatedAt.get(sid),
        paneLastCreatedAt: paneWatermarksToRecord(cur.paneLastCreatedAt.get(sid)),
        teamTodoState: cur.teamTodoStates.get(sid),
        suggestedWorkers: cur.suggestedWorkersBySession.get(sid),
      };
      saveSnapshotIdb(sid, entry);
    }, 1_000);
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
  set((state) => {
    const paneModes = mergePaneModeHints(state, { [key]: modeHint });
    return paneModes === state.paneModes ? {} : { paneModes };
  });
}

function authoritativePaneMode(
  state: Pick<AppState, "paneConfigs">,
  key: string,
): PaneType | undefined {
  const paneId = Number.parseInt(key, 10);
  if (!Number.isFinite(paneId)) return undefined;
  return state.paneConfigs.find((pane) => pane.pane_id === paneId)?.mode;
}

function mergePaneModeHints(
  state: Pick<AppState, "paneConfigs" | "paneModes">,
  paneModeHints: Record<string, PaneType>,
): Record<string, PaneType> {
  let next = state.paneModes;
  for (const [key, hintedMode] of Object.entries(paneModeHints)) {
    const mode = authoritativePaneMode(state, key) ?? hintedMode;
    if (next[key] === mode) continue;
    if (next === state.paneModes) next = { ...state.paneModes };
    next[key] = mode;
  }
  return next;
}

/// Append `message` to the in-memory `sessionCache` entry for a session
/// that isn't currently being viewed. Used to keep background tabs live
/// while the user is on a different project, so the moment they switch
/// they see exactly what claude did while they were away. No-op if the
/// session isn't already cached (we don't synthesize a snapshot for a
/// session we've never seen).
///
/// `serverCreatedAt` lets us advance the cache entry's `lastCreatedAt`
/// watermark alongside the message — important so a tab kept open while
/// background updates roll in still has an accurate watermark when the
/// page eventually reloads (otherwise the IDB snapshot reflects a much
/// older state than the visible messages do).
function appendToCachedSession(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  msgSessionId: string,
  message: Message,
  rawPaneType: string | undefined,
  rawPaneId: number | string | undefined,
  serverCreatedAt?: string,
) {
  const paneId = normalizePaneId(rawPaneType, normalizeRawPaneId(rawPaneId));
  set((state) => {
    const existing = state.sessionCache.get(msgSessionId);
    if (!existing) return {};
    const nextLastCreatedAt =
      serverCreatedAt &&
      (!existing.lastCreatedAt || serverCreatedAt > existing.lastCreatedAt)
        ? serverCreatedAt
        : existing.lastCreatedAt;
    // Advance this pane's persisted watermark in lockstep with the
    // appended message so a later reload catchup for this (background)
    // session asks only for messages AFTER it. Without this, the server
    // would re-send it, and because live messages carry client-generated
    // ids that don't match the storage id, the by-id dedupe couldn't drop
    // the duplicate.
    let nextPaneLastCreatedAt = existing.paneLastCreatedAt;
    if (paneId && serverCreatedAt) {
      const pk = String(paneId);
      const prev = existing.paneLastCreatedAt?.[pk];
      if (!prev || serverCreatedAt > prev) {
        nextPaneLastCreatedAt = {
          ...(existing.paneLastCreatedAt ?? {}),
          [pk]: serverCreatedAt,
        };
      }
    }
    let next: SessionCacheEntry;
    if (paneId) {
      const key = paneKey(paneId);
      const current = existing.paneMessages[key] || [];
      next = {
        ...existing,
        paneMessages: { ...existing.paneMessages, [key]: [...current, message] },
        isDualPane: true,
        cachedAt: Date.now(),
        lastCreatedAt: nextLastCreatedAt,
        paneLastCreatedAt: nextPaneLastCreatedAt,
      };
    } else {
      next = {
        ...existing,
        messages: [...existing.messages, message],
        cachedAt: Date.now(),
        lastCreatedAt: nextLastCreatedAt,
      };
    }
    const cache = new Map(state.sessionCache);
    cache.set(msgSessionId, next);
    return { sessionCache: cache };
  });
}

/// Dispatch a message to either the current-session state (real-time
/// view) or the cached snapshot for another session (background tab),
/// and mark the session unread when it's a background tab so the
/// sidebar can show an activity dot.
function routeMessage(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
  message: Message,
  msgSessionId: string | undefined,
  rawPaneType: string | undefined,
  rawPaneId: number | string | undefined,
  serverCreatedAt?: string,
) {
  const { sessionId: currentSessionId } = get();
  if (!msgSessionId || msgSessionId === currentSessionId) {
    addMessageWithPaneRouting(set, get, message, rawPaneType, rawPaneId);
    return;
  }
  appendToCachedSession(set, msgSessionId, message, rawPaneType, rawPaneId, serverCreatedAt);
  set((state) => {
    if (state.unreadSessions.has(msgSessionId)) return {};
    const next = new Set(state.unreadSessions);
    next.add(msgSessionId);
    return { unreadSessions: next };
  });
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

/// After WS reconnect, ask the server for every stored message with
/// `created_at > <watermark>`. Sends nothing if we have no watermark yet —
/// first attach of the tab has nothing to catch up on, the regular
/// session_messages reply covers it.
///
/// Watermark preference: `reconnectWatermarks[sid]` (frozen at reconnect
/// time) > `sessionLastCreatedAt[sid]` (live). The frozen one matters when
/// a stream_message arrived after reconnect but before the user clicked
/// this tab — without it, the live watermark would skip past the
/// disconnect-window messages still sitting on disk.
const PENDING_SENDS_KEY = "apas_pending_sends";
const PENDING_ANSWERS_KEY = "apas_pending_answers";
const ANSWERED_QUESTIONS_KEY = "apas_answered_questions";
const PENDING_LABELS_KEY = "apas_pending_labels";

/// One unacked pane-label rename waiting for confirmation that the CLI
/// received it (the next PaneList shows the new label). Persisted so a
/// dropped WS frame doesn't silently revert the user's rename.
export interface PendingLabel {
  paneId: number;
  label: string;
  sessionId: string;
  createdAt: number;
  attempts: number;
}

function loadPendingLabels(): PendingLabel[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(PENDING_LABELS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as PendingLabel[]) : [];
  } catch {
    return [];
  }
}

function savePendingLabels(items: PendingLabel[]) {
  if (typeof localStorage === "undefined") return;
  try {
    if (items.length === 0) {
      localStorage.removeItem(PENDING_LABELS_KEY);
    } else {
      localStorage.setItem(PENDING_LABELS_KEY, JSON.stringify(items));
    }
  } catch {
    // best-effort
  }
}

/// One unacked AskUserQuestion answer waiting for confirmation that
/// claude actually processed it (the answered tool_result arrives).
/// Persisted so a page refresh doesn't drop the submission on the
/// floor — mako Claude-6 reproduced this exact symptom (user answered,
/// answer never reached claude, refresh → card asks again).
export interface PendingAnswer {
  toolUseId: string;
  answers: Record<string, string>;
  sessionId: string;
  createdAt: number;
  attempts: number;
}

function loadPendingAnswers(): PendingAnswer[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(PENDING_ANSWERS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as PendingAnswer[]) : [];
  } catch {
    return [];
  }
}

function savePendingAnswers(items: PendingAnswer[]) {
  if (typeof localStorage === "undefined") return;
  try {
    if (items.length === 0) {
      localStorage.removeItem(PENDING_ANSWERS_KEY);
    } else {
      localStorage.setItem(PENDING_ANSWERS_KEY, JSON.stringify(items));
    }
  } catch {
    // best-effort
  }
}

/// Persisted mirror of `answeredQuestions`. Restores the
/// AskUserQuestionCard's "submitted" state after a refresh so the
/// user isn't asked to re-answer a question they've already answered.
/// Keyed by tool_use_id → answers.
function loadAnsweredQuestions(): Map<string, Record<string, string>> {
  if (typeof localStorage === "undefined") return new Map();
  try {
    const raw = localStorage.getItem(ANSWERED_QUESTIONS_KEY);
    if (!raw) return new Map();
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return new Map();
    return new Map(Object.entries(parsed as Record<string, Record<string, string>>));
  } catch {
    return new Map();
  }
}

function saveAnsweredQuestions(m: Map<string, Record<string, string>>) {
  if (typeof localStorage === "undefined") return;
  try {
    if (m.size === 0) {
      localStorage.removeItem(ANSWERED_QUESTIONS_KEY);
    } else {
      const obj: Record<string, Record<string, string>> = {};
      for (const [k, v] of m) obj[k] = v;
      localStorage.setItem(ANSWERED_QUESTIONS_KEY, JSON.stringify(obj));
    }
  } catch {
    // best-effort
  }
}

function loadPendingSends(): PendingSend[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(PENDING_SENDS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as PendingSend[]) : [];
  } catch {
    return [];
  }
}

function savePendingSends(items: PendingSend[]) {
  if (typeof localStorage === "undefined") return;
  try {
    if (items.length === 0) {
      localStorage.removeItem(PENDING_SENDS_KEY);
    } else {
      localStorage.setItem(PENDING_SENDS_KEY, JSON.stringify(items));
    }
  } catch {
    // quota / disabled storage — best-effort, in-memory state still works.
  }
}

/// Retransmit any unacked pending sends for the current session. Called
/// after authenticate (and reconnect) so the server gets a second
/// chance at sends that landed in the void during a silently-stale WS.
/// Duplicate-arrival risk: if the server got the original send, the
/// retry's `user_input` echo will arrive and both copies will dedup
/// against the optimistic placeholder via content+recency match.
function flushPendingSends(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
) {
  const state = get();
  const ws = state.ws;
  const currentSid = state.sessionId;
  if (!ws || ws.readyState !== WebSocket.OPEN || !currentSid) return;
  const toFlush = state.pendingSends.filter((p) => p.sessionId === currentSid);
  if (toFlush.length === 0) return;
  const now = Date.now();
  // Drop entries older than 10 minutes; bump attempt counter on the rest.
  const next: PendingSend[] = state.pendingSends
    .filter((p) => p.sessionId !== currentSid || now - p.createdAt <= 10 * 60_000)
    .map((p) =>
      p.sessionId === currentSid ? { ...p, attempts: p.attempts + 1 } : p,
    );
  for (const entry of toFlush) {
    if (now - entry.createdAt > 10 * 60_000) continue;
    ws.send(
      JSON.stringify({
        type: "input",
        session_id: entry.sessionId,
        text: entry.text,
        pane_type: entry.paneType,
        pane_id: entry.paneId,
        client_msg_id: entry.id,
      }),
    );
  }
  savePendingSends(next);
  set({ pendingSends: next });
}

/// Retransmit any unacked AskUserQuestion answers for the current
/// session. Mirrors flushPendingSends. Called after authenticate so a
/// dropped answer eventually lands. Duplicate-arrival risk: if the
/// CLI got the original answer, it removed the pending_questions entry
/// and the retry logs a warn ("no matching pending AskUserQuestion")
/// and is dropped harmlessly.
function flushPendingAnswers(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
) {
  const state = get();
  const ws = state.ws;
  const currentSid = state.sessionId;
  if (!ws || ws.readyState !== WebSocket.OPEN || !currentSid) return;
  const toFlush = state.pendingAnswers.filter((p) => p.sessionId === currentSid);
  if (toFlush.length === 0) return;
  // No TTL. An AskUserQuestion answer is unreconciled history: retransmit
  // it on every reconnect for as long as it's still pending, no matter how
  // old. The entry is cleared only when the server's message history shows
  // the question resolved — its tool_result arrives (answered OR cancelled;
  // see the tool_result handler that trims pendingAnswers). Dropping it on
  // a timer stranded the pane forever when the answer was lost or
  // misrouted. Idempotent: the CLI matches by tool_use_id, so a duplicate
  // no-ops if the answer already landed.
  const next: PendingAnswer[] = state.pendingAnswers.map((p) =>
    p.sessionId === currentSid ? { ...p, attempts: p.attempts + 1 } : p,
  );
  for (const entry of toFlush) {
    ws.send(
      JSON.stringify({
        type: "answer_question",
        // Carry the question's own session so a reconnect retry lands on
        // the right pane even if the connection's active session drifted.
        session_id: entry.sessionId,
        tool_use_id: entry.toolUseId,
        answers: entry.answers,
      }),
    );
  }
  savePendingAnswers(next);
  set({ pendingAnswers: next });
}

/// Retransmit any unacked pane-label renames for the current session.
/// Confirmed cleared when the incoming PaneList carries the matching
/// label for the pane_id.
function flushPendingLabels(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  get: () => AppState,
) {
  const state = get();
  const ws = state.ws;
  const currentSid = state.sessionId;
  if (!ws || ws.readyState !== WebSocket.OPEN || !currentSid) return;
  const toFlush = state.pendingLabels.filter((p) => p.sessionId === currentSid);
  if (toFlush.length === 0) return;
  const now = Date.now();
  const next: PendingLabel[] = state.pendingLabels
    .filter((p) => p.sessionId !== currentSid || now - p.createdAt <= 60 * 60_000)
    .map((p) =>
      p.sessionId === currentSid ? { ...p, attempts: p.attempts + 1 } : p,
    );
  for (const entry of toFlush) {
    if (now - entry.createdAt > 60 * 60_000) continue;
    ws.send(
      JSON.stringify({
        type: "update_pane_label",
        session_id: currentSid,
        pane_id: entry.paneId,
        label: entry.label,
      }),
    );
  }
  savePendingLabels(next);
  set({ pendingLabels: next });
}

/// Convert a session's in-memory per-pane watermark map (paneId → server
/// `created_at`) into a plain Record for structured-clone persistence in
/// the session snapshot. Skips the synthetic `-1` legacy single-pane key.
/// Returns undefined when there's nothing worth persisting so the snapshot
/// field stays absent rather than `{}`.
export function paneWatermarksToRecord(
  m: Map<number, string> | undefined,
): Record<string, string> | undefined {
  if (!m || m.size === 0) return undefined;
  const rec: Record<string, string> = {};
  for (const [pid, ts] of m) {
    if (pid < 0) continue;
    rec[String(pid)] = ts;
  }
  return Object.keys(rec).length > 0 ? rec : undefined;
}

/// Update the per-pane watermark for `(sessionId, paneId)` and
/// recompute the session-level watermark as the MIN across all known
/// panes. Caller passes `paneId = null` for messages that aren't
/// pane-scoped (legacy single-pane mode) — those advance the session
/// watermark directly under a synthetic "null" key. The MIN-semantics
/// guarantees a `after_created_at = sessionLastCreatedAt[sid]`
/// catchup query returns ALL missed messages for every pane, not
/// just the fast ones.
function bumpWatermark(
  set: (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void,
  sessionId: string,
  paneId: number | null,
  serverCreatedAt: string,
) {
  set((state) => {
    const sessionMap = state.paneLastCreatedAt.get(sessionId) ?? new Map<number, string>();
    const key = paneId ?? -1; // -1 = legacy single-pane bucket
    const prev = sessionMap.get(key);
    if (prev && prev >= serverCreatedAt) return {};
    const nextSession = new Map(sessionMap);
    nextSession.set(key, serverCreatedAt);
    const nextOuter = new Map(state.paneLastCreatedAt);
    nextOuter.set(sessionId, nextSession);
    // Recompute the session watermark as the MIN of per-pane maxes.
    // A pane we haven't seen any message from has no entry, so it
    // doesn't constrain the min. Once it sends its first message
    // we'll start tracking it; until then a catchup that uses the
    // current min is still safe for it (it asks for everything since
    // a time we know we've already seen for some pane).
    let minTs: string | undefined;
    for (const v of nextSession.values()) {
      if (!minTs || v < minTs) minTs = v;
    }
    const nextLast = new Map(state.sessionLastCreatedAt);
    if (minTs) nextLast.set(sessionId, minTs);
    return {
      paneLastCreatedAt: nextOuter,
      sessionLastCreatedAt: nextLast,
    };
  });
}

function requestCatchupIfNeeded(get: () => AppState, sessionId: string) {
  const state = get();
  const ws = state.ws;
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  // Per-pane watermarks are the preferred shape — they let the
  // server return only the messages each pane actually missed, no
  // overfetch. Build a {pane_id: ts} object from paneLastCreatedAt
  // (skipping the synthetic -1 sentinel for legacy single-pane).
  const paneMap = state.paneLastCreatedAt.get(sessionId);
  if (paneMap && paneMap.size > 0) {
    const wm: Record<string, string> = {};
    for (const [pid, ts] of paneMap) {
      if (pid < 0) continue;
      wm[String(pid)] = ts;
    }
    if (Object.keys(wm).length > 0) {
      ws.send(
        JSON.stringify({
          type: "get_session_messages",
          session_id: sessionId,
          pane_watermarks: wm,
        }),
      );
      return;
    }
  }
  // Fall back to the single-cutoff form when we have no per-pane
  // history yet (e.g. fresh tab, only the session-level mark survived
  // via an IDB snapshot). Server still handles `after_created_at`.
  const after =
    state.reconnectWatermarks.get(sessionId) ??
    state.sessionLastCreatedAt.get(sessionId);
  if (!after) return;
  ws.send(
    JSON.stringify({
      type: "get_session_messages",
      session_id: sessionId,
      after_created_at: after,
    }),
  );
}

/** Exported for tests: lets a spec drive a single server frame through
 *  the dispatch without standing up a WebSocket. */
export function handleServerMessage(
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
      storeDebugLog("Authenticated as user:", data.user_id);
      get().refreshCliClients();
      get().listMachines();
      get().listSessions();
      get().startAutoRefresh();
      // Freeze the current live watermarks before any post-reconnect
      // stream_message can advance them. Catchup queries that fire later
      // (either via the fan-out below or via a user click into a cached
      // tab) use these frozen values so disconnect-window messages on
      // disk aren't silently skipped over.
      set((state) => ({
        reconnectWatermarks: new Map(state.sessionLastCreatedAt),
      }));
      {
        // Prefer the in-memory sessionId (what this tab is currently viewing)
        // over localStorage — otherwise a reconnect could hijack this tab to a
        // session another browser tab wrote to localStorage.
        const currentSessionId = get().sessionId;
        const sessionToRestore =
          currentSessionId || localStorage.getItem("apas_session_id");
        if (sessionToRestore) {
          // Register the currently-viewed session's attachment IMMEDIATELY —
          // before the 500ms attachSession below, the staggered background
          // fan-out, and the IDB-hydration fan-out. Otherwise a control action
          // (pause/interrupt from "Stop team") fired right after a reconnect
          // can land before this session is attached and get dropped. The
          // server auto-attaches on access as a backstop, but ordering the
          // current session first avoids the round-trip and wrong-session
          // routing. An extra attach_session is idempotent server-side.
          const wsNow = get().ws;
          if (wsNow && wsNow.readyState === WebSocket.OPEN) {
            wsNow.send(JSON.stringify({ type: "attach_session", session_id: sessionToRestore }));
          }
          storeDebugLog("Restoring session:", sessionToRestore);
          setTimeout(() => {
            // forceReload=false: keep the cached paneMessages visible across
            // the reconnect (e.g. phone unlock killed the WS). Without this,
            // mobile users see the tab render normally for ~0.5s, then flash
            // blank for ~0.5s, then repopulate from the server snapshot —
            // exactly the "looks fine, then suddenly reloads" symptom.
            get().attachSession(sessionToRestore, false);
            // Reconnect catchup: fill the gap that landed while the WS was
            // down. AttachSession alone won't do it — the session_messages
            // initial-load reply is ignored when the local panes already
            // have messages ("live state wins" dedupe rule). The catchup
            // reply is flagged so the handler appends instead of skipping.
            requestCatchupIfNeeded(get, sessionToRestore);
            // Retransmit any sends the user typed during the silently-stale
            // WS window. Duplicate-arrival is fine: the user_input ack
            // dedupes against the optimistic placeholder via content+recency.
            flushPendingSends(set, get);
            // Same idea for unacked AskUserQuestion answers.
            flushPendingAnswers(set, get);
            // Same idea for unacked pane-label renames.
            flushPendingLabels(set, get);
          }, 500);
        }
        // Subscribe to every other session we have a cached snapshot for
        // so the server starts pushing stream_messages for them too. This
        // is what makes background tabs stay live without the user having
        // to click into them. AttachSession is multi-attach on the server
        // since the previous commit — earlier attachments aren't dropped.
        //
        // DO NOT fire requestCatchupIfNeeded here. Each catchup is a full
        // jsonl scan on the server; fanning out across 12 cached sessions
        // on every reconnect (and now on every hard refresh, since the
        // IDB-seeded watermarks made the early-return path inert) was the
        // OOM trigger. Click-time catchup in attachSession uses the
        // frozen `reconnectWatermarks` snapshot so it still fetches the
        // disconnect-window gap correctly — at the cost of background
        // tabs being momentarily stale until the user opens them.
        //
        // Stagger the attach sends anyway so 12 simultaneous attaches
        // don't pile up on the server's per-connection mpsc queue.
        const ws = get().ws;
        const cachedIds = Array.from(get().sessionCache.keys());
        cachedIds
          .filter((sid) => sid !== sessionToRestore)
          .forEach((sid, idx) => {
            setTimeout(() => {
              if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({ type: "attach_session", session_id: sid }));
              }
            }, 800 + idx * 150);
          });
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
            deepseekBackend: machine.deepseek_backend
              ? {
                  apiBaseUrl: ((machine.deepseek_backend as Record<string, unknown>).api_base_url as string | undefined),
                  apiKey: ((machine.deepseek_backend as Record<string, unknown>).api_key as string | undefined),
                  apiKeyConfigured: Boolean(
                    (machine.deepseek_backend as Record<string, unknown>).api_key_configured
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

          const deepseekBackend = entry.machine.deepseekBackend;
          const previousDeepseekKey = previous?.machine.deepseekBackend?.apiKey;
          const mergedDeepseek =
            deepseekBackend &&
            deepseekBackend.apiKey === undefined &&
            deepseekBackend.apiKeyConfigured &&
            previousDeepseekKey
              ? {
                  ...deepseekBackend,
                  apiKey: previousDeepseekKey,
                }
              : deepseekBackend;

          return {
            ...entry,
            machine: {
              ...entry.machine,
              minimaxBackend: mergedMiniMax,
              glmBackend: mergedGlm,
              deepseekBackend: mergedDeepseek,
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
        storeDebugLog("Session started:", newSessionId);
      } else {
        storeDebugLog(
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
      storeDebugLog("Session status:", data.status);
      break;

    case "session_attached": {
      const hasActiveCli = data.has_active_cli as boolean;
      storeDebugLog("Session attached, has active CLI:", hasActiveCli);
      set({ isAttached: hasActiveCli });
      break;
    }

    case "plan_review_request": {
      const paneId = data.pane_id as number | undefined;
      const toolUseId = data.tool_use_id as string | undefined;
      const toolName = data.tool_name as string | undefined;
      if (typeof paneId === "number" && typeof toolUseId === "string" && typeof toolName === "string") {
        set((state) => ({
          planReviewPending: [
            ...state.planReviewPending.filter((p) => p.toolUseId !== toolUseId),
            {
              paneId,
              toolUseId,
              toolName,
              input: data.input,
              arrivedAt: Date.now(),
            },
          ],
        }));
      }
      break;
    }

    case "team_record": {
      const record = data.record as TeamRecord | undefined;
      const sessionId = data.session_id as string | undefined;
      if (sessionId && record && typeof record.ts === "string") {
        set((state) => {
          const records = [...(state.teamRecordsBySession.get(sessionId) ?? []), record];
          const bySession = new Map(state.teamRecordsBySession);
          bySession.set(sessionId, records);
          return {
            teamRecordsBySession: bySession,
            teamRecords: state.sessionId === sessionId ? records : state.teamRecords,
          };
        });
      }
      break;
    }

    // Terminal frames bypass the store entirely — see terminalBus. Putting
    // them in zustand would re-render every subscriber on each pty chunk.
    case "terminal_output": {
      const paneId = data.pane_id as number | undefined;
      const b64 = data.data_b64 as string | undefined;
      if (typeof paneId === "number" && typeof b64 === "string") {
        emitTerminal(paneId, {
          kind: "output",
          bytes: decodeBase64(b64),
          seq: (data.seq as number) ?? 0,
        });
      }
      break;
    }

    case "terminal_snapshot": {
      const paneId = data.pane_id as number | undefined;
      const b64 = data.data_b64 as string | undefined;
      if (typeof paneId === "number" && typeof b64 === "string") {
        emitTerminal(paneId, {
          kind: "snapshot",
          bytes: decodeBase64(b64),
          seq: (data.seq as number) ?? 0,
          truncated: Boolean(data.truncated),
        });
      }
      break;
    }

    case "terminal_exited": {
      const paneId = data.pane_id as number | undefined;
      if (typeof paneId === "number") {
        emitTerminal(paneId, {
          kind: "exited",
          status: data.status as string | undefined,
        });
      }
      break;
    }

    case "pane_diff": {
      const paneId = data.pane_id as number | undefined;
      if (typeof paneId === "number") {
        set((state) => ({
          paneDiffs: {
            ...state.paneDiffs,
            [paneId]: {
              branch: data.branch as string | undefined,
              base: data.base as string | undefined,
              diff: data.diff as string | undefined,
              error: data.error as string | undefined,
              fetchedAt: Date.now(),
            },
          },
        }));
      }
      break;
    }

    case "project_goal_changed": {
      const sessionId = data.session_id as string | undefined;
      const content = data.content as string | undefined;
      if (sessionId && typeof content === "string") {
        set((state) => ({
          projectGoals: { ...state.projectGoals, [sessionId]: content },
        }));
      }
      break;
    }

    case "project_usage_stats": {
      const sessionId = data.session_id as string | undefined;
      const stats = data.stats as ProjectUsageStats | undefined;
      if (sessionId && stats) {
        set((state) => ({
          usageStats: { ...state.usageStats, [sessionId]: stats },
        }));
      }
      break;
    }

    case "project_flags_changed": {
      const sessionId = data.session_id as string | undefined;
      const autoApproveTodos = data.auto_approve_todos === true;
      const autoMergePrs = data.auto_merge_prs === true;
      // Absent means off: a CLI too old to send the field must not read as
      // team-enabled, or the UI would offer a team the CLI will refuse.
      const teamEnabled = data.team_enabled === true;
      if (sessionId) {
        set((state) => ({
          projectFlags: {
            ...state.projectFlags,
            [sessionId]: { autoApproveTodos, autoMergePrs, teamEnabled },
          },
        }));
      }
      break;
    }

    case "team_todo_state": {
      const responseSessionId = data.session_id as string | undefined;
      if (!responseSessionId) break;
      const todoState = data.state as TeamTodoState | undefined;
      if (todoState) {
        set((state) => {
          const next = new Map(state.teamTodoStates);
          next.set(responseSessionId, todoState);
          return { teamTodoStates: next };
        });
      }
      break;
    }

    case "suggested_workers_state": {
      const responseSessionId = data.session_id as string | undefined;
      if (!responseSessionId) break;
      const suggestions = data.suggestions as SuggestedWorker[] | undefined;
      set((state) => {
        const next = new Map(state.suggestedWorkersBySession);
        next.set(responseSessionId, suggestions ?? []);
        return { suggestedWorkersBySession: next };
      });
      break;
    }

    case "pr_created": {
      const url = data.url as string | undefined;
      const error = data.error as string | undefined;
      const { showToast } = get();
      if (url) {
        // Toast as info so it stays around a bit; user clicks the URL
        // from the toast to open the PR on GitHub.
        showToast(`PR created: ${url}`, "success");
        if (typeof window !== "undefined") {
          window.open(url, "_blank", "noopener");
        }
      } else {
        showToast(`PR create failed: ${error ?? "unknown error"}`, "error");
      }
      break;
    }

    case "project_instance_created": {
      const error = data.error as string | undefined;
      const { showToast } = get();
      if (error) {
        showToast(`New instance failed: ${error}`, "error");
      } else {
        showToast("New instance created and starting…", "success");
        // Refresh the machine list so the new running project appears.
        get().listMachines();
      }
      break;
    }

    case "pane_status": {
      // The web is multi-attached to several sessions (background tabs stay
      // live). Statuses from non-foreground sessions would otherwise
      // overwrite paneStatuses[paneId] for the tab the user is actually
      // viewing — that's how a "Pane worker unavailable" status from one
      // project's CLI ended up on another project's tab pill.
      const msgSessionId = data.session_id as string | undefined;
      const curSessionId = get().sessionId;
      if (msgSessionId && curSessionId && msgSessionId !== curSessionId) {
        break;
      }
      const paneType = data.pane_type as string | undefined;
      const paneId = normalizePaneId(paneType, data.pane_id as number | undefined);
      const status = data.status as string | null;
      const modeHint = normalizePaneModeHint(paneType);

      if (paneId) {
        set((state) => ({
          paneStatuses: { ...state.paneStatuses, [paneId]: status },
          paneModes: modeHint
            ? mergePaneModeHints(state, { [paneKey(paneId)]: modeHint })
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
      set((state) => {
        // Acknowledge pending label renames when the CLI's list now
        // reflects the requested label. Only clear on positive match
        // — if the CLI still has the old label, keep the entry and
        // let flushPendingLabels retry it on the next reconnect.
        const stillPending = state.pendingLabels.filter((p) => {
          const match = panes.find((pane) => pane.pane_id === p.paneId);
          return !match || match.label !== p.label;
        });
        const labelsChanged = stillPending.length !== state.pendingLabels.length;
        // If the CLI's list would clobber a still-pending optimistic
        // rename, keep the optimistic label on paneConfigs and let
        // flushPendingLabels drive it home. Otherwise trust the CLI.
        const pendingByPane = new Map(stillPending.map((p) => [p.paneId, p.label] as const));
        const merged = panes.map((pane) => {
          const kept = pendingByPane.get(pane.pane_id);
          return kept ? { ...pane, label: kept } : pane;
        });
        const patch: Partial<AppState> = {
          paneConfigs: merged,
          paneModes,
          pausedPanes: pausedPaneIds,
          isDeadloopPaused: pausedPaneIds.includes(PANE_ID_DEADLOOP),
        };
        if (labelsChanged) {
          savePendingLabels(stillPending);
          patch.pendingLabels = stillPending;
        }
        return patch;
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
        gitRemote: s.git_remote as string | undefined,
        gitRemoteUrl: s.git_remote_url as string | undefined,
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
      // Ack pending sends whose text already shows up in this batch.
      // Covers the post-refresh flow where the original input reached
      // the server but the optimistic placeholder is gone: without this
      // ack, the next reconnect would replay it and the user would see
      // a duplicate copy of their own input.
      const responseSidForAck = data.session_id as string | undefined;
      if (responseSidForAck) {
        const now = Date.now();
        set((state) => {
          const nextPending = state.pendingSends.filter((p) => {
            if (p.sessionId !== responseSidForAck) return true;
            if (now - p.createdAt > 10 * 60_000) return false;
            return !messages.some(
              (m) => m.role === "user" && m.content === p.text,
            );
          });
          if (nextPending.length === state.pendingSends.length) return {};
          savePendingSends(nextPending);
          return { pendingSends: nextPending };
        });
      }
      const hasMore = data.has_more as boolean || false;
      const isCatchup = data.catchup === true;

      // Drop stale responses — if the response is for a different session
      // than the one we're currently viewing, ignore it entirely (do NOT
      // overwrite sessionId or panes). Catchup is exempt: it's allowed to
      // land in the sessionCache for background tabs so they stay current
      // without the user clicking in.
      const responseSessionId = data.session_id as string | undefined;
      const currentSessionId = get().sessionId;
      if (
        !isCatchup &&
        responseSessionId &&
        currentSessionId &&
        responseSessionId !== currentSessionId
      ) {
        break;
      }

      const { isLoadingMore, loadingMorePane } = get();

      // Check if any messages have pane_type or pane_id - if so, enable dual pane
      const hasPaneType = messages.some((m) => m.pane_type || m.pane_id);
      if (hasPaneType) {
        set({ isDualPane: true });
      }

      // Pre-pass: populate toolNameMap from every tool_use in this
      // batch BEFORE mapping to Message objects. Without this, a
      // paginated / load-more fetch that delivers a tool_result whose
      // matching tool_use lives in an earlier (already-loaded or
      // not-yet-loaded) page falls back to
      // `tool: toolUseId` at line "tool: toolNameMap.get(toolUseId) ||
      // toolUseId" — which breaks the AssistantMessage router's
      // by-name filter for AskUserQuestion and leaks the raw
      // "User cancelled the question by sending a new prompt." body
      // into the chat as a red ToolCard.
      for (const m of messages) {
        if ((m.message_type as string) !== "tool_use") continue;
        try {
          const t = JSON.parse(m.content as string) as { id?: string; name?: string };
          if (t.id && t.name) toolNameMap.set(t.id, t.name);
        } catch {
          // Malformed tool_use content — skip; the per-message map
          // pass below will catch it (or fall back to text).
        }
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
            // A tool_result for an AskUserQuestion in the loaded history means
            // the question RESOLVED on the server — the answer landed, or it
            // was cancelled (e.g. the pane restarted). Either way it's no
            // longer open, so clear any pending retry: this is the
            // watermark-style reconciliation that lets the retransmit run
            // with no TTL (it stops as soon as history shows the question
            // closed). Recover the submitted answers for the card's state
            // only when the result actually carries them (a cancel doesn't).
            if (toolUseId && toolNameMap.get(toolUseId) === "AskUserQuestion") {
              const resultObj =
                resultData.tool_use_result &&
                typeof resultData.tool_use_result === "object"
                  ? (resultData.tool_use_result as Record<string, unknown>)
                  : undefined;
              const answers = resultObj?.answers as
                | Record<string, string>
                | undefined;
              set((state) => {
                const patch: Partial<AppState> = {};
                if (
                  answers &&
                  typeof answers === "object" &&
                  !state.answeredQuestions.has(toolUseId)
                ) {
                  const nextMap = new Map(state.answeredQuestions);
                  nextMap.set(toolUseId, answers);
                  saveAnsweredQuestions(nextMap);
                  patch.answeredQuestions = nextMap;
                }
                const trimmed = state.pendingAnswers.filter((p) => p.toolUseId !== toolUseId);
                if (trimmed.length !== state.pendingAnswers.length) {
                  savePendingAnswers(trimmed);
                  patch.pendingAnswers = trimmed;
                }
                return patch;
              });
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

      // Bootstrap per-pane watermarks from the loaded payload — find
      // each pane's max created_at and bump. sessionLastCreatedAt
      // re-derives as the MIN across panes inside bumpWatermark.
      if (responseSessionId) {
        const perPaneMax: Map<number | null, string> = new Map();
        for (const m of messages) {
          const ts = m.created_at as string | undefined;
          if (!ts) continue;
          const rawPaneType = m.pane_type as string | undefined;
          const rawPaneId = m.pane_id as number | undefined;
          const numericPane = normalizePaneId(rawPaneType, rawPaneId);
          const key: number | null = typeof numericPane === "number" ? numericPane : null;
          const prev = perPaneMax.get(key);
          if (!prev || ts > prev) perPaneMax.set(key, ts);
        }
        for (const [key, ts] of perPaneMax) {
          bumpWatermark(set, responseSessionId, key, ts);
        }
      }

      if (isCatchup) {
        // Reconnect tail: server filtered to `created_at > lastSeen`, so no
        // overlap with the live state is expected. Append (dedupe by id is
        // belt-and-suspenders; live IDs are client-random and won't match
        // storage IDs anyway, so the filter is what's actually keeping
        // duplicates out).
        const targetSid = responseSessionId;
        if (!targetSid) break;
        // Compute the new high-water mark for sessionLastCreatedAt.
        let maxCreatedAt: string | undefined;
        for (const m of messages) {
          const ts = m.created_at as string | undefined;
          if (ts && (!maxCreatedAt || ts > maxCreatedAt)) maxCreatedAt = ts;
        }

        if (targetSid === currentSessionId) {
          set((state) => {
            const newPaneMessages = { ...state.paneMessages };
            for (const [paneId, msgs] of Object.entries(paneMsgBuckets)) {
              const existing = newPaneMessages[paneId] || [];
              const existingIds = new Set(existing.map((m) => m.id));
              const tail = msgs.filter((m) => !existingIds.has(m.id));
              if (tail.length > 0) {
                newPaneMessages[paneId] = [...existing, ...tail];
              }
            }
            const existingMainIds = new Set(state.messages.map((m) => m.id));
            const mainTail = mainMsgs.filter((m) => !existingMainIds.has(m.id));
            const updates: Partial<AppState> = {
              paneMessages: newPaneMessages,
              deadloopMessages:
                newPaneMessages[paneKey(PANE_ID_DEADLOOP)] || state.deadloopMessages,
              interactiveMessages:
                newPaneMessages[paneKey(PANE_ID_INTERACTIVE)] || state.interactiveMessages,
            };
            if (mainTail.length > 0) {
              updates.messages = [...state.messages, ...mainTail];
            }
            // Per-pane watermark bumps happen below outside this set()
            // so the MIN re-derivation sees a consistent snapshot.
            if (state.reconnectWatermarks.has(targetSid)) {
              const nextRecon = new Map(state.reconnectWatermarks);
              nextRecon.delete(targetSid);
              updates.reconnectWatermarks = nextRecon;
            }
            return updates;
          });
          // Bump per-pane watermarks from the catchup payload (replaces
          // the old session-level maxCreatedAt bump — using the max
          // would advance the watermark past slower panes' tails).
          if (targetSid) {
            const perPaneMax: Map<number | null, string> = new Map();
            for (const m of messages) {
              const ts = m.created_at as string | undefined;
              if (!ts) continue;
              const numericPane = normalizePaneId(
                m.pane_type as string | undefined,
                m.pane_id as number | undefined,
              );
              const key: number | null = typeof numericPane === "number" ? numericPane : null;
              const prev = perPaneMax.get(key);
              if (!prev || ts > prev) perPaneMax.set(key, ts);
            }
            for (const [key, ts] of perPaneMax) {
              bumpWatermark(set, targetSid, key, ts);
            }
          }
        } else {
          // Cached background session — apply tail to the snapshot so when
          // the user opens that tab next they see what claude did while
          // they were elsewhere.
          set((state) => {
            const cached = state.sessionCache.get(targetSid);
            if (!cached) return {};
            const newPaneMessages = { ...cached.paneMessages };
            for (const [paneId, msgs] of Object.entries(paneMsgBuckets)) {
              const cur = newPaneMessages[paneId] || [];
              const curIds = new Set(cur.map((m) => m.id));
              const tail = msgs.filter((m) => !curIds.has(m.id));
              if (tail.length > 0) newPaneMessages[paneId] = [...cur, ...tail];
            }
            const cache = new Map(state.sessionCache);
            cache.set(targetSid, {
              ...cached,
              paneMessages: newPaneMessages,
              cachedAt: Date.now(),
            });
            const updates: Partial<AppState> = { sessionCache: cache };
            // Per-pane watermark bumps happen below.
            if (state.reconnectWatermarks.has(targetSid)) {
              const nextRecon = new Map(state.reconnectWatermarks);
              nextRecon.delete(targetSid);
              updates.reconnectWatermarks = nextRecon;
            }
            return updates;
          });
          if (targetSid) {
            const perPaneMax: Map<number | null, string> = new Map();
            for (const m of messages) {
              const ts = m.created_at as string | undefined;
              if (!ts) continue;
              const numericPane = normalizePaneId(
                m.pane_type as string | undefined,
                m.pane_id as number | undefined,
              );
              const key: number | null = typeof numericPane === "number" ? numericPane : null;
              const prev = perPaneMax.get(key);
              if (!prev || ts > prev) perPaneMax.set(key, ts);
            }
            for (const [key, ts] of perPaneMax) {
              bumpWatermark(set, targetSid, key, ts);
            }
          }
        }
        break;
      }

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
              updates.paneModes = mergePaneModeHints(state, paneModeHints);
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
            set((state) => {
              const paneModes = mergePaneModeHints(state, paneModeHints);
              return paneModes === state.paneModes ? {} : { paneModes };
            });
          }
          get().prependMessages(parsedMessages, hasMore);
        }
      } else if (isDualPane || hasPaneType) {
        // Initial / window-refresh load - dual pane mode. `paneMsgBuckets`
        // holds a contiguous newest-N slice per pane straight from the
        // server. We reconcile it as a SLIDING WINDOW so the rendered tail
        // is always a hole-free, server-authoritative block ending at
        // "now":
        //   - empty local bucket  -> accept the slice (first-time load).
        //   - non-empty bucket     -> keep cached messages strictly OLDER
        //     than the slice's oldest, then let the slice REPLACE the
        //     recent window. This overwrites any hole a flaky reconnect
        //     left BELOW the watermark — which the `after_created_at`
        //     catchup can only ever skip, since it extends the frontier
        //     forward and never re-examines older territory.
        //
        // Replacing the range wholesale (rather than merging by id) is what
        // keeps it safe: stream_message messages carry client-random ids
        // while session_messages carry storage ids, so an id-merge would
        // duplicate the overlap. The window swap sidesteps that entirely;
        // the only overlap is at the boundary, cut by the strict-older
        // filter. Older history still pages in contiguously via
        // loadMoreMessages (scroll-to-top, the isLoadingMore branch above).
        const { paneModes: existingPaneModes, paneMessages: existingPaneMessages, paneHasMore: existingPaneHasMore, messages: existingMessages } = get();
        const newPaneMessages: Record<string, Message[]> = { ...existingPaneMessages };
        const newPaneHasMore: Record<string, boolean> = { ...existingPaneHasMore };
        for (const [paneId, msgs] of Object.entries(paneMsgBuckets)) {
          const existing = existingPaneMessages[paneId] || [];
          if (existing.length === 0) {
            // First load for this pane — accept the server snapshot.
            newPaneMessages[paneId] = msgs;
            newPaneHasMore[paneId] = hasMore;
          } else if (msgs.length > 0) {
            // Sliding-window reconcile: overwrite the recent range with the
            // server's contiguous slice, keep older cached history.
            const windowOldest = msgs.reduce(
              (min, m) => (m.timestamp < min ? m.timestamp : min),
              msgs[0].timestamp,
            );
            const kept = existing.filter((m) => m.timestamp < windowOldest);
            newPaneMessages[paneId] = [...kept, ...msgs];
            // There's older history to page if we kept anything below the
            // slice, or the server flags more before it.
            newPaneHasMore[paneId] = kept.length > 0 ? true : hasMore;
          }
        }
        // Same rule for the legacy single-pane bucket.
        const effectiveMessages = existingMessages.length === 0 ? mainMsgs : existingMessages;
        set({
          sessionId: data.session_id as string,
          messages: effectiveMessages,
          paneMessages: newPaneMessages,
          paneHasMore: newPaneHasMore,
          deadloopMessages: newPaneMessages[paneKey(PANE_ID_DEADLOOP)] || [],
          interactiveMessages: newPaneMessages[paneKey(PANE_ID_INTERACTIVE)] || [],
          hasMoreMessages: existingMessages.length === 0 ? hasMore : get().hasMoreMessages,
          hasMoreDeadloop: newPaneHasMore[paneKey(PANE_ID_DEADLOOP)] || false,
          hasMoreInteractive: newPaneHasMore[paneKey(PANE_ID_INTERACTIVE)] || false,
          paneModes: hasPaneModeHints
            ? mergePaneModeHints(get(), paneModeHints)
            : existingPaneModes,
          isDualPane: true,
        });
        // Clear in-flight markers for panes we just got data for.
        // Panes the response was filtered to but returned empty don't
        // show up in paneMsgBuckets — those get cleared by the timeout
        // fallback set up in loadPaneMessagesIfNeeded.
        if (Object.keys(paneMsgBuckets).length > 0) {
          set((state) => {
            const next = new Set(state.paneLoadingInitial);
            for (const k of Object.keys(paneMsgBuckets)) {
              const pid = Number.parseInt(k, 10);
              if (Number.isFinite(pid)) next.delete(pid);
            }
            return { paneLoadingInitial: next };
          });
        }
      } else {
        // Initial load - single pane mode. Same "trust local" rule:
        // if we already have messages, ignore the server snapshot to
        // avoid the random-ID-vs-storage-ID duplicate-merge bug.
        set((state) => ({
          sessionId: data.session_id as string,
          messages: state.messages.length === 0 ? parsedMessages : state.messages,
          hasMoreMessages: state.messages.length === 0 ? hasMore : state.hasMoreMessages,
          paneModes: hasPaneModeHints
            ? mergePaneModeHints(state, paneModeHints)
            : state.paneModes,
        }));
      }
      break;
    }

    case "user_input": {
      const msgSessionId = data.session_id as string | undefined;
      const paneType = data.pane_type as string | undefined;
      const paneId = data.pane_id as number | string | undefined;
      const text = data.text as string;
      const isCurrentSession = !msgSessionId || msgSessionId === get().sessionId;

      // Track per-pane watermark for catchup (see stream_message).
      const serverCreatedAt = data.created_at as string | undefined;
      if (serverCreatedAt && msgSessionId) {
        const numericPane =
          typeof paneId === "number"
            ? paneId
            : typeof paneId === "string"
              ? Number.parseInt(paneId, 10)
              : null;
        const paneIdForBump = Number.isFinite(numericPane as number)
          ? (numericPane as number)
          : null;
        bumpWatermark(set, msgSessionId, paneIdForBump, serverCreatedAt);
      }

      // Server bounce of our own input: claim the optimistic slot the
      // send handler placed locally. Prefer exact client_msg_id match
      // (the id we sent rides back on the echo); fall back to content +
      // recency + the "optimistic-" id prefix for older servers. Strip
      // the prefix so a later duplicate user_input with the same text
      // doesn't re-claim this slot.
      const clientMsgId = data.client_msg_id as string | undefined;
      if (isCurrentSession) {
        const normalizedPaneId = normalizePaneId(paneType, normalizeRawPaneId(paneId));
        if (normalizedPaneId != null) {
          const key = paneKey(normalizedPaneId);
          const bucket = get().paneMessages[key] || [];
          // Duplicate echo of an already-claimed send (e.g. the server's
          // re-ack of a retransmit): the optimistic slot was claimed and
          // the prefix stripped, so the message id IS the client_msg_id.
          // Drop the echo instead of appending a second copy.
          if (clientMsgId && bucket.some((m) => m.id === clientMsgId)) {
            const nextPending = get().pendingSends.filter((p) => p.id !== clientMsgId);
            if (nextPending.length !== get().pendingSends.length) {
              savePendingSends(nextPending);
              set({ pendingSends: nextPending });
            }
            break;
          }
          const now = Date.now();
          const idx = bucket.findIndex((m) => {
            if (m.role !== "user" || !m.id.startsWith("optimistic-")) return false;
            if (clientMsgId) return m.id === `optimistic-${clientMsgId}`;
            return m.content === text && now - m.timestamp.getTime() < 30_000;
          });
          if (idx >= 0) {
            const ackedSendId = bucket[idx].id.replace(/^optimistic-/, "");
            set((state) => {
              const updated = [...(state.paneMessages[key] || [])];
              const orig = updated[idx];
              if (orig) {
                updated[idx] = {
                  ...orig,
                  id: orig.id.replace(/^optimistic-/, ""),
                };
              }
              const nextPending = state.pendingSends.filter(
                (p) => p.id !== ackedSendId,
              );
              if (nextPending.length !== state.pendingSends.length) {
                savePendingSends(nextPending);
              }
              const next: Partial<AppState> = {
                paneMessages: { ...state.paneMessages, [key]: updated },
                pendingSends: nextPending,
              };
              if (normalizedPaneId === PANE_ID_DEADLOOP) next.deadloopMessages = updated;
              if (normalizedPaneId === PANE_ID_INTERACTIVE) next.interactiveMessages = updated;
              return next;
            });
            updatePaneModeHint(set, get, paneType, paneId);
            break;
          }
        }
      }

      const userMessage: Message = {
        id: generateId(),
        role: "user",
        content: text,
        timestamp: new Date(),
        outputType: { type: "text" },
      };
      // updatePaneModeHint touches the current-session state; only run
      // it for the active session (other sessions track this in their
      // cache snapshot).
      if (isCurrentSession) {
        updatePaneModeHint(set, get, paneType, paneId);
      }
      routeMessage(set, get, userMessage, msgSessionId, paneType, paneId, serverCreatedAt);
      // Ack any pending-send entry that matches this echo — covers the
      // post-refresh case where the optimistic placeholder is gone (it
      // wasn't persisted) but the queued retry needs to be cleared so
      // the next reconnect doesn't replay it again.
      if (msgSessionId) {
        const now = Date.now();
        const nextPending = get().pendingSends.filter((p) => {
          if (clientMsgId && p.id === clientMsgId) return false;
          return !(
            p.sessionId === msgSessionId &&
            p.text === text &&
            now - p.createdAt < 10 * 60_000
          );
        });
        if (nextPending.length !== get().pendingSends.length) {
          savePendingSends(nextPending);
          set({ pendingSends: nextPending });
        }
      }
      break;
    }

    case "stream_message": {
      const msgSessionId = data.session_id as string | undefined;

      const msg = data.message as Record<string, unknown>;
      if (!msg) break;

      const paneType = data.pane_type as string | undefined;
      const paneId = data.pane_id as number | string | undefined;
      const isCurrentSession = !msgSessionId || msgSessionId === get().sessionId;
      if (isCurrentSession) {
        updatePaneModeHint(set, get, paneType, paneId);
      }
      // Track per-pane watermarks; sessionLastCreatedAt derives as the
      // MIN across panes so a catchup after a WS reconnect doesn't
      // skip over a slow pane's gap just because a fast pane has
      // advanced. (See bumpWatermark for the why.)
      const serverCreatedAt = data.created_at as string | undefined;
      if (serverCreatedAt && msgSessionId) {
        const numericPane =
          typeof paneId === "number"
            ? paneId
            : typeof paneId === "string"
              ? Number.parseInt(paneId, 10)
              : null;
        const paneIdForBump = Number.isFinite(numericPane as number)
          ? (numericPane as number)
          : null;
        bumpWatermark(set, msgSessionId, paneIdForBump, serverCreatedAt);
      }
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
              routeMessage(set, get, assistantMessage, msgSessionId, paneType, paneId, serverCreatedAt);
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
              routeMessage(set, get, toolMessage, msgSessionId, paneType, paneId, serverCreatedAt);
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
                  const capturedId = toolUseId;
                  set((state) => {
                    const patch: Partial<AppState> = {};
                    if (!state.answeredQuestions.has(capturedId)) {
                      const nextMap = new Map(state.answeredQuestions);
                      nextMap.set(capturedId, answers);
                      saveAnsweredQuestions(nextMap);
                      patch.answeredQuestions = nextMap;
                    }
                    // Confirm receipt — drop any pending retry entry.
                    const trimmed = state.pendingAnswers.filter(
                      (p) => p.toolUseId !== capturedId,
                    );
                    if (trimmed.length !== state.pendingAnswers.length) {
                      savePendingAnswers(trimmed);
                      patch.pendingAnswers = trimmed;
                    }
                    return patch;
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
              routeMessage(set, get, toolResultMessage, msgSessionId, paneType, paneId, serverCreatedAt);
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
          // Look in the right bucket — current session uses live state,
          // background sessions use the cached snapshot.
          let existing: Message[];
          if (isCurrentSession) {
            existing = normalizedPaneId
              ? (get().paneMessages[paneKey(normalizedPaneId)] || [])
              : get().messages;
          } else if (msgSessionId) {
            const cached = get().sessionCache.get(msgSessionId);
            existing = cached
              ? (normalizedPaneId
                  ? (cached.paneMessages[paneKey(normalizedPaneId)] || [])
                  : cached.messages)
              : [];
          } else {
            existing = [];
          }
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
            routeMessage(set, get, assistantMessage, msgSessionId, paneType, paneId);
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
        routeMessage(set, get, resultMessage, msgSessionId, paneType, paneId, serverCreatedAt);
      }
      break;
    }

    case "deadloop_status": {
      const isPaused = data.is_paused as boolean;
      storeDebugLog("Deadloop status update:", isPaused ? "paused" : "running");
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
      storeDebugLog(`Pane ${paneId} ${isPaused ? "paused" : "resumed"}`);
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

      storeDebugLog(`Downloaded session ${sessionId} with ${messages.length} messages`);
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
        storeDebugLog("Usage limits updated for CLI:", cliClientId, provider, parsedLimits);
      }
      break;
    }

    default:
      storeDebugLog("Unknown message type:", data.type);
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
