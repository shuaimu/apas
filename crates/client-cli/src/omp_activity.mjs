// OMP 18.3.2 and 18.6.0: extensibility/shared-events.ts AgentEndEvent,
// extensions/types.ts ExtensionContext and ToolExecution{Start,End}Event.
// In particular, agent_end.willContinue is decided AFTER automatic recovery,
// session_stop hooks, and pending async-wake routing. turn_end is not a settle.
import { closeSync, constants, fstatSync, lstatSync, openSync, readFileSync, readSync, realpathSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join } from "node:path";
import { randomUUID } from "node:crypto";

const MAX_QUESTION_BYTES = 131072;
const MAX_ACK_BYTES = 8192;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

// Autopilot: the tab repeats one prompt by itself. The loop, its limits and
// its state live here, in the tab's own OMP process. File formats and reason
// codes: omp_autopilot.rs and shared/src/autopilot.rs. Behaviour was checked
// against OMP 18.20.0; the minimum is the first version whose sendUserMessage
// rejects for a dropped prompt.
const AUTOPILOT_FORMAT = 1;
const AUTOPILOT_MAX_BYTES = 65536;
const AUTOPILOT_MIN_OMP = [18, 18, 2];
const AUTOPILOT_JOURNAL = "apas-autopilot";
const AUTOPILOT_ACK_KEEP = 16;
const AUTOPILOT_POLL_MS = 1000;
const AUTOPILOT_REPORT_MS = 30000;
const AUTOPILOT_REQUEST_MAX_AGE_MS = 2000;
const AUTOPILOT_SETTLE_MS = 5000;
const AUTOPILOT_FAILURE_WAIT_MS = 60000;
const AUTOPILOT_FAILURE_PAUSE = 3;
const AUTOPILOT_HEARTBEAT_FRESH_MS = 60000;
const AUTOPILOT_SERVER_UNREACHABLE_SECS = 600;
const AUTOPILOT_HOLD_PAUSE_MS = 4 * 60 * 60 * 1000;
// A JavaScript timer cannot be relied on much further ahead than 24 days.
const AUTOPILOT_TIMER_MAX_MS = 24 * 60 * 60 * 1000;
const AUTOPILOT_WORKING_AHEAD_MS = 10000;
const AUTOPILOT_WORKING_LATE_MS = 15000;
const AUTOPILOT_TEXT_MAX = 500;
const AUTOPILOT_STATUS_KEY = "apas-autopilot";
const AUTOPILOT_NOT_TAKEN = "The tab did not take the request in time. Nothing was started.";
const AUTOPILOT_UNREADABLE = "This tab's APAS cannot read this request.";
const AUTOPILOT_UNSUPPORTED = "This tab's OMP version cannot run autopilot.";
const AUTOPILOT_MODE_REFUSALS = {
  omp_goal_mode: "This tab is in OMP's own goal mode. Stop that first.",
  omp_loop_mode: "This tab is in OMP's own loop mode. Stop that first.",
  omp_plan_mode: "This tab is in OMP's plan mode. Leave plan mode first.",
  omp_autoresearch: "This tab is running OMP's autoresearch. Type /autoresearch off in the terminal first.",
};
const AUTOPILOT_PAUSED_TEXT = {
  requested: "Paused",
  failures: "Paused after three failed iterations",
  interrupted: "Paused: the turn was interrupted",
  background_wait_ended: "Paused: the wait for background work was ended in the terminal",
  no_control: "Paused: APAS could not control this tab for more than four hours",
  agent_restarted: "Paused: the agent was restarted",
  omp_goal_mode: "Paused: OMP's own goal mode was switched on",
  omp_loop_mode: "Paused: OMP's own loop mode was switched on",
  omp_plan_mode: "Paused: OMP's plan mode was switched on",
  omp_autoresearch: "Paused: OMP's autoresearch was switched on",
};
const AUTOPILOT_ENDED_TEXT = {
  stopped: "Stopped",
  iteration_limit: "Ended: iteration limit reached",
  time_limit: "Ended: time limit reached",
  time_limit_before_next: "Ended: the time limit ends before the next iteration is due",
  no_tool_call: "Ended: the last iteration made no tool call",
  conversation_changed: "Ended: the conversation was changed in the terminal",
};
// OMP's own texts. An abort does not say who asked for it.
const OMP_PROMPT_DROPPED = "Prompt dropped before provider dispatch.";
const OMP_USER_INTERRUPT = "Interrupted by user";
const OMP_RETRY_CANCELLED = "Retry cancelled";

function privateJson(path, limit) {
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const stat = fstatSync(fd);
    if (!stat.isFile() || stat.uid !== process.geteuid() || (stat.mode & 0o077) !== 0 || stat.size > limit) {
      throw new Error("Insecure or oversized OMP bridge file");
    }
    // Bounded read even if a writer grows the file after fstat.
    const bytes = Buffer.alloc(limit + 1);
    let length = 0;
    while (length < bytes.length) {
      const count = readSync(fd, bytes, length, bytes.length - length, null);
      if (count === 0) break;
      length += count;
    }
    if (length > limit) throw new Error("Oversized OMP bridge file");
    return JSON.parse(bytes.subarray(0, length).toString("utf8"));
  } finally {
    closeSync(fd);
  }
}

function validQuestions(questions) {
  if (!Array.isArray(questions) || questions.length === 0) return false;
  const ids = new Set();
  return questions.every(question => {
    if (!question || typeof question.id !== "string" || typeof question.question !== "string" ||
        ids.has(question.id) || !Array.isArray(question.options) ||
        (question.multi !== undefined && typeof question.multi !== "boolean")) return false;
    ids.add(question.id);
    const labels = new Set();
    return question.options.every(option => {
      if (!option || typeof option.label !== "string" || labels.has(option.label)) return false;
      labels.add(option.label);
      return true;
    });
  });
}

function answerError(questions, answers, cancel) {
  if (typeof cancel !== "boolean" || !Array.isArray(answers)) return "Invalid answer payload";
  if (cancel) return answers.length === 0 ? undefined : "Cancellation cannot include answers";
  if (answers.length !== questions.length) return "Answer every question exactly once";
  for (let index = 0; index < questions.length; index++) {
    const question = questions[index];
    const answer = answers[index];
    if (!answer || answer.id !== question.id || !Array.isArray(answer.selectedOptions) ||
        answer.selectedOptions.some(label => typeof label !== "string" ||
          !question.options.some(option => option.label === label)) ||
        new Set(answer.selectedOptions).size !== answer.selectedOptions.length) return "Invalid question or option";
    if (question.multi !== true && answer.selectedOptions.length > 1) return "Select at most one option";
    if (answer.customInput !== undefined &&
        (typeof answer.customInput !== "string" || answer.customInput.trim().length === 0)) return "Custom input must not be blank";
    if (question.multi !== true && answer.selectedOptions.length === 0 && answer.customInput === undefined) {
      return "A single-select question requires an answer";
    }
  }
}

function onlyKeys(value, keys) {
  return value !== null && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).every(key => keys.includes(key));
}

function wholeNumber(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

// A number, null for an explicit "none", or undefined when the limit is
// missing or malformed: a start must say whether it is limited.
function autopilotLimit(limit) {
  if (!onlyKeys(limit, ["kind", "value"])) return undefined;
  if (limit.kind === "none") return "value" in limit ? undefined : null;
  return limit.kind === "at_most" && wholeNumber(limit.value) ? limit.value : undefined;
}

// The bounds and texts of AutopilotSpec::validate, with the strictness of its
// serde form. This tab enforces the limits, and may be older or newer than
// the CLI that hands it the spec.
function autopilotSpecRefusal(spec) {
  if (!onlyKeys(spec, ["prompt", "repeat", "iteration_limit", "time_limit", "end_without_tool_call"])) {
    return AUTOPILOT_UNREADABLE;
  }
  const prompt = spec.prompt;
  if (typeof prompt !== "string" || prompt.trim().length === 0) return "Autopilot needs a prompt.";
  if (Buffer.byteLength(prompt) > 16000) return "The prompt is longer than 16000 bytes.";
  // A lone surrogate would also make every report with this prompt unreadable.
  if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(prompt) || prompt.isWellFormed?.() === false) {
    return "The prompt contains control characters other than newline and tab.";
  }
  const repeat = spec.repeat;
  let interval;
  if (onlyKeys(repeat, ["kind", "seconds"]) && repeat.kind === "every" && wholeNumber(repeat.seconds)) {
    if (repeat.seconds < 60 || repeat.seconds > 86400) return "The repeat interval must be between one minute and 24 hours.";
    interval = repeat.seconds;
  } else if (!onlyKeys(repeat, ["kind"]) || repeat.kind !== "after_each_turn") return "Say when autopilot repeats.";
  const iterations = autopilotLimit(spec.iteration_limit);
  if (iterations === undefined) return "Say whether autopilot has an iteration limit: a number, or none.";
  if (iterations !== null && (iterations < 1 || iterations > 1000000)) {
    return "The iteration limit must be between 1 and 1000000, or none.";
  }
  const time = autopilotLimit(spec.time_limit);
  if (time === undefined) return "Say whether autopilot has a time limit: a duration, or none.";
  if (time !== null) {
    if (time < 60 || time > 31536000) return "The time limit must be between one minute and 365 days, or none.";
    if (interval !== undefined && time < interval) return "The time limit is shorter than the repeat interval.";
  }
  if (typeof spec.end_without_tool_call !== "boolean") return AUTOPILOT_UNREADABLE;
}

// Text that OMP supplies, such as a goal's objective, is cut by code point: a
// split surrogate pair would make the whole report unreadable to the CLI.
function cutText(text) {
  if (typeof text !== "string") return undefined;
  const points = Array.from(text.toWellFormed?.() ?? text);
  return points.length > AUTOPILOT_TEXT_MAX ? `${points.slice(0, AUTOPILOT_TEXT_MAX - 1).join("")}…` : points.join("");
}

function autopilotVersionProblem(version) {
  const match = typeof version === "string" ? /^(\d+)\.(\d+)\.(\d+)/.exec(version) : null;
  if (!match) return "OMP does not report its version";
  for (let at = 0; at < 3; at++) {
    const part = Number(match[at + 1]);
    if (part !== AUTOPILOT_MIN_OMP[at]) {
      return part > AUTOPILOT_MIN_OMP[at] ? undefined : `OMP ${version} is older than ${AUTOPILOT_MIN_OMP.join(".")}`;
    }
  }
}

export default function apasActivity(omp) {
  const runtime = process.env.APAS_OMP_RUNTIME;
  const launchId = process.env.APAS_OMP_LAUNCH;
  if (!runtime || !launchId || process.platform !== "linux") return;
  const launchPath = join(runtime, "omp-launch.json");
  const reportPath = join(runtime, "omp-activity.json");
  const temporary = join(runtime, `omp-activity.tmp-${process.pid}-${launchId}`);
  const asks = new Set();
  let lastBody;
  let owned = false;
  let revision = 0;
  let exitRegistered = false;
  let settled = true;
  let idleCompactionRevision;
  const questionPath = join(runtime, "omp-question.json");
  const submissionPath = join(runtime, "omp-question-submit.json");
  const ackPath = join(runtime, "omp-question-ack.json");
  const preparedQuestions = new Map();
  const executingQuestions = new Map();
  let current;
  let ackCleanup;
  const wrapped = new WeakSet();
  // Set only when autopilot installed itself; every call into it is guarded.
  let autopilot;
  let rested = "idle";

  function atomicJson(path, value, limit) {
    const body = JSON.stringify(value);
    if (Buffer.byteLength(body) > limit) throw new Error("Oversized OMP bridge payload");
    const temp = join(runtime, `omp-question.tmp-${randomUUID()}`);
    try {
      writeFileSync(temp, body, { mode: 0o600, flag: "wx" });
      renameSync(temp, path);
    } finally {
      try { unlinkSync(temp); } catch {}
    }
  }

  function sameScope(left, right) {
    return right && ["launch_id", "pid", "process_start", "session_id", "transcript_path"]
      .every(key => left[key] === right[key]);
  }

  function removeOwned(path, dialog) {
    try {
      const value = privateJson(path, MAX_QUESTION_BYTES);
      if (sameScope(dialog.scope, value) && value.dialog_id === dialog.id) unlinkSync(path);
    } catch {}
  }

  function disableQuestion(dialog = current) {
    if (!dialog) return;
    clearInterval(dialog.timer);
    removeOwned(questionPath, dialog);
    removeOwned(submissionPath, dialog);
    if (current === dialog) current = undefined;
  }

  function clearQuestion() {
    preparedQuestions.clear();
    executingQuestions.clear();
    disableQuestion();
  }

  function acknowledge(dialog, request, accepted, error) {
    atomicJson(ackPath, {
      ...dialog.scope,
      dialog_id: dialog.id,
      tool_call_id: dialog.toolCallId,
      nonce: request.nonce,
      request_id: request.request_id,
      accepted,
      ...(error ? { error } : {}),
    }, MAX_ACK_BYTES);
    clearTimeout(ackCleanup);
    ackCleanup = setTimeout(() => {
      try {
        const ack = privateJson(ackPath, MAX_ACK_BYTES);
        if (ack.nonce === request.nonce && sameScope(dialog.scope, ack)) unlinkSync(ackPath);
      } catch {}
    }, 5000);
    ackCleanup.unref?.();
  }

  function consumeAnswer(dialog) {
    if (current !== dialog || dialog.done) return;
    try {
      if (!sameScope(dialog.scope, identity(dialog.ctx)) || dialog.signal?.aborted) {
        disableQuestion(dialog);
        return;
      }
      let request;
      try {
        request = privateJson(submissionPath, MAX_QUESTION_BYTES);
      } catch (error) {
        if (error.code === "ENOENT") return;
        // A broken transport must not take the native picker down with it.
        try { unlinkSync(submissionPath); } catch {}
        disableQuestion(dialog);
        return;
      }
      unlinkSync(submissionPath);
      if (!request || typeof request.nonce !== "string" || !UUID.test(request.nonce) ||
          typeof request.request_id !== "string" || request.request_id.length === 0 ||
          Buffer.byteLength(request.request_id) > 1024) return;
      const error = !sameScope(dialog.scope, request) || request.dialog_id !== dialog.id ||
        request.tool_call_id !== dialog.toolCallId
        ? "This OMP question is no longer current"
        : answerError(dialog.questions, request.answers, request.cancel);
      if (error) {
        acknowledge(dialog, request, false, error);
        return;
      }
      // Commit the ack before dismissing the picker: failed IO leaves the
      // terminal usable. No await or user callback separates this write from
      // settling the already-validated wrapper promise.
      acknowledge(dialog, request, true);
      dialog.finish(request.cancel ? undefined : {
        kind: "submit",
        results: request.answers.map(answer => ({
          id: answer.id,
          selectedOptions: answer.selectedOptions,
          ...(answer.customInput !== undefined ? { customInput: answer.customInput } : {}),
        })),
      });
      // This abort belongs ONLY to the original presentation, never the tool.
      dialog.dismiss.abort();
    } catch {
      disableQuestion(dialog);
    }
  }

  function sameQuestions(left, right) {
    return validQuestions(left) && left.length === right.length &&
      left.every((question, index) => {
        const expected = right[index];
        return question.id === expected.id && question.question === expected.question &&
          (question.multi === true) === (expected.multi === true) &&
          question.options.length === expected.options.length &&
          question.options.every((option, at) => option.label === expected.options[at].label);
      });
  }

  function bindExecutingQuestion() {
    const dialog = current;
    if (!dialog || dialog.done) return;
    // Native ask is exclusive. Unexpected overlapping executions are never
    // disambiguated by equal question text or by whichever preflight was last.
    if (executingQuestions.size > 1) {
      disableQuestion(dialog);
      return;
    }
    if (executingQuestions.size !== 1 || dialog.toolCallId) return;
    const [toolCallId, call] = executingQuestions.entries().next().value;
    if (!call || !sameScope(dialog.scope, call.scope) ||
        !sameQuestions(dialog.questions, call.questions)) {
      disableQuestion(dialog);
      return;
    }
    dialog.toolCallId = toolCallId;
    dialog.ctx = call.ctx;
    try {
      atomicJson(questionPath, {
        ...dialog.scope, tool_call_id: toolCallId, dialog_id: dialog.id, questions: dialog.questions,
      }, MAX_QUESTION_BYTES);
      dialog.timer = setInterval(() => consumeAnswer(dialog), 25);
      dialog.timer.unref?.();
    } catch {
      disableQuestion(dialog);
    }
  }

  function installQuestionBridge(ctx) {
    if (ctx.agent.kind !== "main" || ctx.mode !== "tui" || !ctx.hasUI || !ctx.ui) return;
    // Event contexts proxy reads to a handler-scoped dialog that carries the
    // hook timeout signal. The own descriptor instead identifies the shared
    // native TUI capability without retaining that short-lived signal.
    const original = Object.getOwnPropertyDescriptor(ctx.ui, "askDialog")?.value;
    if (typeof original !== "function" || wrapped.has(original)) return;
    function askDialog(questions, options = {}) {
      // Preflight provides scope/validation only, never execution identity:
      // OMP can prepare several calls before the first native dialog opens.
      let prepared;
      for (const call of preparedQuestions.values()) {
        if (sameQuestions(questions, call.questions)) {
          prepared = call;
          break;
        }
      }
      let scope;
      try { scope = identity(prepared?.ctx ?? ctx); } catch {}
      if (!prepared || !sameScope(prepared.scope, scope) || current || options.signal?.aborted) {
        return original.call(this, questions, options);
      }
      const dismiss = new AbortController();
      const signal = options.signal ? AbortSignal.any([options.signal, dismiss.signal]) : dismiss.signal;
      const native = original.call(this, questions, { ...options, signal });
      if (!native || typeof native.then !== "function") return native;
      const dialog = {
        scope, ctx: prepared.ctx, id: randomUUID(), toolCallId: undefined,
        questions, dismiss, signal: options.signal, done: false, timer: undefined,
      };
      const promise = new Promise((resolve, reject) => {
        dialog.finish = value => {
          if (dialog.done) return;
          dialog.done = true;
          disableQuestion(dialog);
          resolve(value);
        };
        // Attach directly: Promise.resolve adds an assimilation turn for a
        // cross-realm promise and can let a queued web answer beat the terminal.
        native.then(dialog.finish, error => {
          if (dialog.done) return;
          dialog.done = true;
          disableQuestion(dialog);
          reject(error);
        });
      });
      current = dialog;
      // Depending on provider version, execution_start can precede or follow
      // entry into askDialog. Publish only once both observations agree.
      bindExecutingQuestion();
      return promise;
    }
    try {
      ctx.ui.askDialog = askDialog;
      wrapped.add(askDialog);
    } catch {
      // Read-only or absent native capabilities are terminal-only.
    }
  }

  omp.on("tool_call", (event, ctx) => {
    if (ctx.agent.kind !== "main" || ctx.mode !== "tui" || !ctx.hasUI || event.toolName !== "ask") return;
    try {
      installQuestionBridge(ctx);
      const scope = identity(ctx);
      if (scope && typeof event.toolCallId === "string" && event.toolCallId.length > 0 &&
          validQuestions(event.input?.questions)) {
        preparedQuestions.set(event.toolCallId, {
          scope, ctx, questions: structuredClone(event.input.questions),
        });
      }
    } catch {}
  });

  function privateFile(path, directory = false) {
    const stat = lstatSync(path);
    return (directory ? stat.isDirectory() : stat.isFile()) &&
      stat.uid === process.geteuid() && (stat.mode & 0o077) === 0;
  }

  function processStart() {
    const stat = readFileSync(`/proc/${process.pid}/stat`, "utf8");
    const fields = stat.slice(stat.lastIndexOf(") ") + 2).trim().split(/\s+/);
    if (Number(fields[2]) !== process.pid) return undefined;
    return fields[19];
  }

  function identity(ctx) {
    // Factories are rebound even to in-process subagents; depth is NOT a root
    // discriminator (/tan clones are subagents at depth zero).
    if (ctx.agent.kind !== "main" || !privateFile(runtime, true) || !privateFile(launchPath)) return;
    const launch = privateJson(launchPath, MAX_ACK_BYTES);
    if (launch.launch_id !== launchId) return;
    const file = ctx.sessionManager.getSessionFile();
    if (!file || !isAbsolute(file)) return;
    const sessionDir = realpathSync(ctx.sessionManager.getSessionDir());
    if (sessionDir !== launch.session_dir || realpathSync(dirname(file)) !== sessionDir) return;
    // OMP allocates the path before it first creates the transcript.
    try {
      if (!lstatSync(file).isFile()) return;
    } catch (error) {
      if (error.code !== "ENOENT") return;
    }
    const start = processStart();
    if (!start) return;
    return {
      launch_id: launchId,
      pid: process.pid,
      process_start: start,
      session_id: ctx.sessionManager.getSessionId(),
      transcript_path: join(sessionDir, basename(file)),
    };
  }

  function clear() {
    clearQuestion();
    clearTimeout(ackCleanup);
    try {
      const ack = privateJson(ackPath, MAX_ACK_BYTES);
      if (ack.launch_id === launchId && ack.pid === process.pid && ack.process_start === processStart()) unlinkSync(ackPath);
    } catch {}
    if (!owned) return;
    try {
      const report = privateJson(reportPath, MAX_ACK_BYTES);
      if (report.launch_id === launchId && report.pid === process.pid && report.process_start === processStart()) {
        unlinkSync(reportPath);
      }
    } catch {}
    owned = false;
    lastBody = undefined;
  }

  function publish(ctx, activity) {
    if (ctx.agent.kind !== "main") return;
    try {
      const scope = identity(ctx);
      if (!scope) {
        clear();
        return;
      }
      const body = JSON.stringify({ ...scope, activity });
      if (body === lastBody) return;
      writeFileSync(temporary, body, { mode: 0o600, flag: "wx" });
      renameSync(temporary, reportPath);
      lastBody = body;
      owned = true;
      if (!exitRegistered) {
        process.once("exit", clear);
        exitRegistered = true;
      }
    } catch {
      // Status reporting must never disturb tools or terminal interaction.
      try { unlinkSync(temporary); } catch {}
    }
  }

  function active(ctx) {
    revision++;
    settled = false;
    publish(ctx, asks.size ? "pending_answer" : "working");
  }

  // A settled tab is Idle, unless autopilot or one of OMP's own modes is
  // about to start the next turn by itself.
  function resting(ctx) {
    rested = "idle";
    try {
      if (ctx.agent.kind === "main" && autopilot?.keepsWorking() === true) rested = "working";
    } catch {}
    return rested;
  }

  function settleWhenIdle(ctx) {
    const expected = revision;
    const settle = () => {
      if (revision !== expected) return;
      if (!ctx.isIdle()) {
        ctx.setTimeout(settle, 25);
        return;
      }
      if (!ctx.hasPendingMessages()) {
        asks.clear();
        if (ctx.agent.kind === "main") clearQuestion();
        settled = true;
        try {
          if (ctx.agent.kind === "main") autopilot?.settled(ctx);
        } catch {}
        publish(ctx, resting(ctx));
      }
    };
    // agent_end may still be unwinding its own prompt. Never overwrite a
    // newer agent_start with that older terminal notification.
    ctx.setTimeout(settle, 0);
  }

  function reset(_event, ctx) {
    if (ctx.agent.kind === "main") {
      clearQuestion();
      try { installQuestionBridge(ctx); } catch {}
    }
    revision++;
    asks.clear();
    settled = ctx.isIdle() && !ctx.hasPendingMessages();
    publish(ctx, settled ? "idle" : "working");
  }

  for (const event of ["session_start", "session_switch", "session_branch", "session_tree"]) {
    omp.on(event, reset);
  }
  omp.on("agent_start", (_event, ctx) => {
    if (ctx.agent.kind === "main") clearQuestion();
    asks.clear();
    active(ctx);
  });
  omp.on("tool_execution_start", (event, ctx) => {
    if (event.toolName === "ask") asks.add(event.toolCallId);
    if (ctx.agent.kind === "main" && event.toolName === "ask") {
      executingQuestions.set(event.toolCallId, preparedQuestions.get(event.toolCallId));
      bindExecutingQuestion();
    }
    active(ctx);
  });
  omp.on("tool_execution_end", (event, ctx) => {
    asks.delete(event.toolCallId);
    if (ctx.agent.kind === "main") {
      preparedQuestions.delete(event.toolCallId);
      executingQuestions.delete(event.toolCallId);
      if (current?.toolCallId === event.toolCallId) disableQuestion();
    }
    active(ctx);
  });
  omp.on("agent_end", (event, ctx) => {
    if (ctx.agent.kind === "main") clearQuestion();
    asks.clear();
    active(ctx);
    if (!event.willContinue) settleWhenIdle(ctx);
  });
  omp.on("auto_retry_start", (_event, ctx) => active(ctx));
  omp.on("auto_compaction_start", (_event, ctx) => {
    idleCompactionRevision = settled ? revision + 1 : undefined;
    active(ctx);
  });
  omp.on("auto_compaction_end", (event, ctx) => {
    if (event.willRetry) active(ctx);
    else if (idleCompactionRevision === revision) settleWhenIdle(ctx);
    // During an agent run, only agent_end can rule out other continuations.
    idleCompactionRevision = undefined;
  });
  omp.on("auto_retry_end", (event, ctx) => {
    if (event.success) return;
    // A scheduled retry can fail locally before agent_start, so no subsequent
    // agent_end is guaranteed. Wait for that failed prompt to unwind; any new
    // lifecycle event invalidates this check rather than overriding its state.
    settleWhenIdle(ctx);
  });
  omp.on("session_shutdown", (_event, ctx) => {
    if (ctx.agent.kind !== "main") return;
    revision++;
    clear();
    process.removeListener("exit", clear);
    exitRegistered = false;
  });

  // Autopilot is kept apart from everything above: its setup and every one of
  // its callbacks are guarded, so a fault in it cannot stop activity
  // reporting or the question bridge.
  function installAutopilot() {
    const controllerPath = join(runtime, "apas-controller.json");
    const requestPath = join(runtime, "omp-autopilot-request.json");
    const takenPath = join(runtime, `omp-autopilot-request.taken-${process.pid}`);
    const stopPath = join(runtime, "omp-autopilot-stop.json");
    const resultsPath = join(runtime, "omp-autopilot-ack.json");
    const statePath = join(runtime, "omp-autopilot.json");
    // Results by request id, so a repeated request has no further effect.
    const applied = new Map();
    const results = [];
    let resultsDirty = false;
    const beat = { mark: undefined, seenAt: undefined, last: undefined };
    let root; // the main agent's context: managed timers and OMP calls
    let support; // unknown until OMP's version has been read
    let unsupported;
    let processStarted;
    let ap; // the current autopilot, on or paused
    let ended; // the last ended one, as it is reported
    let endedByChange = false;
    let endedTurnStarted = false;
    let sent; // a submission OMP has not recorded yet
    let running = false; // between agent_start and agent_end
    let ownAbort = false; // this extension aborted, and what that ends has not been seen yet
    let lastTurn;
    let awaiting = false; // the last run ended waiting for background work
    let awaitOverAt;
    let settledAt = 0;
    let goal; // what only goal_updated tells
    let modeGapUntil;
    let counter = 0;
    let written;
    let writtenAt = 0;
    let status;
    let dueTimer;
    let deadlineTimer;

    const wall = () => Date.now();
    // Durations are measured on this clock, so a step of the wall clock can
    // neither keep a dead controller's heartbeat fresh nor cut a hold short.
    const elapsed = () => (typeof performance === "object" ? performance.now() : Date.now());
    const iso = ms => new Date(ms).toISOString();
    const guard = fn => (...args) => {
      try {
        fn(...args);
      } catch {}
    };
    const live = ctx => root !== undefined && ctx?.agent?.kind === "main";
    const switchedOn = mode => mode !== undefined && mode !== null && mode.status !== "inactive";

    // Which launch these files belong to. They are private to the OS account
    // and are not an authority boundary.
    function scope() {
      try {
        if (!privateFile(runtime, true) || !privateFile(launchPath)) return;
        if (privateJson(launchPath, MAX_ACK_BYTES).launch_id !== launchId) return;
        processStarted ??= processStart();
        if (processStarted) return { launch_id: launchId, pid: process.pid, process_start: processStarted };
      } catch {}
    }

    function cancel(timer) {
      if (timer === undefined) return;
      try {
        root?.clearTimer(timer);
      } catch {}
    }

    // An abort gives the same marker, cancelled retry and cancelled compaction
    // whoever asked for it. The flag keeps this extension's own from being
    // taken for a person's interrupt, also by an autopilot started right after.
    function abortOwn(ctx) {
      ownAbort = true;
      try {
        ctx.abort();
      } catch {}
    }

    // The journal travels with the conversation. An entry that cannot be
    // written only means the autopilot is gone after a restart.
    function record(ctx, id, kind, fields) {
      try {
        omp.appendEntry(AUTOPILOT_JOURNAL, {
          v: AUTOPILOT_FORMAT, autopilot_id: id, session_id: ctx.sessionManager.getSessionId(),
          kind, at: iso(wall()), ...fields,
        });
        return true;
      } catch {
        return false;
      }
    }

    // The latest autopilot the active branch shows for this session. OMP
    // appends custom entries of its own, and a fork copies another session's.
    // It is the one that started last: a prompt OMP drops late adds an entry
    // for an autopilot that was stopped before the next one started.
    function journaled(ctx) {
      const session = ctx.sessionManager.getSessionId();
      const branch = ctx.sessionManager.getBranch();
      const own = entry => entry?.type === "custom" && entry.customType === AUTOPILOT_JOURNAL &&
        entry.data?.session_id === session;
      let id;
      for (let at = branch.length - 1; at >= 0 && id === undefined; at--) {
        if (own(branch[at]) && branch[at].data.kind === "start") id = branch[at].data.autopilot_id;
      }
      if (typeof id !== "string" || !UUID.test(id)) return;
      let found;
      for (const entry of branch) {
        if (found?.endedAt !== undefined && entry?.type === "message" && entry.message?.role === "assistant") {
          found.turnSince = true;
        }
        if (!own(entry) || entry.data.autopilot_id !== id) continue;
        const data = entry.data;
        if (data.kind === "start") {
          const startedMs = Date.parse(data.started_at);
          if (autopilotSpecRefusal(data.spec) || !Number.isFinite(startedMs)) return;
          let deadlineMs;
          if (data.spec.time_limit.kind === "at_most") {
            deadlineMs = Date.parse(data.deadline);
            if (!Number.isFinite(deadlineMs)) deadlineMs = startedMs + data.spec.time_limit.value * 1000;
          }
          found = {
            id, spec: JSON.parse(JSON.stringify(data.spec)), startedMs, deadlineMs, iterations: 0,
            lastSent: undefined, paused: false, reason: undefined, endedAt: undefined, turnSince: false,
          };
        } else if (!found) {
          continue;
        } else if (data.kind === "iteration" && wholeNumber(data.iteration)) {
          found.iterations = Math.max(found.iterations, data.iteration);
          const at = Date.parse(data.at);
          if (Number.isFinite(at)) found.lastSent = at;
        } else if (data.kind === "not_submitted") {
          // It cancels the iteration entry it follows.
          if (found.iterations === data.iteration) found.iterations = data.iteration - 1;
        } else if (data.kind === "pause" || data.kind === "resume" || data.kind === "end") {
          if (wholeNumber(data.iterations)) found.iterations = data.iterations;
          found.paused = data.kind === "pause";
          if (data.kind === "end") {
            const at = Date.parse(data.at);
            found.endedAt = Number.isFinite(at) ? at : wall();
            found.reason = typeof data.reason === "string" ? data.reason : "stopped";
          }
        }
      }
      return found;
    }

    function endedView(found) {
      return {
        autopilot_id: found.id, spec: found.spec, started_at: iso(found.startedMs), iterations: found.iterations,
        reason: found.reason, ended_at: iso(found.endedAt), turn_since: found.turnSince,
      };
    }

    function autopilotOf(id, spec, startedMs, deadlineMs) {
      return {
        id, spec, startedMs, deadlineMs, iterations: 0, phase: "on", reason: undefined, stopping: false,
        iter: undefined, failures: 0, notBefore: 0, lastSent: undefined, due: undefined,
        hold: undefined, holdSince: undefined, idleSince: undefined, cond: undefined, since: undefined,
      };
    }

    // session_start fires once per process. An autopilot never resumes by
    // itself after its agent was restarted: it comes back paused, or ended
    // when its time limit passed meanwhile.
    function restore(ctx) {
      const found = journaled(ctx);
      if (!found) return;
      if (found.endedAt === undefined && found.deadlineMs !== undefined && wall() >= found.deadlineMs) {
        found.endedAt = found.deadlineMs;
        found.reason = "time_limit";
        record(ctx, found.id, "end", { iterations: found.iterations, reason: found.reason, at: iso(found.endedAt) });
      }
      if (found.endedAt !== undefined) {
        ended = endedView(found);
        return;
      }
      ap = autopilotOf(found.id, found.spec, found.startedMs, found.deadlineMs);
      ap.iterations = found.iterations;
      ap.lastSent = found.lastSent;
      ap.phase = "paused";
      ap.reason = "agent_restarted";
      if (!found.paused) record(ctx, ap.id, "pause", { iterations: ap.iterations, reason: ap.reason });
      armDeadline(ctx);
    }

    function end(ctx, reason, journal = true) {
      const done = ap;
      if (!done) return;
      ap = undefined;
      cancel(dueTimer);
      cancel(deadlineTimer);
      dueTimer = deadlineTimer = undefined;
      // A time limit ends autopilot at the limit, also when its timer ran late.
      const at = iso(reason === "time_limit" && done.deadlineMs !== undefined ? Math.min(wall(), done.deadlineMs) : wall());
      if (journal) record(ctx, done.id, "end", { iterations: done.iterations, reason, at });
      ended = {
        autopilot_id: done.id, spec: done.spec, started_at: iso(done.startedMs), iterations: done.iterations,
        reason, ended_at: at, turn_since: false,
      };
      endedByChange = reason === "conversation_changed";
      endedTurnStarted = false;
    }

    function pause(ctx, reason) {
      if (!ap || ap.phase !== "on" || ap.stopping) return;
      ap.phase = "paused";
      ap.reason = reason;
      ap.due = ap.hold = ap.holdSince = undefined;
      cancel(dueTimer);
      dueTimer = undefined;
      record(ctx, ap.id, "pause", { iterations: ap.iterations, reason });
    }

    // The time limit has its own timer, so a turn or a tab that never settles
    // cannot keep a tab on autopilot past it. A turn in progress is not
    // interrupted; it finishes as an ordinary turn of the tab.
    function armDeadline(ctx) {
      cancel(deadlineTimer);
      deadlineTimer = undefined;
      if (ap?.deadlineMs === undefined) return;
      const left = ap.deadlineMs - wall();
      if (left <= 0) {
        end(ctx, ap.stopping ? "stopped" : "time_limit");
        return;
      }
      deadlineTimer = ctx.setTimeout(guard(() => {
        deadlineTimer = undefined;
        if (!root) return;
        armDeadline(root);
        sync();
      }), Math.min(left, AUTOPILOT_TIMER_MAX_MS));
    }

    // OMP's loop suppresses its own goal continuation, and plan mode
    // continues by itself: two drivers on one tab cannot both be right.
    function otherDriver(ctx) {
      const state = ctx.sessionModes.getState();
      if (switchedOn(state.goal)) return "omp_goal_mode";
      if (switchedOn(state.loop)) return "omp_loop_mode";
      if (switchedOn(state.plan)) return "omp_plan_mode";
      // ctx.sessionModes does not report autoresearch; its experiment tool does.
      if (omp.getActiveTools().includes("init_experiment")) return "omp_autoresearch";
    }

    function continuing(ctx) {
      const state = ctx.sessionModes.getState();
      return state.goal?.status === "active" || state.loop?.status === "active";
    }

    function modesView(ctx) {
      const state = ctx.sessionModes.getState();
      const view = {};
      if (switchedOn(state.goal)) {
        const budget = state.goal.tokenBudget ?? goal?.tokenBudget;
        const objective = cutText(state.goal.objective);
        const goalStatus = cutText(goal?.status);
        view.goal = {
          status: String(state.goal.status),
          ...(objective !== undefined ? { objective } : {}),
          ...(wholeNumber(budget) ? { token_budget: budget } : {}),
          ...(wholeNumber(goal?.tokensUsed) ? { tokens_used: goal.tokensUsed } : {}),
          ...(goalStatus !== undefined ? { goal_status: goalStatus } : {}),
        };
      }
      if (switchedOn(state.loop)) view.loop = { status: String(state.loop.status) };
      return view.goal || view.loop ? view : undefined;
    }

    function readHeartbeat() {
      let value;
      try {
        value = privateJson(controllerPath, AUTOPILOT_MAX_BYTES);
      } catch {
        return;
      }
      if (!value || value.launch_id !== launchId || typeof value.writer !== "string") return;
      // Compared for difference, never for order: a replaced CLI process
      // writes a new id and starts its counter again.
      const mark = `${value.writer}:${value.counter}`;
      if (mark !== beat.mark) {
        beat.mark = mark;
        beat.seenAt = elapsed();
      }
      beat.last = value;
    }

    // Why nothing may be submitted now: no controller, or a server that
    // cannot show or stop this tab.
    function holdReason() {
      if (beat.seenAt === undefined || elapsed() - beat.seenAt > AUTOPILOT_HEARTBEAT_FRESH_MS) return "controller";
      if (beat.last.connected !== true) {
        // A heartbeat that does not say for how long cannot show a short outage.
        return beat.last.unreachable_secs <= AUTOPILOT_SERVER_UNREACHABLE_SECS ? undefined : "server";
      }
      return beat.last.server_controls_autopilot === true ? undefined : "server";
    }

    function keep(result) {
      results.push(result);
      if (results.length > AUTOPILOT_ACK_KEEP) results.shift();
      resultsDirty = true;
    }

    function answer(id, refusal) {
      const result = {
        request_id: id, outcome: refusal ? "refused" : "accepted", ...(refusal ? { reason: refusal } : {}),
        at_ms: wall(),
      };
      applied.set(id, result);
      if (applied.size > 256) applied.delete(applied.keys().next().value);
      keep(result);
    }

    function writeResults(identity) {
      if (!resultsDirty) return;
      try {
        atomicJson(resultsPath, { v: AUTOPILOT_FORMAT, ...identity, acks: results }, AUTOPILOT_MAX_BYTES);
        resultsDirty = false;
      } catch {
        // Written again at the next read; the loop does not wait for it.
      }
    }

    function addressed(request) {
      return request !== null && typeof request === "object" && request.launch_id === launchId &&
        typeof request.request_id === "string" && request.request_id.length > 0 &&
        Buffer.byteLength(request.request_id) <= 1024;
    }

    function takeRequests(ctx) {
      // Until OMP's version is known the journaled autopilot is not restored
      // yet. A stop applied now would be acknowledged and miss the one that
      // comes back paused a moment later, so it stays pending until then.
      if (support === undefined) return;
      const pending = [];
      // A start or resume is never left pending. It is taken by renaming:
      // when its wait ends the CLI removes the file, and a removal that
      // succeeds tells it the tab never took the request.
      let taken;
      try {
        renameSync(requestPath, takenPath);
        try {
          taken = privateJson(takenPath, AUTOPILOT_MAX_BYTES);
        } finally {
          unlinkSync(takenPath);
        }
      } catch {}
      if (addressed(taken)) pending.push({ request: taken, starts: true });
      // A pause or stop stays in its file, which only the CLI removes, until
      // its id is acknowledged. It is applied whatever its format version
      // and unknown fields: a stop must not be refused for a newer writer.
      try {
        const request = privateJson(stopPath, AUTOPILOT_MAX_BYTES);
        if (addressed(request)) pending.push({ request, starts: false });
      } catch {}
      const issued = entry => (Number.isFinite(entry.request.issued_at_ms) ? entry.request.issued_at_ms : 0);
      pending.sort((left, right) => issued(left) - issued(right));
      // Read after the files: a request written since this step began must
      // not look as if it came from the future.
      const now = wall();
      for (const { request, starts } of pending) {
        const known = applied.get(request.request_id);
        if (known) {
          // Kept as the newest: a request still in its file is never
          // forgotten, however many others are answered meanwhile.
          applied.delete(request.request_id);
          applied.set(request.request_id, known);
          if (!results.includes(known)) keep(known);
        } else if (starts) {
          answer(request.request_id, startOrResume(ctx, request, now));
        } else {
          // One that cannot be applied now stays pending and is tried again.
          try {
            answer(request.request_id, pauseOrStop(ctx, request));
          } catch {}
        }
      }
    }

    function startOrResume(ctx, request, now) {
      // Strict, unlike a stop: nothing starts from a request this build only
      // half understands.
      if (request.v !== AUTOPILOT_FORMAT) return "This tab's APAS does not know this request's format.";
      if (!onlyKeys(request, ["v", "launch_id", "request_id", "issued_at_ms", "command"]) ||
          !Number.isFinite(request.issued_at_ms)) return AUTOPILOT_UNREADABLE;
      // Older than the CLI's wait: its sender was told that nothing started.
      const age = now - request.issued_at_ms;
      if (age < 0 || age > AUTOPILOT_REQUEST_MAX_AGE_MS) return AUTOPILOT_NOT_TAKEN;
      if (support !== "supported") return AUTOPILOT_UNSUPPORTED;
      const command = request.command;
      try {
        if (onlyKeys(command, ["kind", "spec"]) && command.kind === "start") return begin(ctx, command.spec, now);
        if (onlyKeys(command, ["kind", "autopilot_id"]) && command.kind === "resume") {
          return resume(ctx, command.autopilot_id, now);
        }
      } catch {
        return "This tab could not apply the request.";
      }
      return AUTOPILOT_UNREADABLE;
    }

    function begin(ctx, spec, now) {
      const refusal = autopilotSpecRefusal(spec);
      if (refusal) return refusal;
      if (ap) return "This tab is already on autopilot. Stop it first.";
      const driver = otherDriver(ctx);
      if (driver) return AUTOPILOT_MODE_REFUSALS[driver];
      const deadlineMs = spec.time_limit.kind === "at_most" ? now + spec.time_limit.value * 1000 : undefined;
      const fresh = autopilotOf(randomUUID(), spec, now, deadlineMs);
      // Only this entry holds the prompt.
      record(ctx, fresh.id, "start", {
        spec, started_at: iso(now), ...(deadlineMs !== undefined ? { deadline: iso(deadlineMs) } : {}),
      });
      ap = fresh;
      ended = undefined;
      endedByChange = false;
      try {
        armDeadline(ctx);
      } catch {
        // The poll checks the time limit as well.
      }
    }

    function resume(ctx, id, now) {
      if (!ap || ap.phase !== "paused" || ap.stopping || typeof id !== "string" || id.toLowerCase() !== ap.id) {
        return "This autopilot is no longer current.";
      }
      if (ap.deadlineMs !== undefined && now >= ap.deadlineMs) {
        end(ctx, "time_limit");
        return "This autopilot is no longer current.";
      }
      const driver = otherDriver(ctx);
      if (driver) return AUTOPILOT_MODE_REFUSALS[driver];
      ap.phase = "on";
      ap.reason = undefined;
      // A resume starts the failure count again.
      ap.failures = 0;
      ap.notBefore = 0;
      record(ctx, ap.id, "resume", { iterations: ap.iterations });
    }

    // A request that changes nothing is accepted. The autopilot id it may
    // carry is a hint: it applies to whatever autopilot is current.
    function pauseOrStop(ctx, request) {
      if (request.action === "pause") pause(ctx, "requested");
      else if (request.action === "stop") end(ctx, "stopped");
      else if (request.action === "stop_now") stopNow(ctx);
      else return "This tab's APAS does not know this request.";
    }

    // Stop now also interrupts the turn in progress, whoever started it. An
    // iteration OMP has accepted cannot be recalled and counts as that turn:
    // autopilot is reported as stopped when its run has ended, and that run
    // is aborted too if it starts later. While the stop waits, this
    // extension's own abort is not taken for a person's interrupt.
    function stopNow(ctx) {
      if (!ap) return;
      if (ap.stopping || (!running && sent === undefined)) {
        if (!ap.stopping && !settled) abortOwn(ctx);
        end(ctx, "stopped");
        return;
      }
      ap.stopping = true;
      cancel(dueTimer);
      dueTimer = undefined;
      abortOwn(ctx);
    }

    function nextDue() {
      // The first iteration is due as soon as the tab has settled.
      if (ap.lastSent === undefined) return Math.max(settledAt, ap.notBefore);
      let due = settledAt + AUTOPILOT_SETTLE_MS;
      // A timed repeat runs from submission to submission, so a person's
      // message does not move the schedule.
      if (ap.spec.repeat.kind === "every") due = Math.max(due, ap.lastSent + ap.spec.repeat.seconds * 1000);
      return Math.max(due, ap.notBefore);
    }

    function schedule(ctx, due, now, inTimer) {
      if (due === ap.due) return;
      ap.due = due;
      cancel(dueTimer);
      dueTimer = undefined;
      // The step that submits runs from a managed timer, never from an event
      // handler, which has a 30-second budget.
      if (due > now || !inTimer) {
        dueTimer = ctx.setTimeout(guard(() => {
          dueTimer = undefined;
          tick();
        }), Math.min(Math.max(0, due - now), AUTOPILOT_TIMER_MAX_MS));
      }
    }

    function failed(ctx, now) {
      ap.failures++;
      ap.notBefore = now + AUTOPILOT_FAILURE_WAIT_MS;
      if (ap.failures >= AUTOPILOT_FAILURE_PAUSE) pause(ctx, "failures");
    }

    function outcomeOf(iter) {
      const turn = iter.lastTurn;
      // Without any turn_end the turn did not complete.
      if (!turn || turn.stopReason === "error" || turn.stopReason === "aborted") return "failed";
      return "ok";
    }

    // The end of an iteration's turn. A rule that ends autopilot wins over
    // one that pauses it: a pause with nothing left to resume helps nobody.
    // A requested stop comes before all of these, at its callers.
    function finish(ctx, outcome, now, pauseReason = "interrupted") {
      const iter = ap.iter;
      ap.iter = ap.due = undefined;
      if (outcome === "failed") {
        ap.failures++;
        ap.notBefore = now + AUTOPILOT_FAILURE_WAIT_MS;
      } else if (outcome === "ok") {
        ap.failures = 0;
      }
      const limit = ap.spec.iteration_limit;
      let soonest = now + AUTOPILOT_SETTLE_MS;
      if (ap.spec.repeat.kind === "every") soonest = Math.max(soonest, ap.lastSent + ap.spec.repeat.seconds * 1000);
      soonest = Math.max(soonest, ap.notBefore);
      if (ap.deadlineMs !== undefined && now >= ap.deadlineMs) end(ctx, "time_limit");
      else if (limit.kind === "at_most" && ap.iterations >= limit.value) end(ctx, "iteration_limit");
      else if (ap.phase === "on" && ap.deadlineMs !== undefined && soonest > ap.deadlineMs) end(ctx, "time_limit_before_next");
      else if (outcome === "interrupted") pause(ctx, pauseReason);
      else if (outcome === "failed" && ap.failures >= AUTOPILOT_FAILURE_PAUSE) pause(ctx, "failures");
      // A heuristic: only for a turn that completed, and only when no other rule applies.
      else if (outcome === "ok" && ap.spec.end_without_tool_call && iter.tools === 0) end(ctx, "no_tool_call");
    }

    // OMP emits no event when a wait for background work ends without waking
    // the tab, so its own job state is read. Esc on the wait row cannot be
    // seen: the jobs stay listed until they end.
    function backgroundWait(ctx, now) {
      const jobs = ctx.getAsyncJobSnapshot();
      const delivery = jobs?.delivery;
      const over = !jobs || (!(jobs.running?.length > 0) && !(delivery?.queued > 0) &&
        delivery?.delivering !== true && !(delivery?.pendingJobIds?.length > 0));
      if (!over) {
        awaitOverAt = undefined;
        return;
      }
      awaitOverAt ??= elapsed();
      // Five seconds cover a result OMP has taken from its job list and not
      // yet injected. A run that starts meanwhile ends this wait.
      if (elapsed() - awaitOverAt < AUTOPILOT_SETTLE_MS) return;
      awaiting = false;
      awaitOverAt = undefined;
      if (ap.iter?.recorded) finish(ctx, "interrupted", now, "background_wait_ended");
      else pause(ctx, "background_wait_ended");
      // Nothing starts a turn for this wait any more, so the tab is idle.
      settleWhenIdle(ctx);
    }

    function submit(ctx, now) {
      const mine = { id: ap.id, n: ap.iterations + 1, prompt: ap.spec.prompt, calledAt: now, before: ap.lastSent };
      // Journaled just before the prompt is handed to OMP: the transcript
      // reader knows an iteration by this entry, never by its text.
      if (!record(ctx, mine.id, "iteration", { iteration: mine.n })) {
        failed(ctx, now);
        return;
      }
      ap.iterations = mine.n;
      ap.lastSent = now;
      // From here the tab is Working until the run starts, or for 15 seconds.
      ap.due = now;
      ap.iter = { n: mine.n, recorded: false, tools: 0, lastTurn: undefined };
      sent = mine;
      ownAbort = false;
      let call;
      try {
        // "aside" starts a turn on an idle tab. If a run began meanwhile, OMP
        // takes the prompt at its next step boundary; a steer would restart
        // the run right after an abort, and Stop now could not hold.
        call = omp.sendUserMessage(mine.prompt, { attribution: "agent", deliverAs: "aside" });
      } catch (error) {
        rejected(mine, error);
        return;
      }
      // Never awaited: on an idle tab the promise settles only when the whole
      // run has ended. It rejects only for a prompt OMP dropped.
      if (typeof call?.then === "function") call.then(undefined, guard(error => rejected(mine, error)));
    }

    function rejected(mine, error) {
      const ctx = root;
      // After OMP recorded the message the iteration is classified once, by
      // how its turn ends.
      if (!ctx || sent !== mine) return;
      sent = undefined;
      record(ctx, mine.id, "not_submitted", { iteration: mine.n });
      // OMP hands a dropped prompt back to the input line, where the
      // conversation composer types blind. Its editor stores a tab as three spaces.
      try {
        if (ctx.ui.getEditorText() === mine.prompt.replaceAll("\t", "   ")) ctx.ui.setEditorText("");
      } catch {}
      if (ap?.id === mine.id) {
        // A prompt that was not submitted uses up no iteration.
        if (ap.iterations === mine.n) {
          ap.iterations = mine.n - 1;
          ap.lastSent = mine.before;
        }
        if (ap.iter?.n === mine.n) ap.iter = undefined;
        if (ap.stopping) end(ctx, "stopped");
        // Every abort gives the same default text, whoever caused it. A
        // refusal reason, or any other error, is a failed iteration.
        else if (error?.name === "PromptDroppedError" && error.message === OMP_PROMPT_DROPPED) pause(ctx, "interrupted");
        else failed(ctx, wall());
      } else if (ended?.autopilot_id === mine.id && ended.iterations === mine.n) {
        // Nor does it count for an autopilot that was stopped meanwhile.
        ended.iterations = mine.n - 1;
      }
      sync();
    }

    function evaluate(ctx, now) {
      if (!ap) return;
      if (ap.deadlineMs !== undefined && now >= ap.deadlineMs) {
        end(ctx, ap.stopping ? "stopped" : "time_limit");
        return;
      }
      if (ap.stopping) {
        // The turn Stop now waits for can end without its end reaching this
        // extension, as after a /compact typed during it. OMP idle for five
        // seconds, with nothing queued or outstanding, has no turn left.
        if (sent !== undefined || !ctx.isIdle() || ctx.hasPendingMessages()) ap.idleSince = undefined;
        else if (elapsed() - (ap.idleSince ??= elapsed()) >= AUTOPILOT_SETTLE_MS) end(ctx, "stopped");
        return;
      }
      if (awaiting) backgroundWait(ctx, now);
      if (ap?.phase !== "on") return;
      const driver = otherDriver(ctx);
      if (driver) {
        pause(ctx, driver);
        return;
      }
      // A hold lasts from the moment one of its conditions is true until none
      // is, whether or not an iteration is due or a turn is in progress.
      const hold = holdReason();
      if (hold) {
        ap.holdSince ??= elapsed();
        ap.hold = hold;
        if (elapsed() - ap.holdSince >= AUTOPILOT_HOLD_PAUSE_MS) {
          pause(ctx, "no_control");
          return;
        }
      } else {
        ap.hold = ap.holdSince = undefined;
      }
      if (awaiting || !settled || running || sent !== undefined || asks.size > 0 ||
          !ctx.isIdle() || ctx.hasPendingMessages()) return;
      // The tab has settled, yet the end of the last iteration's turn never
      // reached this extension, as when a retry fails before its run starts.
      // It is classified now, so that the failure rules still count it.
      if (ap.iter) {
        finish(ctx, outcomeOf(ap.iter), now);
        return;
      }
      const due = nextDue();
      if (ap.deadlineMs !== undefined && due > ap.deadlineMs) {
        end(ctx, "time_limit_before_next");
        return;
      }
      schedule(ctx, due, now, true);
      if (hold || now < due) return;
      const limit = ap.spec.iteration_limit;
      if (limit.kind === "at_most" && ap.iterations >= limit.value) {
        end(ctx, "iteration_limit");
        return;
      }
      submit(ctx, now);
    }

    // One synchronous step from a managed timer: the request files and the
    // heartbeat are read in the same step as every submission, which is what
    // makes a pending stop reach the next iteration in time.
    function tick() {
      const ctx = root;
      if (!ctx) return;
      try {
        // Without its launch record this provider reads, writes and submits
        // nothing: the tab's files belong to the one that replaced it.
        const identity = scope();
        if (!identity) return;
        readHeartbeat();
        try {
          takeRequests(ctx);
        } finally {
          writeResults(identity);
        }
        if (support === "supported") evaluate(ctx, wall());
      } finally {
        sync();
      }
    }

    function conditionOf() {
      if (ap.phase === "paused") return ["paused", ap.reason];
      if (ap.stopping) return ["running"];
      if (!settled || running || sent !== undefined) {
        if (awaiting) return ["waiting", "background_work"];
        if (ap.iter || sent?.id === ap.id) return ["running"];
        // A turn the person started, or one that was in progress at the start.
        return ap.iterations === 0 ? ["starting"] : ["waiting", "settle"];
      }
      if (ap.hold) return ["holding", ap.hold];
      return ap.iterations === 0 ? ["starting"] : ["waiting", "next_iteration"];
    }

    function viewOf(now) {
      const [kind, detail] = conditionOf();
      const key = `${kind}:${detail ?? ""}`;
      if (ap.cond !== key) {
        ap.cond = key;
        ap.since = iso(now);
      }
      const timed = kind === "waiting" && detail === "next_iteration" && ap.due !== undefined;
      return {
        autopilot_id: ap.id, spec: ap.spec, started_at: iso(ap.startedMs), iterations: ap.iterations,
        ...(ap.deadlineMs !== undefined ? { deadline: iso(ap.deadlineMs) } : {}),
        ...(timed ? { next_iteration_at: iso(ap.due) } : {}),
        condition: { kind, ...(detail ? { detail } : {}), since: ap.since },
      };
    }

    function statusText(view) {
      if (!view) {
        return ended && !ended.turn_since ? `Autopilot · ${AUTOPILOT_ENDED_TEXT[ended.reason] ?? "Ended"}` : undefined;
      }
      const { kind, detail } = view.condition;
      if (kind === "paused") return `Autopilot · ${AUTOPILOT_PAUSED_TEXT[detail] ?? "Paused"}`;
      if (kind === "starting") return "Autopilot · starting";
      const limit = view.spec.iteration_limit;
      const progress = `Autopilot · iteration ${view.iterations}${limit.kind === "at_most" ? ` of ${limit.value}` : ""}`;
      if (kind === "holding") {
        return `${progress} · holding until ${detail === "server" ? "the APAS server" : "APAS"} can control this tab`;
      }
      if (kind !== "waiting") return progress;
      if (detail === "background_work") return `${progress} · waiting for background work`;
      if (detail === "settle") return `${progress} · waiting for the tab to settle`;
      if (!view.next_iteration_at) return progress;
      const next = new Date(ap.due);
      const two = value => String(value).padStart(2, "0");
      return `${progress} · next ${two(next.getHours())}:${two(next.getMinutes())}`;
    }

    // The only source for what the tab is doing. Rewritten on every change
    // and at least every 30 seconds, with a counter that tells the CLI a
    // refresh from a stale file without comparing clocks.
    function report(ctx, view) {
      if (support === undefined) return;
      let modes;
      try {
        if (support === "supported") modes = modesView(ctx);
      } catch {}
      const body = {
        session_id: String(ctx.sessionManager.getSessionId() ?? ""),
        support,
        ...(unsupported ? { unsupported_reason: unsupported } : {}),
        ...(view ? { autopilot: view } : {}),
        ...(ended ? { last_ended: ended } : {}),
        ...(modes ? { omp_modes: modes } : {}),
      };
      const text = JSON.stringify(body);
      const now = elapsed();
      if (text === written && now - writtenAt < AUTOPILOT_REPORT_MS) return;
      const identity = scope();
      if (!identity) return;
      atomicJson(statePath, { v: AUTOPILOT_FORMAT, counter: ++counter, ...identity, ...body }, AUTOPILOT_MAX_BYTES);
      written = text;
      writtenAt = now;
    }

    function sync() {
      const ctx = root;
      if (!ctx) return;
      // An iteration or a mode's continuation coming due, or failing to
      // start, changes what a settled tab shows.
      try {
        if (settled && (keepsWorking() ? "working" : "idle") !== rested) publish(ctx, resting(ctx));
      } catch {}
      // A state that cannot be worked out is left to go stale, which reads
      // as not reported. It is never written as "no autopilot".
      const view = ap ? viewOf(wall()) : undefined;
      try {
        const text = support === "supported" ? statusText(view) : undefined;
        if (text !== status) {
          ctx.ui.setStatus(AUTOPILOT_STATUS_KEY, text);
          status = text;
        }
      } catch {}
      report(ctx, view);
    }

    // Working between iterations while the next one is at most ten seconds
    // away, and across the gap before a built-in mode continues; Idle again
    // when no run has started 15 seconds after it was due.
    function keepsWorking() {
      if (!root) return false;
      const now = wall();
      if (ap?.phase === "on" && !ap.stopping && ap.hold === undefined && ap.due !== undefined &&
          now >= ap.due - AUTOPILOT_WORKING_AHEAD_MS && now <= ap.due + AUTOPILOT_WORKING_LATE_MS) return true;
      return modeGapUntil !== undefined && elapsed() <= modeGapUntil;
    }

    // Called when the tab has settled: no run, retry or automatic compaction
    // in progress, nothing queued, no open question.
    function tabSettled(ctx) {
      if (!root) return;
      settledAt = wall();
      ownAbort = false;
      if (ap?.phase === "on" && !ap.stopping && sent === undefined && !awaiting) {
        schedule(ctx, nextDue(), settledAt, false);
      }
      // A built-in mode submits again about 0.8 s after its turn, with no event.
      modeGapUntil = undefined;
      try {
        if (support === "supported" && continuing(ctx)) modeGapUntil = elapsed() + AUTOPILOT_WORKING_LATE_MS;
      } finally {
        sync();
      }
    }

    // Every call autopilot uses. ctx.sessionModes exists since 18.13.2, and
    // only in an interactive session.
    function missingCall(ctx) {
      const needs = [
        [omp, "sendUserMessage", "pi"], [omp, "appendEntry", "pi"], [omp, "getActiveTools", "pi"],
        [ctx, "isIdle", "ctx"], [ctx, "hasPendingMessages", "ctx"], [ctx, "abort", "ctx"],
        [ctx, "getAsyncJobSnapshot", "ctx"], [ctx, "setTimeout", "ctx"], [ctx, "clearTimer", "ctx"],
        [ctx.sessionModes, "getState", "ctx.sessionModes"], [ctx.sessionManager, "getBranch", "ctx.sessionManager"],
        [ctx.sessionManager, "getSessionId", "ctx.sessionManager"],
        [ctx.sessionManager, "getSessionFile", "ctx.sessionManager"],
        [ctx.ui, "setStatus", "ctx.ui"], [ctx.ui, "getEditorText", "ctx.ui"], [ctx.ui, "setEditorText", "ctx.ui"],
      ];
      const missing = needs.find(([owner, name]) => typeof owner?.[name] !== "function");
      return missing && `OMP has no ${missing[2]}.${missing[1]}`;
    }

    function on(name, handler) {
      // An event this OMP does not know must not take the others with it.
      try {
        omp.on(name, (event, ctx) => {
          try {
            handler(event ?? {}, ctx);
          } catch {}
        });
      } catch {}
    }

    on("session_start", (_event, ctx) => {
      // Only the main agent of a TUI session runs autopilot; the factory is
      // bound to subagents and to the contained worker's RPC session as well.
      if (root || ctx.agent.kind !== "main" || ctx.mode !== "tui" || !ctx.hasUI) return;
      if (typeof ctx.setInterval !== "function") return;
      // Managed timers only: OMP contains their throws and clears them at
      // shutdown. Without its poll autopilot stays off altogether.
      ctx.setInterval(guard(tick), AUTOPILOT_POLL_MS);
      root = ctx;
      settledAt = wall();
      unsupported = missingCall(ctx);
      if (unsupported) {
        support = "unsupported_omp";
      } else {
        // Dynamic, so that an OMP without this export still loads the
        // extension. A check that each call exists cannot see that
        // sendUserMessage began to reject for a dropped prompt in 18.18.2.
        import("@oh-my-pi/pi-coding-agent").then(sdk => sdk.VERSION).catch(() => undefined).then(guard(version => {
          if (!root || support !== undefined) return;
          unsupported = autopilotVersionProblem(version);
          support = unsupported ? "unsupported_omp" : "supported";
          try {
            if (!unsupported) restore(root);
          } finally {
            sync();
          }
        }));
      }
    });

    // An autopilot belongs to the conversation it was started in. Its end is
    // journaled where it will be read back: in the conversation being left.
    function leave(_event, ctx) {
      if (!live(ctx) || !ap) return;
      end(ctx, ap.stopping ? "stopped" : "conversation_changed");
      sync();
    }
    on("session_before_switch", (event, ctx) => {
      // A reload is a resume of the current file and keeps the autopilot.
      if (event.reason === "resume" && event.targetSessionFile !== undefined &&
          event.targetSessionFile === ctx.sessionManager.getSessionFile()) return;
      leave(event, ctx);
    });
    on("session_before_tree", leave);
    on("session_before_branch", (event, ctx) => {
      // OMP refuses a /btw branch when a handler appended an entry here, so
      // that one ends at session_branch.
      if (event.reason !== "btw") leave(event, ctx);
    });
    for (const name of ["session_switch", "session_branch", "session_tree"]) {
      on(name, (event, ctx) => {
        if (!live(ctx)) return;
        const now = wall();
        // None of these keeps a queued prompt, and a reload aborts a running
        // turn without an event reaching this extension.
        running = awaiting = ownAbort = false;
        lastTurn = awaitOverAt = sent = modeGapUntil = undefined;
        settledAt = now;
        // The reset above published the tab's activity without asking autopilot.
        rested = "idle";
        if (ap) ap.iter = ap.due = undefined;
        const reload = name === "session_switch" && typeof event.previousSessionFile === "string" &&
          event.previousSessionFile === ctx.sessionManager.getSessionFile();
        // The turn a Stop now was waiting for is gone with the rest. Its end
        // is journaled only where its own entries are: after a reload.
        if (ap?.stopping) end(ctx, "stopped", reload);
        if (!reload && support === "supported") {
          end(ctx, "conversation_changed", false);
          // An autopilot this position still shows as running is ended here.
          const found = journaled(ctx);
          if (found && found.endedAt === undefined) {
            record(ctx, found.id, "end", { iterations: found.iterations, reason: "conversation_changed" });
            found.endedAt = now;
            found.reason = "conversation_changed";
          }
          // What this conversation shows becomes the reported last autopilot.
          // One that ended in this process because of the change stays, with
          // its values for a new start, and so does any where none is shown.
          if (found && !endedByChange) ended = endedView(found);
        }
        sync();
      });
    }

    on("agent_start", (_event, ctx) => {
      if (!live(ctx)) return;
      running = true;
      awaiting = ownAbort = false;
      lastTurn = awaitOverAt = modeGapUntil = undefined;
      if (ended && !ended.turn_since) endedTurnStarted = true;
      // The prompt handed to OMP before Stop now started its run after all.
      if (ap?.stopping) abortOwn(ctx);
      sync();
    });
    on("message_start", (event, ctx) => {
      const mine = sent;
      if (!mine || !live(ctx)) return;
      const message = event.message;
      // OMP's other agent-attributed messages, a background result or plan
      // mode's context, have the role custom. Text is not compared: OMP
      // records a prompt with its mentions expanded.
      if (message?.role !== "user" || message.attribution !== "agent") return;
      if (typeof message.timestamp === "number" && message.timestamp < mine.calledAt) return;
      sent = undefined;
      if (ap?.id === mine.id && ap.iter?.n === mine.n) ap.iter.recorded = true;
      if (ap?.stopping) {
        if (running) abortOwn(ctx);
        // OMP can record the prompt without a run, when the run that was to
        // take it was interrupted first. Nothing follows that could be awaited.
        else if (ctx.isIdle()) end(ctx, "stopped");
      }
      sync();
    });
    on("tool_execution_start", (_event, ctx) => {
      if (live(ctx) && ap?.iter?.recorded) ap.iter.tools++;
    });
    on("turn_end", (event, ctx) => {
      if (!live(ctx) || !event.message) return;
      // OMP emits one turn_end per model response; only the last one counts.
      lastTurn = { stopReason: event.message.stopReason, errorMessage: event.message.errorMessage };
      if (ap?.iter?.recorded) ap.iter.lastTurn = lastTurn;
    });
    on("agent_end", (event, ctx) => {
      if (!live(ctx)) return;
      running = false;
      if (endedTurnStarted && ended) ended.turn_since = true;
      endedTurnStarted = false;
      // Not terminal while background work may still resume the turn, or
      // while a retry or OMP's own continuation follows by itself.
      const terminal = event.willContinue !== true && event.awaitingAsyncWork !== true;
      awaiting = event.awaitingAsyncWork === true;
      awaitOverAt = undefined;
      // Esc, the "." shortcut, interrupt-and-send and a cancelled ask all end
      // a turn with this marker. It applies to any turn, not only an iteration,
      // but not to a run this extension aborted itself.
      const interrupted = !ownAbort && lastTurn?.stopReason === "aborted" &&
        lastTurn.errorMessage === OMP_USER_INTERRUPT;
      ownAbort = false;
      if (ap?.stopping) end(ctx, "stopped");
      else if (ap?.iter && (interrupted || (terminal && ap.iter.recorded))) {
        finish(ctx, interrupted ? "interrupted" : outcomeOf(ap.iter), wall());
      } else if (interrupted) pause(ctx, "interrupted");
      sync();
    });
    // Esc during a retry wait or an automatic compaction only cancels that.
    // The turn's last message keeps its own stop reason, so the interrupt is
    // applied here. It is not a failed iteration.
    function cancelled(ctx) {
      if (!ap || ap.stopping || ownAbort) return;
      if (ap.iter?.recorded) finish(ctx, "interrupted", wall());
      else pause(ctx, "interrupted");
      sync();
    }
    on("auto_retry_end", (event, ctx) => {
      if (live(ctx) && event.success === false && event.finalError === OMP_RETRY_CANCELLED) cancelled(ctx);
    });
    on("auto_compaction_end", (event, ctx) => {
      if (live(ctx) && event.aborted === true) cancelled(ctx);
    });
    on("goal_updated", (event, ctx) => {
      if (!live(ctx)) return;
      // getState() gives only the status, the objective and the budget.
      goal = event.goal
        ? { status: event.goal.status, tokensUsed: event.goal.tokensUsed, tokenBudget: event.goal.tokenBudget }
        : undefined;
      sync();
    });
    on("mode_changed", (_event, ctx) => {
      if (!live(ctx) || support !== "supported") return;
      if (!switchedOn(ctx.sessionModes.getState().goal)) goal = undefined;
      if (!continuing(ctx)) modeGapUntil = undefined;
      const driver = ap?.phase === "on" ? otherDriver(ctx) : undefined;
      if (driver) pause(ctx, driver);
      sync();
    });
    on("session_shutdown", (_event, ctx) => {
      if (ctx?.agent?.kind === "main") root = undefined;
    });

    return { keepsWorking, settled: tabSettled };
  }

  try {
    autopilot = installAutopilot();
  } catch {}
}
