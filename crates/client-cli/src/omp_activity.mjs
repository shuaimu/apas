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
        publish(ctx, "idle");
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
}
