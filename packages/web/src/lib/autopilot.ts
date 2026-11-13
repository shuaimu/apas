import type {
  AutopilotAction,
  AutopilotEnded,
  AutopilotLimit,
  AutopilotRepeat,
  AutopilotSpec,
  AutopilotSupport,
  AutopilotView,
  OmpModesView,
  PaneAutopilotState,
  PaneAutopilotSummary,
} from "@apas/protocol";

/**
 * Autopilot for interactive OMP terminal tabs: the rules the desktop toolbar,
 * the tab bar, the project lists and the mobile-browser screen share. A tab on
 * autopilot repeats one prompt on its own, inside its OMP process; the page
 * only shows what the tab reports and relays requests. Everything here is
 * pure, so every surface gives the same state the same words and disables the
 * same actions for the same reasons.
 *
 * The wire contract is `crates/shared/src/autopilot.rs`. Its condition and
 * reason codes are plain strings because a tab's extension can be newer than
 * this page: a code this page does not know is shown as a plain "Paused",
 * "Ended" or "On autopilot", never dropped.
 *
 * Autopilot is not the autonomous worker and not the retired bot, and shares
 * none of their words or icons.
 */

/** Mirrors `shared::TAB_AUTOPILOT_CAPABILITY`. Advertised in
 *  `Authenticate.capabilities`; a server that does not echo it sends no
 *  autopilot frame and accepts no request, so nothing is shown at all. */
export const TAB_AUTOPILOT_CAPABILITY = "tab_autopilot_v1";

/** Mirrors `shared::AUTOPILOT_ITERATION_TOOL_NAME`: the reserved tool-use
 *  block that marks where an iteration starts in a tab's conversation. */
export const AUTOPILOT_ITERATION_TOOL_NAME = "ApasAutopilotIteration";

// The bounds of a start, mirrored from the shared crate. The server, the
// tab's machine and the tab check them again; checking here only saves a
// round trip for a mistyped number.
export const AUTOPILOT_PROMPT_MAX_BYTES = 16_000;
export const AUTOPILOT_INTERVAL_MIN_SECS = 60;
export const AUTOPILOT_INTERVAL_MAX_SECS = 24 * 60 * 60;
export const AUTOPILOT_TIME_LIMIT_MIN_SECS = 60;
export const AUTOPILOT_TIME_LIMIT_MAX_SECS = 365 * 24 * 60 * 60;
export const AUTOPILOT_ITERATION_LIMIT_MAX = 1_000_000;

/** What the page holds for one OMP terminal tab. Holding nothing reads as
 *  not reported, never as off. */
export interface PaneAutopilotEntry {
  support: AutopilotSupport;
  state?: PaneAutopilotState;
  /** When the tab's machine last saw the tab refresh its report (RFC 3339). */
  reportedAt?: string;
}

/** Frames arrive for every attached session, so entries are keyed by both. */
export function paneAutopilotKey(sessionId: string, paneId: number): string {
  return `${sessionId}/${paneId}`;
}

const SUPPORT_VALUES: ReadonlySet<string> = new Set<AutopilotSupport>([
  "supported",
  "not_reported",
  "needs_agent_reboot",
  "unsupported_omp",
  "unavailable_on_platform",
  "instance_update_required",
]);

// A spec is the one strict part of a report, as it is in the shared crate
// (`deny_unknown_fields`): this page can be an older bundle than the tab that
// reports, and a spec with a field it does not know may carry a limit it
// would otherwise show, and offer for a new start, without.
function hasOnlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  return Object.keys(value).every((key) => keys.includes(key));
}

function parseLimit(raw: unknown): AutopilotLimit | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  if (value.kind === "none" && hasOnlyKeys(value, ["kind"])) return { kind: "none" };
  if (value.kind === "at_most" && hasOnlyKeys(value, ["kind", "value"])
    && typeof value.value === "number" && Number.isFinite(value.value)) {
    return { kind: "at_most", value: value.value };
  }
  return null;
}

const SPEC_KEYS = ["prompt", "repeat", "iteration_limit", "time_limit", "end_without_tool_call"] as const;

function parseSpec(raw: unknown): AutopilotSpec | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  if (!hasOnlyKeys(value, SPEC_KEYS)) return null;
  const repeatRaw = value.repeat as Record<string, unknown> | null | undefined;
  let repeat: AutopilotRepeat | null = null;
  if (repeatRaw && typeof repeatRaw === "object") {
    if (repeatRaw.kind === "after_each_turn" && hasOnlyKeys(repeatRaw, ["kind"])) repeat = { kind: "after_each_turn" };
    else if (repeatRaw.kind === "every" && hasOnlyKeys(repeatRaw, ["kind", "seconds"])
      && typeof repeatRaw.seconds === "number" && Number.isFinite(repeatRaw.seconds)) {
      repeat = { kind: "every", seconds: repeatRaw.seconds };
    }
  }
  const iterationLimit = parseLimit(value.iteration_limit);
  const timeLimit = parseLimit(value.time_limit);
  if (typeof value.prompt !== "string" || !repeat || !iterationLimit || !timeLimit
    || typeof value.end_without_tool_call !== "boolean") {
    return null;
  }
  return {
    prompt: value.prompt,
    repeat,
    iteration_limit: iterationLimit,
    time_limit: timeLimit,
    end_without_tool_call: value.end_without_tool_call,
  };
}

function optionalText(raw: unknown): string | undefined {
  return typeof raw === "string" && raw ? raw : undefined;
}

/**
 * Read one `pane_autopilot_state` frame. `null` for a frame that names no
 * tab. A state whose autopilot this page cannot read (a newer spec, say) is
 * kept as not reported, where Stop still works, instead of showing limits
 * that may have been misread.
 */
export function parsePaneAutopilotFrame(
  data: Record<string, unknown>,
): { sessionId: string; paneId: number; entry: PaneAutopilotEntry } | null {
  const sessionId = data.session_id;
  const paneId = data.pane_id;
  if (typeof sessionId !== "string" || !sessionId || typeof paneId !== "number" || !Number.isInteger(paneId)) {
    return null;
  }
  const reportedAt = optionalText(data.reported_at);
  const notReported = { sessionId, paneId, entry: { support: "not_reported" as const, reportedAt } };
  if (typeof data.support !== "string" || !SUPPORT_VALUES.has(data.support)) return notReported;
  const support = data.support as AutopilotSupport;
  const rawState = data.state;
  // Not reported means nothing is known now. A state that came with it could
  // only be an old one, and showing it would offer Pause or Resume for an
  // autopilot nobody is refreshing.
  if (support === "not_reported" || !rawState || typeof rawState !== "object") {
    return { sessionId, paneId, entry: { support, reportedAt } };
  }
  const raw = rawState as Record<string, unknown>;
  const state: PaneAutopilotState = {};
  // A tab that cannot run autopilot has no current one, whatever came along.
  if (support === "supported" && raw.autopilot && typeof raw.autopilot === "object") {
    const view = raw.autopilot as Record<string, unknown>;
    const spec = parseSpec(view.spec);
    const condition = view.condition as Record<string, unknown> | null | undefined;
    if (!spec || typeof view.autopilot_id !== "string" || typeof view.started_at !== "string"
      || typeof view.iterations !== "number" || !condition || typeof condition.kind !== "string") {
      return notReported;
    }
    state.autopilot = {
      autopilot_id: view.autopilot_id,
      spec,
      started_at: view.started_at,
      iterations: view.iterations,
      deadline: optionalText(view.deadline),
      next_iteration_at: optionalText(view.next_iteration_at),
      condition: {
        kind: condition.kind,
        detail: optionalText(condition.detail),
        since: optionalText(condition.since),
      },
    };
  }
  if (raw.last_ended && typeof raw.last_ended === "object") {
    const ended = raw.last_ended as Record<string, unknown>;
    const spec = parseSpec(ended.spec);
    if (!spec || typeof ended.autopilot_id !== "string" || typeof ended.started_at !== "string"
      || typeof ended.iterations !== "number" || typeof ended.reason !== "string"
      || typeof ended.ended_at !== "string") {
      return notReported;
    }
    state.last_ended = {
      autopilot_id: ended.autopilot_id,
      spec,
      started_at: ended.started_at,
      iterations: ended.iterations,
      reason: ended.reason,
      ended_at: ended.ended_at,
      turn_since: ended.turn_since === true,
    };
  }
  if (raw.omp_modes && typeof raw.omp_modes === "object") {
    const modes = raw.omp_modes as Record<string, unknown>;
    const goal = modes.goal as Record<string, unknown> | null | undefined;
    const loop = modes.loop as Record<string, unknown> | null | undefined;
    state.omp_modes = {
      goal: goal && typeof goal === "object" && typeof goal.status === "string"
        ? {
            status: goal.status,
            objective: optionalText(goal.objective),
            token_budget: typeof goal.token_budget === "number" ? goal.token_budget : undefined,
            tokens_used: typeof goal.tokens_used === "number" ? goal.tokens_used : undefined,
            goal_status: optionalText(goal.goal_status),
          }
        : undefined,
      loop: loop && typeof loop === "object" && typeof loop.status === "string"
        ? { status: loop.status }
        : undefined,
    };
  }
  if (raw.reported_by_cli === true) state.reported_by_cli = true;
  return { sessionId, paneId, entry: { support, state, reportedAt } };
}

// ---------------------------------------------------------------------------
// Conditions and their words
// ---------------------------------------------------------------------------

/** Formats an RFC 3339 time for display; `null` when it cannot be read. */
export type AutopilotTimeFormatter = (iso: string) => string | null;

/** A clock time for today, a date and a time for any other day. */
export function formatAutopilotTime(iso: string, now: Date = new Date()): string | null {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (date.toDateString() === now.toDateString()) return time;
  return `${date.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

/** A full date and time, for the details view. */
export function formatAutopilotDateTime(iso: string): string | null {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

// `autopilot_condition::PAUSE_*` → the spec's texts, word for word.
const PAUSE_TEXTS: Record<string, string> = {
  requested: "Paused",
  failures: "Paused after three failed iterations",
  interrupted: "Paused: the turn was interrupted",
  background_wait_ended: "Paused: the wait for background work was ended in the terminal",
  no_control: "Paused: APAS could not control this tab for more than four hours",
  agent_restarted: "Paused: the agent was restarted",
  agent_exited: "Paused: the agent process exited",
  omp_goal_mode: "Paused: OMP's own goal mode was switched on",
  omp_loop_mode: "Paused: OMP's own loop mode was switched on",
  omp_plan_mode: "Paused: OMP's plan mode was switched on",
  omp_autoresearch: "Paused: OMP's autoresearch was switched on",
};

// `autopilot_condition::END_*` → the spec's texts, word for word.
const END_TEXTS: Record<string, string> = {
  stopped: "Stopped",
  iteration_limit: "Ended: iteration limit reached",
  time_limit: "Ended: time limit reached",
  time_limit_before_next: "Ended: the time limit ends before the next iteration is due",
  no_tool_call: "Ended: the last iteration made no tool call",
  conversation_changed: "Ended: the conversation was changed in the terminal",
};

/** The pause the tab's machine reports itself, after the tab's agent process
 *  exited. Nothing can resume it or apply a stop until the agent runs again. */
export const AUTOPILOT_PAUSE_AGENT_EXITED = "agent_exited";

/** Why an autopilot ended. */
export function autopilotEndedText(reason: string | null | undefined): string {
  return END_TEXTS[reason ?? ""] ?? "Ended";
}

/** A tab is on autopilot while its autopilot is starting, running, waiting or
 *  holding; a paused one is not on. Mirrors `AutopilotCondition::is_on`. */
export function autopilotConditionIsOn(kind: string): boolean {
  return kind !== "paused";
}

/** What a current autopilot is doing, in words. */
export function autopilotConditionText(
  condition: { kind: string; detail?: string | null },
  nextIterationAt?: string | null,
  formatTime: AutopilotTimeFormatter = formatAutopilotTime,
): string {
  switch (condition.kind) {
    case "starting":
      return "Starting";
    case "running":
      return "Running an iteration";
    case "waiting":
      switch (condition.detail) {
        case "next_iteration": {
          const at = nextIterationAt ? formatTime(nextIterationAt) : null;
          return at ? `Waiting for the next iteration at ${at}` : "Waiting for the next iteration";
        }
        case "settle":
          return "Waiting for the tab to settle";
        case "background_work":
          return "Waiting for background work";
        default:
          return "Waiting";
      }
    case "holding":
      switch (condition.detail) {
        case "controller":
          return "Holding for APAS on its machine";
        case "server":
          return "Holding for the server";
        default:
          return "Holding";
      }
    case "paused":
      return PAUSE_TEXTS[condition.detail ?? ""] ?? "Paused";
    default:
      return "On autopilot";
  }
}

// ---------------------------------------------------------------------------
// Availability and the actions that apply
// ---------------------------------------------------------------------------

export const AUTOPILOT_NEEDS_AGENT_REBOOT_TEXT =
  "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress.";
export const AUTOPILOT_UNSUPPORTED_OMP_TEXT = "This tab's OMP version cannot run autopilot.";
export const AUTOPILOT_UNAVAILABLE_ON_PLATFORM_TEXT = "Autopilot is not available for tabs on this machine.";
export const AUTOPILOT_AGENT_NOT_RUNNING_TEXT =
  "This tab's agent is not running. Reboot it first; its autopilot comes back paused.";
export const AUTOPILOT_NOT_REPORTED_TEXT = "This tab has not reported its autopilot state.";
/** No request is queued while the page is offline. */
export const AUTOPILOT_OFFLINE_TEXT = "Not connected to the server.";
/** With nothing known per tab, a project that is not running has no instance
 *  to ask either. A tab's own reason comes before this one. */
export const AUTOPILOT_PROJECT_NOT_RUNNING_TEXT = "This project is not running. Open it first.";
/** Offering another start while one is on its way would only be refused. */
export const AUTOPILOT_START_IN_FLIGHT_TEXT = "A start is on its way to this tab.";
/** Shown wherever Stop now is offered. */
export const AUTOPILOT_STOP_NOW_TEXT =
  "Stop now interrupts the turn in progress. Background work already launched keeps running, and a tab that was only waiting for such work can still start a turn when it finishes.";

export function autopilotOlderMachineText(hostname: string | undefined): string {
  return hostname
    ? `${hostname} runs an older APAS that cannot run autopilot. Reboot ${hostname} to update it.`
    : "This tab's machine runs an older APAS that cannot run autopilot. Reboot it to update it.";
}

/** Why autopilot cannot be used on a tab at all, or `undefined`. Not reported
 *  is not such a reason: Stop and Stop now stay available then. */
export function autopilotUnavailableReason(
  support: AutopilotSupport,
  hostname?: string,
): string | undefined {
  switch (support) {
    case "needs_agent_reboot":
      return AUTOPILOT_NEEDS_AGENT_REBOOT_TEXT;
    case "unsupported_omp":
      return AUTOPILOT_UNSUPPORTED_OMP_TEXT;
    case "unavailable_on_platform":
      return AUTOPILOT_UNAVAILABLE_ON_PLATFORM_TEXT;
    case "instance_update_required":
      return autopilotOlderMachineText(hostname);
    default:
      return undefined;
  }
}

/** Not reported, with when the tab last reported if that is known. */
export function autopilotNotReportedText(
  reportedAt?: string | null,
  formatTime: AutopilotTimeFormatter = formatAutopilotTime,
): string {
  const at = reportedAt ? formatTime(reportedAt) : null;
  return at ? `${AUTOPILOT_NOT_REPORTED_TEXT} Last reported at ${at}.` : AUTOPILOT_NOT_REPORTED_TEXT;
}

/** Autopilot applies only to an interactive terminal tab whose provider is
 *  OMP. Legacy bot history is read-only and gets no control. */
export function paneCanHaveAutopilot(
  pane: { kind?: string | null; provider?: string | null; mode?: string | null } | null | undefined,
): boolean {
  return pane?.kind === "terminal" && pane.provider === "omp" && pane.mode !== "deadloop";
}

export type AutopilotPhase = "none" | "on" | "paused" | "ended";
export type AutopilotRequestKind = "start" | "pause" | "resume" | "stop" | "stop_now";

export interface AutopilotControlInput {
  /** What the page holds for the tab; absent reads as not reported. */
  entry: PaneAutopilotEntry | undefined;
  /** The tab's machine, for the older-machine text. */
  hostname?: string;
  /** The page is connected to the server. */
  connected: boolean;
  /** Why the tab cannot be used at all (`TabHost.refusal`): its machine is not
   *  running the project, or this viewer has no runtime access there. The
   *  first comes first; `describeTabHosts` already orders them that way. */
  tabRefusal?: string;
  formatTime?: AutopilotTimeFormatter;
}

export interface AutopilotControlModel {
  phase: AutopilotPhase;
  /** The tab's state is not reported: said so, with Stop and Stop now only. */
  notReported: boolean;
  /** The condition, the ended reason, or that nothing is reported. */
  statusText?: string;
  /** When an ended autopilot ended (RFC 3339). */
  endedAt?: string;
  /** A statement shown with the state that is not a refusal of the listed
   *  actions: the agent is not running, or the state is not reported. */
  notice?: string;
  /** Starting applies: there is no current autopilot to stop first. */
  canStart: boolean;
  /** The actions that apply to the condition, in menu order. */
  actions: Exclude<AutopilotRequestKind, "start">[];
  /** Why starting and every listed action is disabled; absent when they
   *  work. The state stays readable either way. */
  disabledReason?: string;
  autopilot?: AutopilotView;
  /** The last ended autopilot, whose values a new start offers. */
  lastEnded?: AutopilotEnded;
}

/**
 * What a tab's autopilot control shows and offers. One function for the
 * desktop toolbar and the mobile-browser screen, so the same viewer sees the
 * same actions disabled for the same reason on both.
 */
export function autopilotControlModel(input: AutopilotControlInput): AutopilotControlModel {
  const { entry, hostname, connected, tabRefusal, formatTime } = input;
  const support: AutopilotSupport = entry?.support ?? "not_reported";
  // Only a tab that supports autopilot and is reporting has a current one.
  const autopilot = support === "supported" ? entry?.state?.autopilot ?? undefined : undefined;
  const lastEnded = entry?.state?.last_ended ?? undefined;
  // Offline first: nothing can be sent. Then the tab's own reason, which
  // covers a stopped or reconnecting machine and a viewer without access.
  // Only then what autopilot itself lacks on this tab.
  const disabledReason = !connected
    ? AUTOPILOT_OFFLINE_TEXT
    : tabRefusal ?? autopilotUnavailableReason(support, hostname);

  if (autopilot) {
    const on = autopilotConditionIsOn(autopilot.condition.kind);
    // Reported by the tab's machine after the agent process exited: there is
    // no agent to resume and nothing that could apply a stop.
    const agentExited = !on && (entry?.state?.reported_by_cli === true
      || autopilot.condition.detail === AUTOPILOT_PAUSE_AGENT_EXITED);
    return {
      phase: on ? "on" : "paused",
      notReported: false,
      statusText: autopilotConditionText(autopilot.condition, autopilot.next_iteration_at, formatTime),
      notice: agentExited ? AUTOPILOT_AGENT_NOT_RUNNING_TEXT : undefined,
      canStart: false,
      actions: agentExited ? [] : [on ? "pause" : "resume", "stop", "stop_now"],
      disabledReason: agentExited ? undefined : disabledReason,
      autopilot,
    };
  }
  if (support === "not_reported") {
    const notice = autopilotNotReportedText(entry?.reportedAt, formatTime);
    return {
      phase: "none",
      notReported: true,
      statusText: "Not reported",
      notice,
      canStart: false,
      // A stop needs no state: the tab's machine builds it from the launch
      // record alone, so it works when the report is unreadable or stale.
      actions: ["stop", "stop_now"],
      disabledReason,
    };
  }
  return {
    phase: lastEnded ? "ended" : "none",
    notReported: false,
    statusText: lastEnded ? autopilotEndedText(lastEnded.reason) : undefined,
    endedAt: lastEnded?.ended_at,
    canStart: true,
    actions: [],
    disabledReason,
    lastEnded,
  };
}

/** The request for one of the control's actions. A pause or stop carries the
 *  autopilot's id only as a hint; a resume must name it. */
export function autopilotActionRequest(
  kind: Exclude<AutopilotRequestKind, "start">,
  autopilotId: string | undefined,
): AutopilotAction | null {
  switch (kind) {
    case "pause":
      return { kind: "pause", ...(autopilotId ? { autopilot_id: autopilotId } : {}) };
    case "resume":
      return autopilotId ? { kind: "resume", autopilot_id: autopilotId } : null;
    case "stop":
      return { kind: "stop", ...(autopilotId ? { autopilot_id: autopilotId } : {}), now: false };
    case "stop_now":
      return { kind: "stop", ...(autopilotId ? { autopilot_id: autopilotId } : {}), now: true };
  }
}

export function autopilotRequestKind(action: AutopilotAction): AutopilotRequestKind {
  if (action.kind === "stop") return action.now ? "stop_now" : "stop";
  return action.kind;
}

const ACTION_LABELS: Record<AutopilotRequestKind, string> = {
  start: "Start autopilot",
  pause: "Pause",
  resume: "Resume",
  stop: "Stop",
  stop_now: "Stop now",
};

export function autopilotActionLabel(kind: AutopilotRequestKind): string {
  return ACTION_LABELS[kind];
}

const ACTION_DETAILS: Record<Exclude<AutopilotRequestKind, "start">, string> = {
  pause: "No further iteration is submitted. A turn in progress continues.",
  resume: "Continues from the iteration it had reached, with the same limits.",
  stop: "Ends autopilot. A turn in progress runs to its end.",
  stop_now: AUTOPILOT_STOP_NOW_TEXT,
};

/** What an action does, shown under its label wherever it is offered. */
export function autopilotActionDetail(kind: Exclude<AutopilotRequestKind, "start">): string {
  return ACTION_DETAILS[kind];
}

// ---------------------------------------------------------------------------
// Requests and their receipts
// ---------------------------------------------------------------------------

/**
 * `pending` waits for the receipt. `accepted` means the tab took the request;
 * the displayed state changes only when the tab reports it. `refused` carries
 * the reason. `unconfirmed` means no answer can be relied on: a pause or stop
 * stays in force until the tab applies it, a start or resume may or may not
 * have been taken. Nothing is ever sent again by the page.
 */
export type AutopilotRequestStatus = "pending" | "accepted" | "refused" | "unconfirmed";

export interface AutopilotRequest {
  requestId: string;
  sessionId: string;
  paneId: number;
  kind: AutopilotRequestKind;
  status: AutopilotRequestStatus;
  /** The server's reason for a refusal. */
  reason?: string;
  /** Why an unconfirmed request has no answer. */
  cause?: "uncertain" | "connection" | "timeout";
  sentAt: number;
}

const REQUEST_TITLES: Record<AutopilotRequestKind, string> = {
  start: "Start autopilot",
  pause: "Pause autopilot",
  resume: "Resume autopilot",
  stop: "Stop autopilot",
  stop_now: "Stop autopilot now",
};

/** A request named in full, where "Stop" alone would not say what stops. */
export function autopilotRequestTitle(kind: AutopilotRequestKind): string {
  return REQUEST_TITLES[kind];
}

/** A start for the tab is waiting for its receipt, or was accepted and the
 *  tab has not reported since. */
export function autopilotStartInFlight(
  requests: Record<string, AutopilotRequest>,
  sessionId: string,
  paneId: number,
): boolean {
  return Object.values(requests).some((request) =>
    request.sessionId === sessionId && request.paneId === paneId && request.kind === "start"
    && (request.status === "pending" || request.status === "accepted"));
}

/** One session's requests, oldest first, as its request list shows them. */
export function autopilotRequestsOfSession(
  requests: Record<string, AutopilotRequest>,
  sessionId: string | null,
): AutopilotRequest[] {
  return Object.values(requests)
    .filter((request) => request.sessionId === sessionId)
    .sort((a, b) => a.sentAt - b.sentAt);
}

export interface AutopilotRequestNotice {
  title: string;
  detail: string;
  /** Still waiting; nothing to dismiss yet. */
  inFlight: boolean;
  tone: "progress" | "error" | "warning";
}

/** One request as the request list shows it. */
export function autopilotRequestNotice(request: AutopilotRequest, tabLabel?: string): AutopilotRequestNotice {
  const title = tabLabel ? `${REQUEST_TITLES[request.kind]} on ${tabLabel}` : REQUEST_TITLES[request.kind];
  switch (request.status) {
    case "pending":
      return { title, detail: "Sending…", inFlight: true, tone: "progress" };
    case "accepted":
      return { title, detail: "Accepted. The state changes when the tab reports it.", inFlight: true, tone: "progress" };
    case "refused":
      return { title, detail: request.reason || "The request was refused.", inFlight: false, tone: "error" };
    case "unconfirmed": {
      const cause = request.cause === "connection"
        ? "Connection lost before the answer arrived."
        : request.cause === "timeout"
          ? "The server did not answer."
          : "Not confirmed.";
      const outcome = request.kind === "start" || request.kind === "resume"
        ? "The tab may or may not have taken it; its reported state shows which. It was not sent again."
        : "It stays in force until the tab applies it or its agent is restarted. It was not sent again.";
      return { title, detail: `${cause} ${outcome}`, inFlight: false, tone: "warning" };
    }
  }
}

// ---------------------------------------------------------------------------
// One autopilot's values, for the details view and the start dialog
// ---------------------------------------------------------------------------

function plural(count: number, unit: string): string {
  return `${count} ${unit}${count === 1 ? "" : "s"}`;
}

/** "15 minutes", "8 hours", "1 day 6 hours". */
export function formatAutopilotDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0 seconds";
  let rest = Math.round(seconds);
  const parts: string[] = [];
  for (const [unit, size] of [["day", 86_400], ["hour", 3_600], ["minute", 60], ["second", 1]] as const) {
    const count = Math.floor(rest / size);
    if (count > 0) parts.push(plural(count, unit));
    rest -= count * size;
  }
  return parts.join(" ");
}

export const AUTOPILOT_NO_LIMIT_TEXT = "No limit";

export function autopilotRepeatText(repeat: AutopilotRepeat): string {
  return repeat.kind === "every"
    ? `Every ${formatAutopilotDuration(repeat.seconds)}`
    : "As soon as each turn ends";
}

export function autopilotIterationLimitText(limit: AutopilotLimit): string {
  return limit.kind === "at_most" ? plural(limit.value, "iteration") : AUTOPILOT_NO_LIMIT_TEXT;
}

export function autopilotTimeLimitText(limit: AutopilotLimit): string {
  return limit.kind === "at_most" ? formatAutopilotDuration(limit.value) : AUTOPILOT_NO_LIMIT_TEXT;
}

/** "7 of 20 iterations", or "300 iterations" without an iteration limit. */
export function autopilotProgressText(iterations: number, limit: AutopilotLimit): string {
  return limit.kind === "at_most"
    ? `${iterations} of ${plural(limit.value, "iteration")}`
    : plural(iterations, "iteration");
}

export interface AutopilotDetailRow {
  label: string;
  value: string;
}

/** What Details shows beside the prompt: the timing, the limits or that none
 *  is set, when it started, the progress and, with a time limit, its end. */
export function autopilotDetailRows(
  autopilot: Pick<AutopilotView, "spec" | "started_at" | "iterations"> & { deadline?: string | null },
  formatDateTime: AutopilotTimeFormatter = formatAutopilotDateTime,
): AutopilotDetailRow[] {
  const { spec } = autopilot;
  const rows: AutopilotDetailRow[] = [
    { label: "Repeats", value: autopilotRepeatText(spec.repeat) },
    { label: "Iteration limit", value: autopilotIterationLimitText(spec.iteration_limit) },
    { label: "Time limit", value: autopilotTimeLimitText(spec.time_limit) },
  ];
  if (spec.end_without_tool_call) {
    rows.push({ label: "Also ends", value: "When an iteration makes no tool call" });
  }
  rows.push(
    { label: "Started", value: formatDateTime(autopilot.started_at) ?? autopilot.started_at },
    { label: "Progress", value: autopilotProgressText(autopilot.iterations, spec.iteration_limit) },
  );
  if (autopilot.deadline) {
    rows.push({ label: "Time limit ends", value: formatDateTime(autopilot.deadline) ?? autopilot.deadline });
  }
  return rows;
}

export type AutopilotDurationUnit = "minutes" | "hours" | "days";

const UNIT_SECONDS: Record<AutopilotDurationUnit, number> = {
  minutes: 60,
  hours: 3_600,
  days: 86_400,
};

/** The start dialog's fields, as typed. */
export interface AutopilotStartForm {
  prompt: string;
  repeat: "after_each_turn" | "every";
  intervalValue: string;
  intervalUnit: Exclude<AutopilotDurationUnit, "days">;
  /** `false` is "No limit", which is what the dialog proposes. */
  iterationLimited: boolean;
  iterationLimit: string;
  timeLimited: boolean;
  timeLimitValue: string;
  timeLimitUnit: AutopilotDurationUnit;
  endWithoutToolCall: boolean;
}

function splitDuration<U extends AutopilotDurationUnit>(seconds: number, units: U[]): { value: string; unit: U } {
  for (const unit of units) {
    if (seconds % UNIT_SECONDS[unit] === 0) return { value: String(seconds / UNIT_SECONDS[unit]), unit };
  }
  // Not a whole number of minutes: keep it exact as a fraction of one.
  const smallest = units[units.length - 1];
  return { value: String(Number((seconds / UNIT_SECONDS[smallest]).toFixed(6))), unit: smallest };
}

/**
 * The dialog's proposal. With no earlier autopilot: no prompt of APAS's own,
 * "as soon as each turn ends", no limits, and the no-tool-call ending on,
 * which is proposed only for that timing. With one, that autopilot's prompt,
 * timing and limits, ready to change.
 */
export function autopilotStartForm(last?: AutopilotSpec): AutopilotStartForm {
  const form: AutopilotStartForm = {
    prompt: "",
    repeat: "after_each_turn",
    intervalValue: "15",
    intervalUnit: "minutes",
    iterationLimited: false,
    iterationLimit: "",
    timeLimited: false,
    timeLimitValue: "",
    timeLimitUnit: "hours",
    endWithoutToolCall: true,
  };
  if (!last) return form;
  form.prompt = last.prompt;
  form.endWithoutToolCall = last.end_without_tool_call;
  if (last.repeat.kind === "every") {
    const interval = splitDuration(last.repeat.seconds, ["hours", "minutes"]);
    form.repeat = "every";
    form.intervalValue = interval.value;
    form.intervalUnit = interval.unit;
  }
  if (last.iteration_limit.kind === "at_most") {
    form.iterationLimited = true;
    form.iterationLimit = String(last.iteration_limit.value);
  }
  if (last.time_limit.kind === "at_most") {
    const limit = splitDuration(last.time_limit.value, ["days", "hours", "minutes"]);
    form.timeLimited = true;
    form.timeLimitValue = limit.value;
    form.timeLimitUnit = limit.unit;
  }
  return form;
}

function refusedControlCharacter(prompt: string): boolean {
  // U+0000..U+001F except newline and tab, U+007F, and U+0080..U+009F.
  for (let index = 0; index < prompt.length; index += 1) {
    const code = prompt.charCodeAt(index);
    if ((code <= 0x1f && code !== 0x0a && code !== 0x09) || (code >= 0x7f && code <= 0x9f)) return true;
  }
  return false;
}

function typedSeconds(value: string, unit: AutopilotDurationUnit): number | null {
  const amount = Number(value.trim());
  if (!value.trim() || !Number.isFinite(amount)) return null;
  return Math.round(amount * UNIT_SECONDS[unit]);
}

/**
 * The spec a filled-in dialog asks for, or why it is refused before anything
 * is sent. The texts are the server's own (`AutopilotSpec::validate`), so a
 * refusal reads the same wherever it is caught.
 */
export function autopilotStartSpec(
  form: AutopilotStartForm,
): { spec: AutopilotSpec; error?: undefined } | { spec?: undefined; error: string } {
  if (!form.prompt.trim()) return { error: "Autopilot needs a prompt." };
  if (new TextEncoder().encode(form.prompt).length > AUTOPILOT_PROMPT_MAX_BYTES) {
    return { error: `The prompt is longer than ${AUTOPILOT_PROMPT_MAX_BYTES} bytes.` };
  }
  if (refusedControlCharacter(form.prompt)) {
    return { error: "The prompt contains control characters other than newline and tab." };
  }
  let repeat: AutopilotRepeat = { kind: "after_each_turn" };
  if (form.repeat === "every") {
    const seconds = typedSeconds(form.intervalValue, form.intervalUnit);
    if (seconds == null || seconds < AUTOPILOT_INTERVAL_MIN_SECS || seconds > AUTOPILOT_INTERVAL_MAX_SECS) {
      return { error: "The repeat interval must be between one minute and 24 hours." };
    }
    repeat = { kind: "every", seconds };
  }
  let iterationLimit: AutopilotLimit = { kind: "none" };
  if (form.iterationLimited) {
    const value = Number(form.iterationLimit.trim());
    if (!form.iterationLimit.trim() || !Number.isInteger(value) || value < 1 || value > AUTOPILOT_ITERATION_LIMIT_MAX) {
      return { error: `The iteration limit must be between 1 and ${AUTOPILOT_ITERATION_LIMIT_MAX}, or none.` };
    }
    iterationLimit = { kind: "at_most", value };
  }
  let timeLimit: AutopilotLimit = { kind: "none" };
  if (form.timeLimited) {
    const seconds = typedSeconds(form.timeLimitValue, form.timeLimitUnit);
    if (seconds == null || seconds < AUTOPILOT_TIME_LIMIT_MIN_SECS || seconds > AUTOPILOT_TIME_LIMIT_MAX_SECS) {
      return { error: "The time limit must be between one minute and 365 days, or none." };
    }
    if (repeat.kind === "every" && seconds < repeat.seconds) {
      return { error: "The time limit is shorter than the repeat interval." };
    }
    timeLimit = { kind: "at_most", value: seconds };
  }
  return {
    spec: {
      prompt: form.prompt,
      repeat,
      iteration_limit: iterationLimit,
      time_limit: timeLimit,
      end_without_tool_call: form.endWithoutToolCall,
    },
  };
}

/**
 * What the start dialog states before anything is sent: what the mode is and
 * is not. It is the same for everyone who may start, whoever hosts the tab.
 */
export function autopilotStatement(input: { hostname?: string; sharedCheckout: boolean }): string[] {
  const host = input.hostname ?? "its machine";
  return [
    `This tab keeps running on ${host}, as the account APAS runs under there and with the tab's own permissions, including tools that run without asking. It can change, commit and push from ${host}, and it spends the OMP login configured there, whoever starts it.`,
    "APAS adds no sandbox, no independent review and no spending limit.",
    "Limits are checked between turns, so a turn in progress can outlast them. They are kept by code inside the agent's own process and are not a security boundary.",
    "Everyone who can open this session can read the prompt. Slash commands and skills in the prompt are not run.",
    "A question the agent asks waits until someone answers it, and it may have to be answered in the terminal.",
    input.sharedCheckout
      ? "This tab works in the project's shared checkout."
      : "This tab works in its own worktree, not in the project's shared checkout.",
  ];
}

/** What the dialog says about the limits as chosen so far. */
export function autopilotLimitsStatement(
  form: Pick<AutopilotStartForm, "repeat" | "iterationLimited" | "iterationLimit" | "timeLimited" | "timeLimitValue" | "timeLimitUnit">,
): string[] {
  const lines: string[] = [];
  const iterations = form.iterationLimited ? Number(form.iterationLimit.trim()) : null;
  const seconds = form.timeLimited ? typedSeconds(form.timeLimitValue, form.timeLimitUnit) : null;
  const chosen = [
    iterations != null && Number.isInteger(iterations) && iterations > 0 ? plural(iterations, "iteration") : null,
    seconds != null && seconds > 0 ? formatAutopilotDuration(seconds) : null,
  ].filter((value): value is string => value !== null);
  if (!form.iterationLimited && !form.timeLimited) {
    lines.push("No limit is set. Autopilot then runs until someone stops it.");
  } else if (chosen.length === 2) {
    lines.push(`Autopilot ends after ${chosen[0]} or ${chosen[1]}, whichever comes first.`);
  } else if (chosen.length === 1) {
    lines.push(`Autopilot ends after ${chosen[0]}.`);
  }
  if (form.repeat === "every") {
    lines.push("A timed repeat has no end of its own: it runs until a limit is reached or a person stops it.");
  }
  return lines;
}

// ---------------------------------------------------------------------------
// Lists: the pane summary's autopilot block
// ---------------------------------------------------------------------------

/** `PaneAutopilotSummary::CONDITION_ENDED`. */
export const AUTOPILOT_SUMMARY_ENDED = "ended";

/**
 * The list summary of a tab's reported state, as the server builds it
 * (`PaneAutopilotSummary::from_state`). Lets a pushed state update a list
 * entry between two polls of the session list. It never carries the prompt.
 */
export function autopilotSummaryFromState(
  support: AutopilotSupport,
  state: PaneAutopilotState | null | undefined,
): PaneAutopilotSummary {
  const summary: PaneAutopilotSummary = { support };
  if (!state) return summary;
  if (state.autopilot) {
    summary.condition = state.autopilot.condition.kind;
    if (state.autopilot.condition.detail) summary.detail = state.autopilot.condition.detail;
    if (state.autopilot.next_iteration_at) summary.next_iteration_at = state.autopilot.next_iteration_at;
    summary.iterations = state.autopilot.iterations;
    if (state.autopilot.spec.iteration_limit.kind === "at_most") {
      summary.iteration_limit = state.autopilot.spec.iteration_limit.value;
    }
  } else if (state.last_ended && !state.last_ended.turn_since) {
    summary.condition = AUTOPILOT_SUMMARY_ENDED;
    summary.detail = state.last_ended.reason;
    summary.iterations = state.last_ended.iterations;
    if (state.last_ended.spec.iteration_limit.kind === "at_most") {
      summary.iteration_limit = state.last_ended.spec.iteration_limit.value;
    }
  }
  if (state.omp_modes?.goal) summary.omp_goal = state.omp_modes.goal.status;
  if (state.omp_modes?.loop) summary.omp_loop = state.omp_modes.loop.status;
  return summary;
}

/** A tab's autopilot as a list shows it: on, paused, or ended with a reason
 *  lists still show. */
export interface AutopilotListStatus {
  kind: "on" | "paused" | "ended";
  text: string;
}

export function autopilotListStatus(
  summary: PaneAutopilotSummary | null | undefined,
  formatTime: AutopilotTimeFormatter = formatAutopilotTime,
): AutopilotListStatus | null {
  // A block that is not reported, or is for a tab that cannot run autopilot,
  // has no condition to show, whatever it carries.
  const condition = summary?.support === "supported" ? summary.condition : undefined;
  if (!summary || !condition) return null;
  if (condition === AUTOPILOT_SUMMARY_ENDED) {
    return { kind: "ended", text: autopilotEndedText(summary.detail) };
  }
  return {
    kind: autopilotConditionIsOn(condition) ? "on" : "paused",
    text: autopilotConditionText(
      { kind: condition, detail: summary.detail },
      summary.next_iteration_at,
      formatTime,
    ),
  };
}

/** The tab is on autopilot: starting, running, waiting or holding. Such a tab
 *  is not an idle tab waiting for its user. */
export function paneIsOnAutopilot(summary: PaneAutopilotSummary | null | undefined): boolean {
  return autopilotListStatus(summary)?.kind === "on";
}

/**
 * The summary to show for one tab: from the state the page holds when it
 * holds one, else the session list's block, which is all a list of other
 * projects has.
 */
export function paneAutopilotSummary(
  entry: PaneAutopilotEntry | undefined,
  listed: PaneAutopilotSummary | null | undefined,
): PaneAutopilotSummary | null {
  if (entry) return autopilotSummaryFromState(entry.support, entry.state);
  return listed ?? null;
}

/** What a tab's marker says, spelled out for a screen reader. */
export function autopilotMarkerLabel(kind: AutopilotListStatus["kind"]): string | null {
  if (kind === "on") return "on autopilot";
  if (kind === "paused") return "autopilot paused";
  return null;
}

// ---------------------------------------------------------------------------
// The conversation's iteration marker
// ---------------------------------------------------------------------------

export interface AutopilotIterationMarker {
  iteration: number;
  /** Absent when the autopilot has no iteration limit. */
  limit?: number;
  /** On the first marker of an autopilot and on the first after each resume. */
  prompt?: string;
}

/** The input of the reserved tool-use block, or `null` if it is not one. */
export function parseAutopilotIterationMarker(input: unknown): AutopilotIterationMarker | null {
  if (!input || typeof input !== "object") return null;
  const value = input as Record<string, unknown>;
  if (typeof value.iteration !== "number" || !Number.isFinite(value.iteration)) return null;
  return {
    iteration: value.iteration,
    limit: typeof value.limit === "number" && Number.isFinite(value.limit) ? value.limit : undefined,
    prompt: typeof value.prompt === "string" && value.prompt ? value.prompt : undefined,
  };
}

/** "Autopilot · iteration 3 of 20", or "Autopilot · iteration 3". */
export function autopilotIterationMarkerText(marker: Pick<AutopilotIterationMarker, "iteration" | "limit">): string {
  return marker.limit != null
    ? `Autopilot · iteration ${marker.iteration} of ${marker.limit}`
    : `Autopilot · iteration ${marker.iteration}`;
}

// ---------------------------------------------------------------------------
// OMP's own goal and loop modes: shown, never switched
// ---------------------------------------------------------------------------

export interface OmpModeNotice {
  mode: "goal" | "loop";
  paused: boolean;
  title: string;
  /** What OMP reports about the mode. */
  facts: string[];
  /** How to stop it in the terminal. APAS does not stop it. */
  howToStop: string;
  /** A message typed into a looping tab replaces the loop's prompt. */
  warning?: string;
}

/**
 * What to say about the modes OMP reports for a tab. Empty when it reports
 * none: the instructions are shown only while the mode is reported, because
 * `/loop` is a toggle and typing it after a loop has ended switches a new one
 * on.
 */
export function ompModeNotices(modes: OmpModesView | null | undefined): OmpModeNotice[] {
  const notices: OmpModeNotice[] = [];
  const goal = modes?.goal;
  if (goal) {
    const paused = goal.status === "paused";
    const facts: string[] = [];
    if (goal.objective) facts.push(`Objective: ${goal.objective}`);
    if (goal.token_budget != null) facts.push(`Token budget: ${goal.token_budget.toLocaleString("en-US")}`);
    if (goal.tokens_used != null) facts.push(`Tokens used: ${goal.tokens_used.toLocaleString("en-US")}`);
    if (goal.goal_status && goal.goal_status !== goal.status) facts.push(`Goal status: ${goal.goal_status}`);
    notices.push({
      mode: "goal",
      paused,
      title: paused
        ? "This tab is in OMP's own goal mode, paused."
        : "This tab is in OMP's own goal mode and continues by itself.",
      facts,
      howToStop: "To stop it, use the terminal: Esc pauses it, and /goal drop removes the goal.",
    });
  }
  const loop = modes?.loop;
  if (loop) {
    const paused = loop.status === "paused";
    notices.push({
      mode: "loop",
      paused,
      title: paused
        ? "This tab is in OMP's own loop mode, paused."
        : "This tab is in OMP's own loop mode and continues by itself.",
      facts: [],
      howToStop: "To stop it, use the terminal: Esc pauses it, and /loop switches the loop off.",
      warning: "A message sent to this tab now becomes the prompt the loop repeats.",
    });
  }
  return notices;
}

// ---------------------------------------------------------------------------
// Confirmations that affect a tab's autopilot
// ---------------------------------------------------------------------------

/** Added to the confirmation for closing a tab. */
export function closeTabAutopilotNote(status: AutopilotListStatus | null): string | undefined {
  if (status?.kind === "on") {
    return "This tab is on autopilot. Closing the tab ends its autopilot together with the tab's agent.";
  }
  if (status?.kind === "paused") {
    return "This tab has a paused autopilot. Closing the tab discards it.";
  }
  return undefined;
}

/** Added to the confirmation for rebooting a tab's agent. */
export function rebootAgentAutopilotNote(status: AutopilotListStatus | null): string | undefined {
  if (status?.kind === "on") {
    return "This tab is on autopilot. After the reboot its autopilot comes back paused, and stays paused until someone resumes it.";
  }
  if (status?.kind === "paused") {
    return "This tab's autopilot stays paused after the reboot, until someone resumes it.";
  }
  return undefined;
}

/** "zoo-005", "zoo-005 and zoo-006", "a, b and c". */
function nameList(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** Added to the confirmation for closing the project: it names each tab that
 *  is on autopilot. */
export function closeProjectAutopilotNote(tabLabels: string[]): string | undefined {
  if (tabLabels.length === 0) return undefined;
  return tabLabels.length === 1
    ? `${tabLabels[0]} is on autopilot. Its autopilot comes back paused when the project is started again.`
    : `${nameList(tabLabels)} are on autopilot. Their autopilots come back paused when the project is started again.`;
}
