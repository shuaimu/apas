import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { validateClientMessage, type AutopilotSpec, type PaneAutopilotState } from "@apas/protocol";
import {
  AUTOPILOT_AGENT_NOT_RUNNING_TEXT,
  AUTOPILOT_INTERVAL_MAX_SECS,
  AUTOPILOT_INTERVAL_MIN_SECS,
  AUTOPILOT_ITERATION_LIMIT_MAX,
  AUTOPILOT_ITERATION_TOOL_NAME,
  AUTOPILOT_NEEDS_AGENT_REBOOT_TEXT,
  AUTOPILOT_NOT_REPORTED_TEXT,
  AUTOPILOT_OFFLINE_TEXT,
  AUTOPILOT_PROMPT_MAX_BYTES,
  AUTOPILOT_STOP_NOW_TEXT,
  AUTOPILOT_TIME_LIMIT_MAX_SECS,
  AUTOPILOT_TIME_LIMIT_MIN_SECS,
  AUTOPILOT_UNAVAILABLE_ON_PLATFORM_TEXT,
  AUTOPILOT_UNSUPPORTED_OMP_TEXT,
  TAB_AUTOPILOT_CAPABILITY,
  autopilotActionDetail,
  autopilotActionRequest,
  autopilotConditionText,
  autopilotControlModel,
  autopilotDetailRows,
  autopilotEndedText,
  autopilotIterationMarkerText,
  autopilotLimitsStatement,
  autopilotListStatus,
  autopilotMarkerLabel,
  autopilotNotReportedText,
  autopilotProgressText,
  autopilotRequestNotice,
  autopilotRequestTitle,
  autopilotRequestsOfSession,
  autopilotStartForm,
  autopilotStartInFlight,
  autopilotStartSpec,
  autopilotStatement,
  autopilotSummaryFromState,
  closeProjectAutopilotNote,
  closeTabAutopilotNote,
  formatAutopilotDuration,
  ompModeNotices,
  paneAutopilotSummary,
  paneCanHaveAutopilot,
  paneIsOnAutopilot,
  parseAutopilotIterationMarker,
  parsePaneAutopilotFrame,
  rebootAgentAutopilotNote,
  type AutopilotRequest,
  type PaneAutopilotEntry,
} from "./autopilot";
import { noRuntimeAccessReason, stoppedOnHostReason } from "./tabHosts";

const RUST = readFileSync(resolve(__dirname, "../../../../crates/shared/src/autopilot.rs"), "utf8");

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8"));
}

/** `pub const NAME: &str = "value";` of the shared crate, by prefix. */
function rustCodes(prefix: string): string[] {
  return [...RUST.matchAll(new RegExp(`pub const ${prefix}[A-Z_]*: &str = "([a-z_]+)";`, "g"))]
    .map((match) => match[1]);
}

function rustNumber(name: string): number {
  const match = RUST.match(new RegExp(`pub const ${name}: \\w+ = ([^;]+);`));
  if (!match) throw new Error(`no constant ${name}`);
  // The shared crate writes them as products, such as `24 * 60 * 60`.
  return match[1].replaceAll("_", "").split("*").reduce((product, part) => product * Number(part.trim()), 1);
}

const fixedTime = (iso: string) => iso.slice(11, 16);

const SPEC: AutopilotSpec = {
  prompt: "Check the build and fix what fails.",
  repeat: { kind: "every", seconds: 900 },
  iteration_limit: { kind: "at_most", value: 20 },
  time_limit: { kind: "none" },
  end_without_tool_call: false,
};

function state(
  kind: string,
  detail?: string,
  extra: Partial<NonNullable<PaneAutopilotState["autopilot"]>> = {},
): PaneAutopilotState {
  return {
    autopilot: {
      autopilot_id: "60000000-0000-4000-8000-000000000001",
      spec: SPEC,
      started_at: "2026-11-13T14:00:00Z",
      iterations: 7,
      condition: { kind, detail },
      ...extra,
    },
  };
}

function supported(value?: PaneAutopilotState): PaneAutopilotEntry {
  return { support: "supported", state: value };
}

describe("the contract this module mirrors", () => {
  it("names the capability, the reserved tool and the bounds as the shared crate does", () => {
    expect(RUST).toContain(`pub const TAB_AUTOPILOT_CAPABILITY: &str = "${TAB_AUTOPILOT_CAPABILITY}";`);
    expect(RUST).toContain(`pub const AUTOPILOT_ITERATION_TOOL_NAME: &str = "${AUTOPILOT_ITERATION_TOOL_NAME}";`);
    expect(AUTOPILOT_PROMPT_MAX_BYTES).toBe(rustNumber("AUTOPILOT_PROMPT_MAX_BYTES"));
    expect(AUTOPILOT_INTERVAL_MIN_SECS).toBe(rustNumber("AUTOPILOT_INTERVAL_MIN_SECS"));
    expect(AUTOPILOT_INTERVAL_MAX_SECS).toBe(rustNumber("AUTOPILOT_INTERVAL_MAX_SECS"));
    expect(AUTOPILOT_TIME_LIMIT_MIN_SECS).toBe(rustNumber("AUTOPILOT_TIME_LIMIT_MIN_SECS"));
    expect(AUTOPILOT_TIME_LIMIT_MAX_SECS).toBe(rustNumber("AUTOPILOT_TIME_LIMIT_MAX_SECS"));
    expect(AUTOPILOT_ITERATION_LIMIT_MAX).toBe(rustNumber("AUTOPILOT_ITERATION_LIMIT_MAX"));
  });

  it("has words of its own for every pause, end, wait and hold code the shared crate defines", () => {
    const pauses = rustCodes("PAUSE_");
    expect(pauses).toHaveLength(11);
    for (const detail of pauses.filter((code) => code !== "requested")) {
      expect(autopilotConditionText({ kind: "paused", detail }), detail).not.toBe("Paused");
    }
    const ends = rustCodes("END_");
    expect(ends).toHaveLength(6);
    for (const reason of ends) expect(autopilotEndedText(reason), reason).not.toBe("Ended");
    for (const detail of rustCodes("WAIT_")) {
      expect(autopilotConditionText({ kind: "waiting", detail }), detail).not.toBe("Waiting");
    }
    for (const detail of rustCodes("HOLD_")) {
      expect(autopilotConditionText({ kind: "holding", detail }), detail).not.toBe("Holding");
    }
  });
});

describe("conditions in words", () => {
  it("uses the specification's texts for every pause", () => {
    const text = (detail: string) => autopilotConditionText({ kind: "paused", detail });
    expect(text("requested")).toBe("Paused");
    expect(text("failures")).toBe("Paused after three failed iterations");
    expect(text("interrupted")).toBe("Paused: the turn was interrupted");
    expect(text("background_wait_ended")).toBe("Paused: the wait for background work was ended in the terminal");
    expect(text("no_control")).toBe("Paused: APAS could not control this tab for more than four hours");
    expect(text("agent_restarted")).toBe("Paused: the agent was restarted");
    expect(text("agent_exited")).toBe("Paused: the agent process exited");
    expect(text("omp_goal_mode")).toBe("Paused: OMP's own goal mode was switched on");
    expect(text("omp_loop_mode")).toBe("Paused: OMP's own loop mode was switched on");
    expect(text("omp_plan_mode")).toBe("Paused: OMP's plan mode was switched on");
    expect(text("omp_autoresearch")).toBe("Paused: OMP's autoresearch was switched on");
  });

  it("uses the specification's texts for every end", () => {
    expect(autopilotEndedText("stopped")).toBe("Stopped");
    expect(autopilotEndedText("iteration_limit")).toBe("Ended: iteration limit reached");
    expect(autopilotEndedText("time_limit")).toBe("Ended: time limit reached");
    expect(autopilotEndedText("time_limit_before_next"))
      .toBe("Ended: the time limit ends before the next iteration is due");
    expect(autopilotEndedText("no_tool_call")).toBe("Ended: the last iteration made no tool call");
    expect(autopilotEndedText("conversation_changed")).toBe("Ended: the conversation was changed in the terminal");
  });

  it("says what a tab on autopilot waits for, with the time of its next iteration", () => {
    expect(autopilotConditionText({ kind: "starting" })).toBe("Starting");
    expect(autopilotConditionText({ kind: "running" })).toBe("Running an iteration");
    expect(autopilotConditionText({ kind: "waiting", detail: "next_iteration" }, "2026-11-13T14:30:00Z", fixedTime))
      .toBe("Waiting for the next iteration at 14:30");
    expect(autopilotConditionText({ kind: "waiting", detail: "next_iteration" }))
      .toBe("Waiting for the next iteration");
    expect(autopilotConditionText({ kind: "waiting", detail: "settle" })).toBe("Waiting for the tab to settle");
    expect(autopilotConditionText({ kind: "waiting", detail: "background_work" })).toBe("Waiting for background work");
    expect(autopilotConditionText({ kind: "holding", detail: "controller" })).toBe("Holding for APAS on its machine");
    expect(autopilotConditionText({ kind: "holding", detail: "server" })).toBe("Holding for the server");
  });

  it("shows a code a newer tab reports as a plain state and never drops it", () => {
    expect(autopilotConditionText({ kind: "paused", detail: "moon_phase" })).toBe("Paused");
    expect(autopilotConditionText({ kind: "waiting", detail: "tide" })).toBe("Waiting");
    expect(autopilotConditionText({ kind: "meditating", detail: "deeply" })).toBe("On autopilot");
    expect(autopilotEndedText("heat_death")).toBe("Ended");
    expect(autopilotListStatus({ support: "supported", condition: "meditating" })?.kind).toBe("on");
  });
});

describe("reading a state frame", () => {
  it("keeps the reported autopilot, OMP's modes and the time of the report", () => {
    const parsed = parsePaneAutopilotFrame(fixture("server-pane-autopilot-state.json"));
    expect(parsed?.sessionId).toBe("20000000-0000-4000-8000-000000000001");
    expect(parsed?.paneId).toBe(4);
    expect(parsed?.entry.support).toBe("supported");
    expect(parsed?.entry.reportedAt).toBe("2026-11-13T14:16:05Z");
    expect(parsed?.entry.state?.autopilot?.spec).toEqual(SPEC);
    expect(parsed?.entry.state?.autopilot?.condition).toMatchObject({ kind: "waiting", detail: "next_iteration" });
    expect(parsed?.entry.state?.omp_modes?.goal).toMatchObject({ status: "paused", objective: "Ship the parser" });
  });

  it("keeps the last ended autopilot and an availability without a state", () => {
    const ended = parsePaneAutopilotFrame(fixture("server-pane-autopilot-state-ended.json"));
    expect(ended?.entry.state?.autopilot).toBeUndefined();
    expect(ended?.entry.state?.last_ended).toMatchObject({ reason: "conversation_changed", iterations: 31 });
    const unavailable = parsePaneAutopilotFrame(fixture("server-pane-autopilot-state-unavailable.json"));
    expect(unavailable?.entry).toEqual({ support: "instance_update_required", reportedAt: undefined });
  });

  it("reads a state it cannot interpret as not reported, not as off", () => {
    const frame = fixture("server-pane-autopilot-state.json") as {
      state: { autopilot: { spec: Record<string, unknown> } };
    } & Record<string, unknown>;
    // A newer spec: a limit this page does not know how to read.
    frame.state.autopilot.spec.time_limit = { kind: "until_sunrise" };
    expect(parsePaneAutopilotFrame(frame)?.entry.support).toBe("not_reported");
    expect(parsePaneAutopilotFrame({ ...fixture("server-pane-autopilot-state-unavailable.json"), support: "on_strike" })
      ?.entry.support).toBe("not_reported");
    expect(parsePaneAutopilotFrame({ type: "pane_autopilot_state", support: "supported" })).toBeNull();
  });

  it("reads a spec with a field it does not know as not reported, as the shared crate refuses one", () => {
    type Frame = { state: { autopilot: { spec: Record<string, unknown> } } } & Record<string, unknown>;
    const withSpec = (change: (spec: Record<string, unknown>) => void) => {
      const frame = fixture("server-pane-autopilot-state.json") as Frame;
      change(frame.state.autopilot.spec);
      return parsePaneAutopilotFrame(frame)?.entry;
    };
    // Untouched, the golden frame reads as what it is.
    expect(withSpec(() => {})?.support).toBe("supported");
    // A newer tab's limit this page would otherwise show the autopilot
    // without, and offer for a new start without.
    for (const change of [
      (spec: Record<string, unknown>) => { spec.token_limit = { kind: "at_most", value: 500_000 }; },
      (spec: Record<string, unknown>) => { spec.iteration_limit = { kind: "at_most", value: 20, soft: true }; },
      (spec: Record<string, unknown>) => { spec.time_limit = { kind: "none", grace: 60 }; },
      (spec: Record<string, unknown>) => { spec.repeat = { kind: "every", seconds: 900, jitter: 30 }; },
    ]) {
      const entry = withSpec(change);
      expect(entry?.support).toBe("not_reported");
      expect(entry?.state).toBeUndefined();
    }
    // Everything else in a report may grow: only the spec is strict.
    const grown = fixture("server-pane-autopilot-state.json") as {
      state: Record<string, unknown> & { autopilot: Record<string, unknown> };
    } & Record<string, unknown>;
    grown.state.future = [1, 2, 3];
    grown.state.autopilot.started_by = "someone";
    expect(parsePaneAutopilotFrame(grown)?.entry.state?.autopilot?.spec).toEqual(SPEC);
  });

  it("holds no state for a tab that is not reported, whatever came with the frame", () => {
    // A machine that sent its last state along with "not reported" must not
    // make the page show that state, with Pause and Resume, as current.
    const stale = parsePaneAutopilotFrame({ ...fixture("server-pane-autopilot-state.json"), support: "not_reported" });
    expect(stale?.entry).toEqual({ support: "not_reported", reportedAt: "2026-11-13T14:16:05Z" });
    const model = autopilotControlModel({ entry: stale?.entry, connected: true, formatTime: fixedTime });
    expect(model).toMatchObject({ notReported: true, canStart: false, actions: ["stop", "stop_now"] });
    expect(model.autopilot).toBeUndefined();
    expect(model.notice).toBe("This tab has not reported its autopilot state. Last reported at 14:16.");
    expect(autopilotListStatus(paneAutopilotSummary(stale?.entry, undefined))).toBeNull();

    // A tab that cannot run autopilot has no current one either; what OMP
    // reports about its own modes is still shown.
    const unsupported = parsePaneAutopilotFrame({
      ...fixture("server-pane-autopilot-state.json"),
      support: "unsupported_omp",
    });
    expect(unsupported?.entry.state?.autopilot).toBeUndefined();
    expect(unsupported?.entry.state?.omp_modes?.goal?.status).toBe("paused");
    expect(autopilotControlModel({ entry: unsupported?.entry, connected: true }))
      .toMatchObject({ phase: "none", actions: [], disabledReason: AUTOPILOT_UNSUPPORTED_OMP_TEXT });
  });
});

describe("what a tab's control offers", () => {
  const usable = { connected: true, hostname: "zoo-006", formatTime: fixedTime };

  it("offers starting when the tab has no autopilot", () => {
    const model = autopilotControlModel({ ...usable, entry: supported() });
    expect(model).toMatchObject({ phase: "none", canStart: true, actions: [], notReported: false });
    expect(model.disabledReason).toBeUndefined();
    expect(model.statusText).toBeUndefined();
  });

  it("offers Pause, Stop and Stop now while the tab is on autopilot", () => {
    for (const [kind, detail] of [["starting"], ["running"], ["waiting", "next_iteration"], ["holding", "server"]]) {
      const model = autopilotControlModel({ ...usable, entry: supported(state(kind, detail)) });
      expect(model.phase).toBe("on");
      expect(model.actions).toEqual(["pause", "stop", "stop_now"]);
      expect(model.canStart).toBe(false);
    }
    const waiting = autopilotControlModel({
      ...usable,
      entry: supported(state("waiting", "next_iteration", { next_iteration_at: "2026-11-13T14:30:00Z" })),
    });
    expect(waiting.statusText).toBe("Waiting for the next iteration at 14:30");
  });

  it("offers Resume, Stop and Stop now for a paused autopilot", () => {
    const model = autopilotControlModel({ ...usable, entry: supported(state("paused", "failures")) });
    expect(model.phase).toBe("paused");
    expect(model.statusText).toBe("Paused after three failed iterations");
    expect(model.actions).toEqual(["resume", "stop", "stop_now"]);
  });

  it("shows an ended autopilot's reason and offers its values for a new start", () => {
    const parsed = parsePaneAutopilotFrame(fixture("server-pane-autopilot-state-ended.json"));
    const model = autopilotControlModel({ ...usable, entry: parsed?.entry });
    expect(model).toMatchObject({
      phase: "ended",
      canStart: true,
      statusText: "Ended: the conversation was changed in the terminal",
      endedAt: "2026-11-13T19:02:11Z",
    });
    expect(model.lastEnded?.spec.prompt).toBe("Check the build and fix what fails.");
  });

  it("offers nothing for an autopilot paused because the agent process exited", () => {
    const exited: PaneAutopilotState = { ...state("paused", "agent_exited"), reported_by_cli: true };
    const model = autopilotControlModel({ ...usable, entry: supported(exited) });
    expect(model.statusText).toBe("Paused: the agent process exited");
    expect(model.actions).toEqual([]);
    expect(model.canStart).toBe(false);
    expect(model.notice).toBe("This tab's agent is not running. Reboot it first; its autopilot comes back paused.");
    expect(model.notice).toBe(AUTOPILOT_AGENT_NOT_RUNNING_TEXT);
    // The mark alone is enough, and so is the reason alone.
    expect(autopilotControlModel({ ...usable, entry: supported({ ...state("paused", "requested"), reported_by_cli: true }) })
      .actions).toEqual([]);
    expect(autopilotControlModel({ ...usable, entry: supported(state("paused", "agent_exited")) }).actions).toEqual([]);
  });

  it("offers Stop and Stop now only while the state is not reported", () => {
    for (const entry of [undefined, { support: "not_reported" as const }]) {
      const model = autopilotControlModel({ ...usable, entry });
      expect(model.notReported).toBe(true);
      expect(model.actions).toEqual(["stop", "stop_now"]);
      expect(model.canStart).toBe(false);
      expect(model.disabledReason).toBeUndefined();
      expect(model.notice).toBe(AUTOPILOT_NOT_REPORTED_TEXT);
    }
    expect(autopilotNotReportedText("2026-11-13T14:16:05Z", fixedTime))
      .toBe("This tab has not reported its autopilot state. Last reported at 14:16.");
    // An entry that says not reported is not reported, even one that still
    // carries a state: no Pause, no Resume, and nothing shown as current.
    const stale = autopilotControlModel({ ...usable, entry: { support: "not_reported", state: state("running") } });
    expect(stale).toMatchObject({ notReported: true, phase: "none", canStart: false, actions: ["stop", "stop_now"] });
    expect(stale.autopilot).toBeUndefined();
    expect(stale.statusText).toBe("Not reported");
  });

  it("disables starting with the reason where autopilot cannot be used", () => {
    const reason = (support: PaneAutopilotEntry["support"]) =>
      autopilotControlModel({ ...usable, entry: { support } });
    expect(reason("needs_agent_reboot")).toMatchObject({
      canStart: true,
      disabledReason: "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress.",
    });
    expect(reason("instance_update_required").disabledReason)
      .toBe("zoo-006 runs an older APAS that cannot run autopilot. Reboot zoo-006 to update it.");
    expect(reason("unsupported_omp").disabledReason).toBe("This tab's OMP version cannot run autopilot.");
    expect(reason("unavailable_on_platform").disabledReason).toBe("Autopilot is not available for tabs on this machine.");
    expect(AUTOPILOT_NEEDS_AGENT_REBOOT_TEXT).toBe(reason("needs_agent_reboot").disabledReason);
    expect(AUTOPILOT_UNSUPPORTED_OMP_TEXT).toBe(reason("unsupported_omp").disabledReason);
    expect(AUTOPILOT_UNAVAILABLE_ON_PLATFORM_TEXT).toBe(reason("unavailable_on_platform").disabledReason);
  });

  it("disables every action, with the tab's own reason, for a viewer without runtime access", () => {
    const tabRefusal = noRuntimeAccessReason("zoo-006");
    const paused = autopilotControlModel({ ...usable, tabRefusal, entry: supported(state("paused", "interrupted")) });
    // The state and its details stay readable.
    expect(paused.statusText).toBe("Paused: the turn was interrupted");
    expect(paused.autopilot?.spec.prompt).toBe(SPEC.prompt);
    expect(paused.actions).toEqual(["resume", "stop", "stop_now"]);
    expect(paused.disabledReason).toBe(tabRefusal);
    expect(autopilotControlModel({ ...usable, tabRefusal, entry: supported() }))
      .toMatchObject({ canStart: true, disabledReason: tabRefusal });
    expect(autopilotControlModel({ ...usable, tabRefusal, entry: undefined }).disabledReason).toBe(tabRefusal);
  });

  it("puts a machine that is not running the project before what autopilot lacks", () => {
    const tabRefusal = stoppedOnHostReason("zoo-005");
    expect(tabRefusal).toBe("zoo-005 is not running this project. Start it on zoo-005 first.");
    expect(autopilotControlModel({ ...usable, tabRefusal, entry: { support: "needs_agent_reboot" } }).disabledReason)
      .toBe(tabRefusal);
    // Stop is refused, never redirected, while that machine is stopped.
    expect(autopilotControlModel({ ...usable, tabRefusal, entry: undefined }))
      .toMatchObject({ actions: ["stop", "stop_now"], disabledReason: tabRefusal });
  });

  it("disables everything while the page is offline", () => {
    const model = autopilotControlModel({ ...usable, connected: false, entry: supported(state("running")) });
    expect(model.disabledReason).toBe(AUTOPILOT_OFFLINE_TEXT);
    expect(model.statusText).toBe("Running an iteration");
  });

  it("builds each action's request; a resume names its autopilot", () => {
    const id = "60000000-0000-4000-8000-000000000001";
    expect(autopilotActionRequest("pause", id)).toEqual({ kind: "pause", autopilot_id: id });
    expect(autopilotActionRequest("resume", id)).toEqual({ kind: "resume", autopilot_id: id });
    expect(autopilotActionRequest("resume", undefined)).toBeNull();
    expect(autopilotActionRequest("stop", id)).toEqual({ kind: "stop", autopilot_id: id, now: false });
    // With nothing reported there is no id to hint at.
    expect(autopilotActionRequest("stop_now", undefined)).toEqual({ kind: "stop", now: true });
  });

  it("says what Stop now does about background work", () => {
    expect(AUTOPILOT_STOP_NOW_TEXT).toContain("interrupts the turn in progress");
    expect(AUTOPILOT_STOP_NOW_TEXT).toContain("Background work already launched keeps running");
    expect(AUTOPILOT_STOP_NOW_TEXT).toContain("only waiting for such work can still start a turn when it finishes");
  });

  it("applies only to interactive OMP terminal tabs", () => {
    expect(paneCanHaveAutopilot({ kind: "terminal", provider: "omp", mode: "interactive" })).toBe(true);
    expect(paneCanHaveAutopilot({ kind: "terminal", provider: "claude", mode: "interactive" })).toBe(false);
    expect(paneCanHaveAutopilot({ kind: "agent", provider: "omp", mode: "interactive" })).toBe(false);
    expect(paneCanHaveAutopilot({ provider: "omp", mode: "interactive" })).toBe(false);
    // Legacy bot history is read-only and is never started through autopilot.
    expect(paneCanHaveAutopilot({ kind: "terminal", provider: "omp", mode: "deadloop" })).toBe(false);
  });
});

describe("requests and receipts", () => {
  const request = (overrides: Partial<AutopilotRequest>): AutopilotRequest => ({
    requestId: "r1",
    sessionId: "s1",
    paneId: 4,
    kind: "stop",
    status: "pending",
    sentAt: 0,
    ...overrides,
  });

  it("describes each outcome", () => {
    expect(autopilotRequestNotice(request({}), "OMP 1"))
      .toMatchObject({ title: "Stop autopilot on OMP 1", inFlight: true, tone: "progress" });
    expect(autopilotRequestNotice(request({ status: "accepted" })).detail)
      .toBe("Accepted. The state changes when the tab reports it.");
    expect(autopilotRequestNotice(request({
      kind: "start",
      status: "refused",
      reason: "This tab is already on autopilot. Stop it first.",
    }))).toMatchObject({
      title: "Start autopilot",
      detail: "This tab is already on autopilot. Stop it first.",
      inFlight: false,
      tone: "error",
    });
  });

  it("says a stop that was not confirmed stays in force, and that nothing was sent again", () => {
    const stop = autopilotRequestNotice(request({ status: "unconfirmed", cause: "uncertain" }));
    expect(stop.tone).toBe("warning");
    expect(stop.detail).toBe(
      "Not confirmed. It stays in force until the tab applies it or its agent is restarted. It was not sent again.",
    );
    const start = autopilotRequestNotice(request({ kind: "start", status: "unconfirmed", cause: "connection" }));
    expect(start.detail).toBe(
      "Connection lost before the answer arrived. The tab may or may not have taken it; its reported state shows which. It was not sent again.",
    );
  });

  it("names a request in full, and says what each action does where it is offered", () => {
    expect(autopilotRequestTitle("stop")).toBe("Stop autopilot");
    expect(autopilotRequestTitle("stop_now")).toBe("Stop autopilot now");
    expect(autopilotActionDetail("stop")).toBe("Ends autopilot. A turn in progress runs to its end.");
    // Wherever Stop now is offered, it says what it leaves running.
    expect(autopilotActionDetail("stop_now")).toBe(AUTOPILOT_STOP_NOW_TEXT);
  });

  it("knows a start is on its way until its tab reports or it is refused", () => {
    const byId = (...requests: AutopilotRequest[]) =>
      Object.fromEntries(requests.map((entry) => [entry.requestId, entry]));
    expect(autopilotStartInFlight(byId(request({ kind: "start" })), "s1", 4)).toBe(true);
    expect(autopilotStartInFlight(byId(request({ kind: "start", status: "accepted" })), "s1", 4)).toBe(true);
    expect(autopilotStartInFlight(byId(request({ kind: "start", status: "refused" })), "s1", 4)).toBe(false);
    expect(autopilotStartInFlight(byId(request({ kind: "start", status: "unconfirmed" })), "s1", 4)).toBe(false);
    // A stop on its way, or another tab's or another session's start, is not one.
    expect(autopilotStartInFlight(byId(request({})), "s1", 4)).toBe(false);
    expect(autopilotStartInFlight(byId(request({ kind: "start" })), "s1", 5)).toBe(false);
    expect(autopilotStartInFlight(byId(request({ kind: "start" })), "s2", 4)).toBe(false);
  });

  it("lists one session's requests oldest first", () => {
    const requests = {
      b: request({ requestId: "b", sentAt: 20 }),
      other: request({ requestId: "other", sessionId: "s2", sentAt: 5 }),
      a: request({ requestId: "a", sentAt: 10 }),
    };
    expect(autopilotRequestsOfSession(requests, "s1").map((entry) => entry.requestId)).toEqual(["a", "b"]);
    expect(autopilotRequestsOfSession(requests, null)).toEqual([]);
  });
});

describe("one autopilot's values", () => {
  it("formats durations, limits and progress", () => {
    expect(formatAutopilotDuration(60)).toBe("1 minute");
    expect(formatAutopilotDuration(900)).toBe("15 minutes");
    expect(formatAutopilotDuration(8 * 3600)).toBe("8 hours");
    expect(formatAutopilotDuration(86_400 + 6 * 3600)).toBe("1 day 6 hours");
    expect(autopilotProgressText(7, { kind: "at_most", value: 20 })).toBe("7 of 20 iterations");
    expect(autopilotProgressText(300, { kind: "none" })).toBe("300 iterations");
    expect(autopilotProgressText(1, { kind: "none" })).toBe("1 iteration");
  });

  it("lists the timing, the limits or that none is set, the start, the progress and the time limit end", () => {
    const rows = autopilotDetailRows({
      spec: { ...SPEC, time_limit: { kind: "at_most", value: 8 * 3600 }, end_without_tool_call: true },
      started_at: "2026-11-13T14:00:00Z",
      iterations: 7,
      deadline: "2026-11-13T22:00:00Z",
    }, fixedTime);
    expect(rows).toEqual([
      { label: "Repeats", value: "Every 15 minutes" },
      { label: "Iteration limit", value: "20 iterations" },
      { label: "Time limit", value: "8 hours" },
      { label: "Also ends", value: "When an iteration makes no tool call" },
      { label: "Started", value: "14:00" },
      { label: "Progress", value: "7 of 20 iterations" },
      { label: "Time limit ends", value: "22:00" },
    ]);
    const unlimited = autopilotDetailRows({
      spec: { ...SPEC, repeat: { kind: "after_each_turn" }, iteration_limit: { kind: "none" } },
      started_at: "2026-11-10T14:00:00Z",
      iterations: 300,
    }, fixedTime);
    expect(unlimited).toEqual([
      { label: "Repeats", value: "As soon as each turn ends" },
      { label: "Iteration limit", value: "No limit" },
      { label: "Time limit", value: "No limit" },
      { label: "Started", value: "14:00" },
      { label: "Progress", value: "300 iterations" },
    ]);
  });
});

describe("the start dialog's form", () => {
  it("proposes no prompt, no limit, and the no-tool-call ending for an immediate repeat", () => {
    const form = autopilotStartForm();
    expect(form).toMatchObject({
      prompt: "",
      repeat: "after_each_turn",
      iterationLimited: false,
      timeLimited: false,
      endWithoutToolCall: true,
    });
    expect(autopilotStartSpec({ ...form, prompt: "go" }).spec).toEqual({
      prompt: "go",
      repeat: { kind: "after_each_turn" },
      iteration_limit: { kind: "none" },
      time_limit: { kind: "none" },
      end_without_tool_call: true,
    });
  });

  it("offers the values of the last autopilot the tab reports", () => {
    const last: AutopilotSpec = {
      prompt: "Check the build.",
      repeat: { kind: "every", seconds: 2 * 3600 },
      iteration_limit: { kind: "at_most", value: 20 },
      time_limit: { kind: "at_most", value: 3 * 86_400 },
      end_without_tool_call: false,
    };
    const form = autopilotStartForm(last);
    expect(form).toMatchObject({
      prompt: "Check the build.",
      repeat: "every",
      intervalValue: "2",
      intervalUnit: "hours",
      iterationLimited: true,
      iterationLimit: "20",
      timeLimited: true,
      timeLimitValue: "3",
      timeLimitUnit: "days",
      endWithoutToolCall: false,
    });
    // Unchanged, it asks for exactly that autopilot again.
    expect(autopilotStartSpec(form).spec).toEqual(last);
    // A value that is not a whole number of minutes survives the round trip.
    const odd = { ...last, repeat: { kind: "every" as const, seconds: 90 }, time_limit: { kind: "none" as const } };
    expect(autopilotStartSpec(autopilotStartForm(odd)).spec).toEqual(odd);
  });

  it("refuses each bound before anything is sent, in the server's words", () => {
    const base = { ...autopilotStartForm(), prompt: "go" };
    const error = (patch: Partial<typeof base>) => autopilotStartSpec({ ...base, ...patch }).error;
    expect(error({ prompt: "  \n" })).toBe("Autopilot needs a prompt.");
    expect(error({ prompt: "x".repeat(16_001) })).toBe("The prompt is longer than 16000 bytes.");
    // Bytes, not characters.
    expect(error({ prompt: "é".repeat(8_001) })).toBe("The prompt is longer than 16000 bytes.");
    expect(error({ prompt: "é".repeat(8_000) })).toBeUndefined();
    expect(error({ prompt: "line\r\n" })).toBe("The prompt contains control characters other than newline and tab.");
    expect(error({ prompt: "c1\u0085" })).toBe("The prompt contains control characters other than newline and tab.");
    expect(error({ prompt: "first\n\tsecond" })).toBeUndefined();
    expect(error({ repeat: "every", intervalValue: "0.1", intervalUnit: "minutes" }))
      .toBe("The repeat interval must be between one minute and 24 hours.");
    expect(error({ repeat: "every", intervalValue: "25", intervalUnit: "hours" }))
      .toBe("The repeat interval must be between one minute and 24 hours.");
    expect(error({ repeat: "every", intervalValue: "", intervalUnit: "minutes" }))
      .toBe("The repeat interval must be between one minute and 24 hours.");
    expect(error({ iterationLimited: true, iterationLimit: "0" }))
      .toBe("The iteration limit must be between 1 and 1000000, or none.");
    expect(error({ iterationLimited: true, iterationLimit: "2.5" }))
      .toBe("The iteration limit must be between 1 and 1000000, or none.");
    expect(error({ iterationLimited: true, iterationLimit: "" }))
      .toBe("The iteration limit must be between 1 and 1000000, or none.");
    expect(error({ timeLimited: true, timeLimitValue: "0.1", timeLimitUnit: "minutes" }))
      .toBe("The time limit must be between one minute and 365 days, or none.");
    expect(error({ timeLimited: true, timeLimitValue: "366", timeLimitUnit: "days" }))
      .toBe("The time limit must be between one minute and 365 days, or none.");
  });

  it("refuses a time limit shorter than the repeat interval", () => {
    const form = {
      ...autopilotStartForm(),
      prompt: "go",
      repeat: "every" as const,
      intervalValue: "15",
      intervalUnit: "minutes" as const,
      timeLimited: true,
      timeLimitValue: "10",
      timeLimitUnit: "minutes" as const,
    };
    expect(autopilotStartSpec(form).error).toBe("The time limit is shorter than the repeat interval.");
    expect(autopilotStartSpec({ ...form, timeLimitValue: "15" }).error).toBeUndefined();
    // With "as soon as each turn ends" there is no interval to be shorter than.
    expect(autopilotStartSpec({ ...form, repeat: "after_each_turn" }).error).toBeUndefined();
  });

  it("asks for a start the protocol accepts, with each limit said explicitly", () => {
    const { spec } = autopilotStartSpec({ ...autopilotStartForm(), prompt: "go" });
    const result = validateClientMessage({
      type: "control_pane_autopilot",
      session_id: "20000000-0000-4000-8000-000000000001",
      pane_id: 4,
      request_id: "autopilot-1",
      action: { kind: "start", spec },
    });
    expect(result.errors).toEqual([]);
    expect(spec?.iteration_limit).toEqual({ kind: "none" });
    expect(spec?.time_limit).toEqual({ kind: "none" });
  });

  it("states what the mode is and is not, naming the machine", () => {
    const text = autopilotStatement({ hostname: "zoo-006", sharedCheckout: true }).join(" ");
    expect(text).toContain("keeps running on zoo-006");
    expect(text).toContain("as the account APAS runs under there");
    expect(text).toContain("the tab's own permissions, including tools that run without asking");
    expect(text).toContain("It can change, commit and push from zoo-006");
    expect(text).toContain("spends the OMP login configured there, whoever starts it");
    expect(text).toContain("APAS adds no sandbox, no independent review and no spending limit.");
    expect(text).toContain("Limits are checked between turns, so a turn in progress can outlast them.");
    expect(text).toContain("kept by code inside the agent's own process and are not a security boundary");
    expect(text).toContain("Everyone who can open this session can read the prompt.");
    expect(text).toContain("Slash commands and skills in the prompt are not run.");
    expect(text).toContain("A question the agent asks waits until someone answers it, and it may have to be answered in the terminal.");
    expect(text).toContain("This tab works in the project's shared checkout.");
    expect(autopilotStatement({ hostname: "zoo-006", sharedCheckout: false }).join(" "))
      .toContain("works in its own worktree, not in the project's shared checkout");
    // Never the worker's vocabulary, and never a spending limit of its own.
    expect(text).not.toMatch(/\b(bot|worker|verified|delivered)\b/i);
  });

  it("says what the chosen limits mean, and what no limit means", () => {
    const form = autopilotStartForm();
    expect(autopilotLimitsStatement(form))
      .toEqual(["No limit is set. Autopilot then runs until someone stops it."]);
    expect(autopilotLimitsStatement({ ...form, repeat: "every" })).toEqual([
      "No limit is set. Autopilot then runs until someone stops it.",
      "A timed repeat has no end of its own: it runs until a limit is reached or a person stops it.",
    ]);
    expect(autopilotLimitsStatement({
      ...form,
      iterationLimited: true,
      iterationLimit: "20",
      timeLimited: true,
      timeLimitValue: "8",
      timeLimitUnit: "hours",
    })).toEqual(["Autopilot ends after 20 iterations or 8 hours, whichever comes first."]);
    expect(autopilotLimitsStatement({ ...form, iterationLimited: true, iterationLimit: "20" }))
      .toEqual(["Autopilot ends after 20 iterations."]);
  });
});

describe("lists", () => {
  it("summarises a state as the server does, without the prompt", () => {
    const waiting = state("waiting", "next_iteration", { next_iteration_at: "2026-11-13T14:30:00Z" });
    const summary = autopilotSummaryFromState("supported", waiting);
    expect(summary).toEqual({
      support: "supported",
      condition: "waiting",
      detail: "next_iteration",
      next_iteration_at: "2026-11-13T14:30:00Z",
      iterations: 7,
      iteration_limit: 20,
    });
    expect(JSON.stringify(summary)).not.toContain("Check the build");

    const ended: PaneAutopilotState = {
      last_ended: {
        autopilot_id: "60000000-0000-4000-8000-000000000001",
        spec: { ...SPEC, iteration_limit: { kind: "none" } },
        started_at: "2026-11-13T14:00:00Z",
        iterations: 12,
        reason: "time_limit",
        ended_at: "2026-11-13T22:00:00Z",
        turn_since: false,
      },
    };
    expect(autopilotSummaryFromState("supported", ended))
      .toEqual({ support: "supported", condition: "ended", detail: "time_limit", iterations: 12 });
    // Once a later turn has run, lists stop showing the ended reason.
    expect(autopilotSummaryFromState("supported", { last_ended: { ...ended.last_ended!, turn_since: true } }))
      .toEqual({ support: "supported" });
    expect(autopilotSummaryFromState("not_reported", undefined)).toEqual({ support: "not_reported" });
    expect(autopilotSummaryFromState("supported", { omp_modes: { loop: { status: "paused" } } }))
      .toEqual({ support: "supported", omp_loop: "paused" });
  });

  it("tells a tab on autopilot from a paused or ended one", () => {
    const on = { support: "supported" as const, condition: "waiting", detail: "next_iteration", next_iteration_at: "2026-11-13T14:30:00Z" };
    expect(autopilotListStatus(on, fixedTime)).toEqual({ kind: "on", text: "Waiting for the next iteration at 14:30" });
    expect(paneIsOnAutopilot(on)).toBe(true);
    expect(paneIsOnAutopilot({ support: "supported", condition: "holding", detail: "server" })).toBe(true);

    const paused = { support: "supported" as const, condition: "paused", detail: "failures" };
    expect(autopilotListStatus(paused)).toEqual({ kind: "paused", text: "Paused after three failed iterations" });
    expect(paneIsOnAutopilot(paused)).toBe(false);

    const ended = { support: "supported" as const, condition: "ended", detail: "iteration_limit" };
    expect(autopilotListStatus(ended)).toEqual({ kind: "ended", text: "Ended: iteration limit reached" });
    expect(paneIsOnAutopilot(ended)).toBe(false);

    for (const nothing of [undefined, null, { support: "supported" as const }, { support: "needs_agent_reboot" as const }]) {
      expect(autopilotListStatus(nothing)).toBeNull();
      expect(paneIsOnAutopilot(nothing)).toBe(false);
    }
    // A block that is not reported, or for a tab that cannot run autopilot,
    // shows no condition, whatever it still carries: such a tab is neither
    // marked nor moved out of the idle tabs.
    for (const support of ["not_reported", "instance_update_required", "unsupported_omp"] as const) {
      expect(autopilotListStatus({ ...on, support })).toBeNull();
      expect(paneIsOnAutopilot({ ...on, support })).toBe(false);
      expect(autopilotListStatus({ ...ended, support })).toBeNull();
    }
  });

  it("prefers the state the page holds over the session list's block", () => {
    const listed = { support: "supported" as const, condition: "running" };
    expect(paneAutopilotSummary(undefined, listed)).toBe(listed);
    expect(paneAutopilotSummary(undefined, undefined)).toBeNull();
    expect(paneAutopilotSummary(supported(state("paused", "requested")), listed)?.condition).toBe("paused");
    // Held and reporting nothing: the list's older block is not shown.
    expect(paneAutopilotSummary({ support: "not_reported" }, listed)).toEqual({ support: "not_reported" });
  });

  it("spells out the tab marker, and has none for an ended autopilot", () => {
    expect(autopilotMarkerLabel("on")).toBe("on autopilot");
    expect(autopilotMarkerLabel("paused")).toBe("autopilot paused");
    expect(autopilotMarkerLabel("ended")).toBeNull();
  });
});

describe("the conversation's iteration marker", () => {
  it("reads the reserved block's input and says which iteration starts", () => {
    const marker = parseAutopilotIterationMarker({
      autopilot_id: "60000000-0000-4000-8000-000000000001",
      iteration: 3,
      limit: 20,
    });
    expect(marker).toEqual({ iteration: 3, limit: 20, prompt: undefined });
    expect(autopilotIterationMarkerText(marker!)).toBe("Autopilot · iteration 3 of 20");
    const unlimited = parseAutopilotIterationMarker({ autopilot_id: "x", iteration: 3, prompt: "Check the build." });
    expect(unlimited).toEqual({ iteration: 3, limit: undefined, prompt: "Check the build." });
    expect(autopilotIterationMarkerText(unlimited!)).toBe("Autopilot · iteration 3");
    expect(parseAutopilotIterationMarker({ autopilot_id: "x" })).toBeNull();
    expect(parseAutopilotIterationMarker("iteration 3")).toBeNull();
  });
});

describe("OMP's own modes", () => {
  it("shows nothing where OMP reports none", () => {
    expect(ompModeNotices(undefined)).toEqual([]);
    expect(ompModeNotices({})).toEqual([]);
  });

  it("shows a goal with what OMP reports and how to stop it in the terminal", () => {
    const [goal] = ompModeNotices({
      goal: { status: "active", objective: "Ship the parser", token_budget: 200_000, tokens_used: 507, goal_status: "budget-limited" },
    });
    expect(goal.title).toBe("This tab is in OMP's own goal mode and continues by itself.");
    expect(goal.facts).toEqual([
      "Objective: Ship the parser",
      "Token budget: 200,000",
      "Tokens used: 507",
      "Goal status: budget-limited",
    ]);
    expect(goal.howToStop).toBe("To stop it, use the terminal: Esc pauses it, and /goal drop removes the goal.");
    expect(goal.warning).toBeUndefined();
    // Until OMP's first update only the objective and the budget are known.
    expect(ompModeNotices({ goal: { status: "paused", objective: "Ship the parser" } })[0])
      .toMatchObject({ paused: true, title: "This tab is in OMP's own goal mode, paused.", facts: ["Objective: Ship the parser"] });
  });

  it("says that a message sent to a looping tab becomes the loop's prompt, active or paused", () => {
    for (const status of ["active", "paused"]) {
      const [loop] = ompModeNotices({ loop: { status } });
      expect(loop.mode).toBe("loop");
      expect(loop.paused).toBe(status === "paused");
      expect(loop.howToStop).toBe("To stop it, use the terminal: Esc pauses it, and /loop switches the loop off.");
      expect(loop.warning).toBe("A message sent to this tab now becomes the prompt the loop repeats.");
    }
    expect(ompModeNotices({ loop: { status: "paused" } })[0].title).toBe("This tab is in OMP's own loop mode, paused.");
  });
});

describe("confirmations", () => {
  const on = { kind: "on" as const, text: "Running an iteration" };
  const paused = { kind: "paused" as const, text: "Paused" };
  const ended = { kind: "ended" as const, text: "Stopped" };

  it("says what closing a tab does to its autopilot", () => {
    expect(closeTabAutopilotNote(on))
      .toBe("This tab is on autopilot. Closing the tab ends its autopilot together with the tab's agent.");
    expect(closeTabAutopilotNote(paused)).toBe("This tab has a paused autopilot. Closing the tab discards it.");
    expect(closeTabAutopilotNote(ended)).toBeUndefined();
    expect(closeTabAutopilotNote(null)).toBeUndefined();
  });

  it("says that a rebooted agent's autopilot comes back paused", () => {
    expect(rebootAgentAutopilotNote(on)).toBe(
      "This tab is on autopilot. After the reboot its autopilot comes back paused, and stays paused until someone resumes it.",
    );
    expect(rebootAgentAutopilotNote(paused))
      .toBe("This tab's autopilot stays paused after the reboot, until someone resumes it.");
    expect(rebootAgentAutopilotNote(ended)).toBeUndefined();
  });

  it("names the tabs on autopilot when the project is closed", () => {
    expect(closeProjectAutopilotNote([])).toBeUndefined();
    expect(closeProjectAutopilotNote(["OMP 1"]))
      .toBe("OMP 1 is on autopilot. Its autopilot comes back paused when the project is started again.");
    expect(closeProjectAutopilotNote(["OMP 1", "OMP 2", "Nightly"])).toBe(
      "OMP 1, OMP 2 and Nightly are on autopilot. Their autopilots come back paused when the project is started again.",
    );
  });
});
