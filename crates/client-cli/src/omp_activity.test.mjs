import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as crypto from "node:crypto";
import { EventEmitter } from "node:events";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";

// No provider, network, terminal keys, or real timers. The real extension runs
// against private files and a controlled native picker/promise in an ESM VM.
// Autopilot's time is a fake clock: the VM's Date and performance, and OMP's
// managed timers, all read it, so limits and holds are tested without waiting.
const source = fs.readFileSync(new URL("./omp_activity.mjs", import.meta.url), "utf8");
const questions = [
  { id: "db", question: "Database?", recommended: 1, options: [{ label: "SQLite" }, { label: "Postgres", description: "Server", preview: "<plain>" }] },
  { id: "features", question: "Features?", multi: true, options: [{ label: "Auth" }, { label: "Search" }] },
  { id: "note", question: "Note?", options: [{ label: "None" }] },
];
const answers = [
  { id: "db", selectedOptions: ["Postgres"] },
  { id: "features", selectedOptions: [] },
  { id: "note", selectedOptions: [], customInput: "  literal free text\nwith spacing  " },
];

async function fixture(t, { mode = "tui", kind = "main", capability = true, version = "18.20.0", journal = [], prepare, importAfter } = {}) {
  const root = fs.mkdtempSync(path.join(tmpdir(), "apas-omp-bridge-"));
  const runtime = path.join(root, "runtime");
  const sessions = path.join(root, "sessions");
  fs.mkdirSync(runtime, { mode: 0o700 });
  fs.mkdirSync(sessions);
  const launch = { launch_id: crypto.randomUUID(), session_dir: sessions, features: ["autopilot_v1"] };
  const timers = new Map();
  const handlers = new Map();
  const natives = [];
  let session = "native-session";
  let failure;
  // What the fake OMP answers, and what the extension asked of it.
  const clock = { wall: Date.UTC(2026, 10, 13, 14, 0, 0), mono: 5000 };
  const managed = new Set();
  const omp = {
    idle: true, pending: false, aborts: 0, editor: "", tools: ["read", "bash", "ask"], calls: [], delays: [], activities: [],
    statuses: new Map(), branches: new Map([[session, structuredClone(journal)]]),
    jobs: { running: [], recent: [], delivery: { queued: 0, delivering: false, pendingJobIds: [] } },
    modes: { plan: { status: "inactive" }, vibe: { status: "inactive" }, goal: { status: "inactive" }, loop: { status: "inactive" } },
    throws: {},
  };
  const controller = { writer: crypto.randomUUID(), counter: 0, at: undefined, alive: false, report: {} };
  class FakeDate extends Date {
    constructor(...args) {
      if (args.length === 0) super(clock.wall);
      else super(...args);
    }
    static now() { return clock.wall; }
  }
  const branch = () => {
    if (!omp.branches.has(session)) omp.branches.set(session, []);
    return omp.branches.get(session);
  };
  function addManaged(callback, ms = 0, repeat = false) {
    omp.delays.push(ms);
    const handle = { callback, due: clock.wall + ms, every: repeat ? ms : undefined, order: omp.delays.length };
    managed.add(handle);
    return handle;
  }
  const proc = Object.assign(new EventEmitter(), {
    platform: "linux", pid: process.pid, geteuid: () => process.geteuid(),
    env: { APAS_OMP_RUNTIME: runtime, APAS_OMP_LAUNCH: launch.launch_id },
  });
  const native = (items, options) => new Promise((resolve, reject) => {
    natives.push({ items, options, resolve, reject });
    options?.signal?.addEventListener("abort", () => resolve(undefined), { once: true });
  });
  const ui = capability ? {
    askDialog: native,
    setStatus(key, text) {
      if (omp.throws.setStatus) throw new Error("injected status failure");
      if (text === undefined) omp.statuses.delete(key);
      else omp.statuses.set(key, text);
    },
    getEditorText: () => omp.editor,
    setEditorText: text => { omp.editor = text; },
  } : {};
  const ctx = {
    agent: { kind }, mode, hasUI: mode === "tui", ui,
    sessionManager: {
      getSessionId: () => session,
      getSessionFile: () => path.join(sessions, `${session}.jsonl`),
      getSessionDir: () => sessions,
      getBranch: () => branch(),
    },
    sessionModes: { getState() {
      if (omp.throws.modes) throw new Error("injected mode failure");
      return structuredClone(omp.modes);
    } },
    isIdle: () => omp.idle, hasPendingMessages: () => omp.pending,
    abort: () => { omp.aborts++; },
    getAsyncJobSnapshot() {
      if (omp.throws.jobs) throw new Error("injected job failure");
      return omp.jobs;
    },
    // OMP's managed timers, run by advance() below on the fake clock.
    setTimeout: (callback, ms) => addManaged(callback, ms),
    setInterval: (callback, ms) => addManaged(callback, ms, true),
    clearTimer: handle => managed.delete(handle),
  };
  function addTimer(callback, repeat) {
    const handle = { unref() {} };
    timers.set(handle, { callback, repeat });
    return handle;
  }
  const context = createContext({
    process: proc, Buffer, AbortController, AbortSignal, structuredClone,
    Date: FakeDate, performance: { now: () => clock.mono },
    setInterval: callback => addTimer(callback, true), clearInterval: handle => timers.delete(handle),
    setTimeout: callback => addTimer(callback, false), clearTimeout: handle => timers.delete(handle),
  });
  const modules = {
    "node:fs": {
      ...fs,
      readFileSync(file, ...args) {
        if (file === `/proc/${proc.pid}/stat`) {
          const fields = Array(20).fill("0");
          fields[0] = "S"; fields[2] = String(proc.pid); fields[19] = "4242";
          return `${proc.pid} (omp) ${fields.join(" ")}`;
        }
        return fs.readFileSync(file, ...args);
      },
      renameSync(from, to) {
        if (failure === path.basename(to)) throw new Error("injected atomic IO failure");
        omp.renaming?.(path.basename(to));
        // Every activity the extension publishes, in order.
        if (path.basename(to) === "omp-activity.json") omp.activities.push(JSON.parse(fs.readFileSync(from, "utf8")).activity);
        return fs.renameSync(from, to);
      },
    },
    "node:path": path,
    "node:crypto": crypto,
  };
  // The SDK import autopilot reads OMP's version from. With a null version the
  // import fails, as it does on an OMP that lacks the export. importAfter
  // keeps it from settling until a test lets it.
  const sdk = new SyntheticModule(["VERSION"], function () { this.setExport("VERSION", version); }, { context });
  await sdk.link(() => {});
  await sdk.evaluate();
  const extension = new SourceTextModule(source, { context, importModuleDynamically: async specifier => {
    await importAfter;
    if (specifier === "@oh-my-pi/pi-coding-agent" && version !== null) return sdk;
    throw new Error(`Cannot find package ${specifier}`);
  } });
  await extension.link(specifier => {
    const exports = modules[specifier];
    return new SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await extension.evaluate();
  function write(name, value) {
    fs.writeFileSync(path.join(runtime, name), JSON.stringify(value), { mode: 0o600 });
  }
  function read(name) {
    return JSON.parse(fs.readFileSync(path.join(runtime, name), "utf8"));
  }
  async function emit(name, event = {}, eventCtx = ctx) {
    for (const callback of handlers.get(name) ?? []) await callback(event, eventCtx);
  }
  write("omp-launch.json", launch);
  const api = {
    on(name, callback) {
      handlers.set(name, [...(handlers.get(name) ?? []), callback]);
    },
    sendUserMessage(text, options) {
      if (omp.throws.send) throw omp.throws.send;
      const call = { text, options, at: clock.wall };
      const promise = new Promise((resolve, reject) => Object.assign(call, { resolve, reject }));
      omp.calls.push(call);
      return promise;
    },
    appendEntry(customType, data) {
      if (omp.throws.append) throw new Error("injected journal failure");
      branch().push({ type: "custom", customType, data: JSON.parse(JSON.stringify(data)), id: crypto.randomUUID().slice(0, 8) });
    },
    getActiveTools: () => omp.tools,
  };
  prepare?.({ ctx, api, omp });
  extension.namespace.default(api);
  // Emulate OMP's event-context Proxy: get is handler-scoped, descriptor/set
  // delegates to the real shared UI. Reading the scoped getter is unsafe to
  // retain once the event finishes.
  function scopedContext() {
    const handler = new AbortController();
    const original = ui.askDialog;
    const proxy = new Proxy(ui, { get(target, key) {
      if (key === "askDialog" && original) return (items, options) => {
        assert.equal(handler.signal.aborted, false, "retained a completed event's UI signal");
        return original(items, options);
      };
      return target[key];
    } });
    return { context: { ...ctx, ui: proxy }, close: () => handler.abort() };
  }
  const startCtx = scopedContext();
  await emit("session_start", {}, startCtx.context);
  startCtx.close();
  t.after(async () => {
    await emit("session_shutdown");
    for (const picker of natives) picker.resolve(undefined);
    fs.rmSync(root, { recursive: true, force: true });
  });
  const microtasks = async () => { for (let turn = 0; turn < 4; turn++) await Promise.resolve(); };
  function beat(overrides = {}) {
    controller.at = clock.mono;
    write("apas-controller.json", {
      v: 1, launch_id: launch.launch_id, writer: controller.writer, counter: ++controller.counter,
      connected: true, unreachable_secs: 0, server_controls_autopilot: true, ...controller.report, ...overrides,
    });
  }
  function pass(ms) {
    clock.wall += ms;
    clock.mono += ms;
  }
  // Runs OMP's managed timers in order up to now + ms. While the fake CLI is
  // alive it writes its heartbeat every three seconds, as its own thread does.
  async function advance(ms) {
    const target = clock.wall + ms;
    for (;;) {
      let next;
      for (const timer of managed) {
        if (timer.due <= target && (!next || timer.due < next.due || (timer.due === next.due && timer.order < next.order))) next = timer;
      }
      if (!next) break;
      pass(Math.max(0, next.due - clock.wall));
      if (controller.alive && !(clock.mono - controller.at < 3000)) beat();
      if (next.every === undefined) managed.delete(next);
      else next.due = clock.wall + next.every;
      next.callback();
      await microtasks();
    }
    pass(target - clock.wall);
  }
  return {
    ctx, ui, runtime, launch, natives, read, write, emit, omp, clock, controller, beat, advance, sessions, managed,
    exists: name => fs.existsSync(path.join(runtime, name)),
    events: () => [...handlers.keys()],
    fail: name => { failure = name; },
    switchSession: value => { session = value; },
    branch,
    // The clock moves without a timer firing, as while a process is suspended.
    skip: pass,
    // Lets the extension's import of the SDK settle; a microtask is not enough.
    flush: () => new Promise(resolve => setImmediate(resolve)),
    journal: () => branch().filter(entry => entry.customType === "apas-autopilot").map(entry => entry.data),
    state: () => read("omp-autopilot.json"),
    result: id => read("omp-autopilot-ack.json").acks.find(ack => ack.request_id === id),
    command(command, envelope = {}) {
      const request = { v: 1, launch_id: launch.launch_id, request_id: crypto.randomUUID(), issued_at_ms: clock.wall, command, ...envelope };
      write("omp-autopilot-request.json", request);
      return request.request_id;
    },
    stop(action, envelope = {}) {
      const request = { v: 1, launch_id: launch.launch_id, request_id: crypto.randomUUID(), issued_at_ms: clock.wall, action, ...envelope };
      write("omp-autopilot-stop.json", request);
      return request.request_id;
    },
    async start(items = questions, options = {}) {
      const callCtx = scopedContext();
      await emit("tool_call", { toolName: "ask", toolCallId: "call-native", input: { questions: items } }, callCtx.context);
      callCtx.close();
      // Intentionally enter UI BEFORE tool_execution_start, as real OMP does.
      const result = ui.askDialog(items, options);
      await emit("tool_execution_start", { toolName: "ask", toolCallId: "call-native" });
      return { result };
    },
    request(overrides = {}) {
      const report = read("omp-question.json");
      const request = { ...report, nonce: crypto.randomUUID(), request_id: "web-request", answers, cancel: false, ...overrides };
      delete request.questions;
      write("omp-question-submit.json", request);
      return request;
    },
    async tick() {
      for (const [handle, timer] of [...timers]) {
        if (!timer.repeat) continue;
        timer.callback();
        if (!timers.has(handle)) continue;
      }
      await Promise.resolve();
    },
  };
}

test("main native capability binds execution identity and accepts exact typed selections", async t => {
  const f = await fixture(t);
  const toolAbort = new AbortController();
  const { result } = await f.start(questions, { signal: toolAbort.signal, timeout: 999, acceptImages: true });
  const report = f.read("omp-question.json");
  assert.deepEqual(report.questions, questions);
  assert.equal(report.tool_call_id, "call-native");
  assert.equal(f.read("omp-activity.json").activity, "pending_answer");
  assert.equal(fs.statSync(path.join(f.runtime, "omp-question.json")).mode & 0o777, 0o600);
  assert.equal(f.natives[0].options.timeout, 999);
  const request = f.request();
  await f.tick();
  assert.deepEqual(JSON.parse(JSON.stringify(await result)), { kind: "submit", results: answers });
  assert.equal(f.read("omp-question-ack.json").nonce, request.nonce);
  assert.equal(f.read("omp-question-ack.json").accepted, true);
  assert.equal(f.natives[0].options.signal.aborted, true);
  assert.equal(toolAbort.signal.aborted, false);
  assert.equal(f.exists("omp-question.json"), false);
  assert.equal(f.exists("omp-question-submit.json"), false);
  await f.emit("tool_execution_end", { toolCallId: "call-native" });
  assert.equal(f.read("omp-activity.json").activity, "working");
});

test("invalid IDs, labels, duplicates, blank input and single-select cardinality keep picker open", async t => {
  const f = await fixture(t);
  const { result } = await f.start();
  const invalid = [
    [],
    [{ ...answers[0], id: "wrong" }, ...answers.slice(1)],
    [{ ...answers[0], selectedOptions: ["fabricated"] }, ...answers.slice(1)],
    [{ ...answers[0], selectedOptions: ["SQLite", "Postgres"] }, ...answers.slice(1)],
    [answers[0], { ...answers[1], selectedOptions: ["Auth", "Auth"] }, answers[2]],
    [answers[0], answers[1], { ...answers[2], customInput: "  " }],
    [{ ...answers[0], selectedOptions: [] }, ...answers.slice(1)],
  ];
  for (const value of invalid) {
    f.request({ answers: value });
    await f.tick();
    assert.equal(f.read("omp-question-ack.json").accepted, false);
    assert.equal(f.exists("omp-question.json"), true);
    assert.equal(f.natives[0].options.signal.aborted, false);
  }
  f.request();
  await f.tick();
  assert.equal((await result).kind, "submit");
});

test("stale dialog, session, process generation and tool identity cannot answer", async t => {
  const f = await fixture(t);
  const { result } = await f.start();
  for (const override of [
    { dialog_id: crypto.randomUUID() }, { launch_id: crypto.randomUUID() },
    { session_id: "sibling" }, { tool_call_id: "other-call" }, { process_start: "old" },
    { transcript_path: "/tmp/unrelated.jsonl" }, { pid: process.pid + 1 },
  ]) {
    f.request(override);
    await f.tick();
    assert.equal(f.read("omp-question-ack.json").accepted, false);
    assert.equal(f.natives[0].options.signal.aborted, false);
  }
  f.request({ cancel: true, answers: [] });
  await f.tick();
  assert.equal(await result, undefined);
  assert.equal(f.read("omp-question-ack.json").accepted, true);
});

test("native terminal wins a queued web answer and leaves no false acceptance", async t => {
  const f = await fixture(t);
  const { result } = await f.start();
  f.request();
  const terminal = { kind: "submit", results: [{ id: "db", selectedOptions: ["SQLite"] }] };
  f.natives[0].resolve(terminal);
  await Promise.resolve();
  await f.tick();
  assert.equal(await result, terminal);
  assert.equal(f.exists("omp-question.json"), false);
  assert.equal(f.exists("omp-question-ack.json"), false);
});

test("prepared identical asks bind the executing call, not the latest preflight", async t => {
  const f = await fixture(t);
  for (const toolCallId of ["call-first", "call-second"]) {
    await f.emit("tool_call", { toolName: "ask", toolCallId, input: { questions } });
  }
  await f.emit("tool_call", { toolName: "read", toolCallId: "unrelated", input: {} });
  const result = f.ui.askDialog(questions);
  assert.equal(f.exists("omp-question.json"), false, "preflight is not execution proof");
  await f.emit("tool_execution_start", { toolName: "ask", toolCallId: "call-first" });
  assert.equal(f.read("omp-question.json").tool_call_id, "call-first");
  f.request({ tool_call_id: "call-second" });
  await f.tick();
  assert.equal(f.read("omp-question-ack.json").accepted, false);
  assert.equal(f.exists("omp-question.json"), true);
  f.request();
  await f.tick();
  assert.deepEqual(JSON.parse(JSON.stringify(await result)), { kind: "submit", results: answers });
  assert.equal(f.read("omp-question-ack.json").tool_call_id, "call-first");
});

test("overlapping native ask executions disable remote answering instead of guessing", async t => {
  const f = await fixture(t);
  const { result } = await f.start();
  await f.emit("tool_call", { toolName: "ask", toolCallId: "other", input: { questions } });
  await f.emit("tool_execution_start", { toolName: "ask", toolCallId: "other" });
  assert.equal(f.exists("omp-question.json"), false);
  const terminal = { kind: "submit", results: answers };
  f.natives[0].resolve(terminal);
  assert.equal(await result, terminal);
});

test("native rejection and parent abort settle without advertising a dead dialog", async t => {
  const f = await fixture(t);
  const abort = new AbortController();
  const { result } = await f.start(questions, { signal: abort.signal });
  abort.abort();
  await f.tick();
  assert.equal(await result, undefined);
  assert.equal(f.exists("omp-question.json"), false);
  await f.emit("tool_execution_end", { toolName: "ask", toolCallId: "call-native" });
  const next = await f.start();
  const rejection = assert.rejects(next.result, /native error/);
  f.natives[1].reject(new Error("native error"));
  await rejection;
  assert.equal(f.exists("omp-question.json"), false);
});

test("IO failure never dismisses or strands the native picker", async t => {
  for (const failure of ["omp-question.json", "omp-question-ack.json"]) {
    await t.test(failure, async t => {
      const f = await fixture(t);
      f.fail(failure);
      const { result } = await f.start();
      if (failure === "omp-question-ack.json") {
        f.request();
        await f.tick();
      }
      assert.equal(f.exists("omp-question.json"), false);
      assert.equal(f.natives[0].options.signal.aborted, false);
      f.natives[0].resolve(undefined);
      assert.equal(await result, undefined);
    });
  }
});

test("switch/relaunch invalidates remote capability but leaves native interaction intact", async t => {
  for (const change of ["session", "launch"]) {
    await t.test(change, async t => {
      const f = await fixture(t);
      const { result } = await f.start();
      f.request();
      if (change === "session") f.switchSession("new-session");
      else f.write("omp-launch.json", { ...f.launch, launch_id: crypto.randomUUID() });
      await f.tick();
      assert.equal(f.exists("omp-question.json"), false);
      assert.equal(f.natives[0].options.signal.aborted, false);
      f.natives[0].resolve(undefined);
      assert.equal(await result, undefined);
    });
  }
});

test("subagents, non-TUI modes, absent capability and unarmed dialogs remain terminal-only", async t => {
  for (const config of [{ kind: "subagent" }, { mode: "rpc" }, { capability: false }]) {
    await t.test(JSON.stringify(config), async t => {
      const f = await fixture(t, config);
      await f.emit("tool_call", { toolName: "ask", toolCallId: "child", input: { questions } });
      if (f.ui.askDialog) {
        const promise = f.ui.askDialog(questions, {});
        f.natives[0].resolve(undefined);
        await promise;
      }
      assert.equal(f.exists("omp-question.json"), false);
    });
  }
  const f = await fixture(t);
  const unarmed = f.ui.askDialog(questions, {});
  assert.equal(f.exists("omp-question.json"), false);
  f.natives[0].resolve(undefined);
  await unarmed;
  await f.emit("tool_call", { toolName: "ask", toolCallId: "parent", input: { questions } });
  await f.emit("tool_call", { toolName: "ask", toolCallId: "child", input: { questions } }, { ...f.ctx, agent: { kind: "subagent" } });
  const nested = f.ui.askDialog(questions, {});
  assert.equal(f.exists("omp-question.json"), false);
  f.natives[1].resolve(undefined);
  await nested;
});

test("symlinked or oversized request files cannot deliver answers", async t => {
  for (const invalid of ["symlink", "oversize"]) {
    await t.test(invalid, async t => {
      const f = await fixture(t);
      const { result } = await f.start();
      const request = f.request();
      const file = path.join(f.runtime, "omp-question-submit.json");
      fs.unlinkSync(file);
      if (invalid === "symlink") {
        const target = path.join(f.runtime, "foreign.json");
        fs.writeFileSync(target, JSON.stringify(request), { mode: 0o600 });
        fs.symlinkSync(target, file);
      } else fs.writeFileSync(file, " ".repeat(131073), { mode: 0o600 });
      await f.tick();
      assert.equal(f.exists("omp-question.json"), false);
      assert.equal(f.exists("omp-question-ack.json"), false);
      assert.equal(f.natives[0].options.signal.aborted, false);
      f.natives[0].resolve(undefined);
      await result;
    });
  }
});

test("one-question empty multi is accepted and duplicate old dialogs cannot answer a later ask", async t => {
  const f = await fixture(t);
  const items = [{ id: "none", question: "Select none?", multi: true, options: [{ label: "Optional" }] }];
  const first = await f.start(items);
  const request = f.request({ answers: [{ id: "none", selectedOptions: [] }] });
  await f.tick();
  assert.deepEqual(JSON.parse(JSON.stringify(await first.result)), {
    kind: "submit", results: [{ id: "none", selectedOptions: [] }],
  });
  await f.emit("tool_execution_end", { toolName: "ask", toolCallId: "call-native" });
  const second = await f.start(items);
  assert.notEqual(f.read("omp-question.json").dialog_id, request.dialog_id);
  f.write("omp-question-submit.json", request);
  await f.tick();
  assert.equal(f.read("omp-question-ack.json").accepted, false);
  assert.equal(f.natives[1].options.signal.aborted, false);
  f.request({ answers: [], cancel: true });
  await f.tick();
  assert.equal(await second.result, undefined);
});

test("sanitized or replaced question identity fails closed to the original native UI", async t => {
  const f = await fixture(t);
  await f.emit("tool_call", { toolName: "ask", toolCallId: "call-native", input: { questions } });
  const changed = structuredClone(questions);
  changed[0].options[0].label = "Different";
  const result = f.ui.askDialog(changed, {});
  assert.equal(f.exists("omp-question.json"), false);
  f.natives[0].resolve(undefined);
  assert.equal(await result, undefined);
});

// Autopilot. The CLI's side is played by the fixture: a heartbeat, request
// files, and reads of the acknowledgement and state files. OMP's side is the
// lifecycle events it was seen to emit (18.20.0).
const prompt = "Check the build and fix what fails.";
const NOT_TAKEN = "The tab did not take the request in time. Nothing was started.";
const UNREADABLE = "This tab's APAS cannot read this request.";
const spec = (overrides = {}) => ({
  prompt, repeat: { kind: "after_each_turn" }, iteration_limit: { kind: "none" }, time_limit: { kind: "none" },
  end_without_tool_call: false, ...overrides,
});
const atMost = value => ({ kind: "at_most", value });
const iso = ms => new Date(ms).toISOString();
const plain = value => JSON.parse(JSON.stringify(value));

// A tab whose OMP version has been read and whose CLI writes its heartbeat.
async function tab(t, options) {
  const f = await fixture(t, options);
  await f.flush();
  f.controller.alive = true;
  f.beat();
  return f;
}

// The poll reads the request files once a second.
async function send(f, command, envelope) {
  const id = f.command(command, envelope);
  await f.advance(1000);
  return id;
}
const start = (f, overrides) => send(f, { kind: "start", spec: spec(overrides) });
async function stop(f, action, envelope) {
  const id = f.stop(action, envelope);
  await f.advance(1000);
  return id;
}

async function begin(f, { own = true } = {}) {
  f.omp.idle = false;
  await f.emit("agent_start");
  await f.emit("message_start", { message: {
    role: "user", attribution: own ? "agent" : "user", timestamp: f.clock.wall,
    content: [{ type: "text", text: own ? prompt : "a person's message" }],
  } });
}
// A null stop reason is a run that reaches its end without another turn_end.
async function finish(f, { tools = 0, stopReason = "stop", errorMessage, end = {} } = {}) {
  for (let call = 0; call < tools; call++) {
    await f.emit("tool_execution_start", { toolName: "bash", toolCallId: `tool-${call}` });
    await f.emit("tool_execution_end", { toolName: "bash", toolCallId: `tool-${call}` });
  }
  if (stopReason) await f.emit("turn_end", { message: { role: "assistant", stopReason, ...(errorMessage ? { errorMessage } : {}) } });
  f.omp.idle = true;
  await f.emit("agent_end", end);
  // The settle that follows a terminal end.
  await f.advance(0);
}
// One whole run as OMP reports it: an iteration's, or a person's turn.
async function run(f, { own = true, ...how } = {}) {
  await begin(f, { own });
  await finish(f, how);
}
// A long quiet span: the clock moves on and each overdue timer then fires once.
async function jump(f, ms) {
  f.skip(ms);
  await f.advance(0);
}
const interrupted = { stopReason: "aborted", errorMessage: "Interrupted by user" };
const failing = { stopReason: "error", errorMessage: "429 usage limit reached" };
const condition = f => {
  const { kind, detail } = f.state().autopilot.condition;
  return detail ? `${kind}:${detail}` : kind;
};
const kinds = f => f.journal().map(entry => entry.kind);
const activity = f => f.read("omp-activity.json").activity;
function dropped(message = "Prompt dropped before provider dispatch.") {
  return Object.assign(new Error(message), { name: "PromptDroppedError" });
}

test("autopilot takes a start by renaming it, acknowledges it and submits once from a settled tab", async t => {
  const f = await tab(t);
  assert.equal(f.state().support, "supported");
  assert.equal(f.state().autopilot, undefined);
  const id = await start(f);
  const startedAt = f.clock.wall;
  assert.equal(f.exists("omp-autopilot-request.json"), false);
  assert.deepEqual(fs.readdirSync(f.runtime).filter(name => /taken|tmp/.test(name)), []);
  const { acks, ...scope } = f.read("omp-autopilot-ack.json");
  assert.deepEqual(scope, { v: 1, launch_id: f.launch.launch_id, pid: process.pid, process_start: "4242" });
  assert.deepEqual(acks, [{ request_id: id, outcome: "accepted", at_ms: startedAt }]);
  // Agent-attributed, and an aside: a steer would restart a run after an abort.
  assert.deepEqual(f.omp.calls.map(({ text, options }) => ({ text, options: plain(options) })),
    [{ text: prompt, options: { attribution: "agent", deliverAs: "aside" } }]);

  const { autopilot, counter, ...report } = f.state();
  assert.deepEqual(report, {
    v: 1, launch_id: f.launch.launch_id, pid: process.pid, process_start: "4242",
    session_id: "native-session", support: "supported",
  });
  assert.ok(counter > 0);
  assert.match(autopilot.autopilot_id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  assert.deepEqual(autopilot, {
    autopilot_id: autopilot.autopilot_id, spec: spec(), started_at: iso(startedAt), iterations: 1,
    condition: { kind: "running", since: iso(startedAt) },
  });
  // Only the start entry holds the prompt.
  assert.deepEqual(f.journal(), [
    { v: 1, autopilot_id: autopilot.autopilot_id, session_id: "native-session", kind: "start", at: iso(startedAt), spec: spec(), started_at: iso(startedAt) },
    { v: 1, autopilot_id: autopilot.autopilot_id, session_id: "native-session", kind: "iteration", at: iso(startedAt), iteration: 1 },
  ]);
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · iteration 1");
  for (const name of ["omp-autopilot.json", "omp-autopilot-ack.json"]) {
    assert.equal(fs.statSync(path.join(f.runtime, name)).mode & 0o777, 0o600);
  }
  // An older CLI still reads the activity report: its six fields are unchanged.
  assert.deepEqual(Object.keys(f.read("omp-activity.json")).sort(),
    ["activity", "launch_id", "pid", "process_start", "session_id", "transcript_path"]);
});

test("a start issued more than two seconds ago or in the future is refused", async t => {
  const f = await tab(t);
  for (const offset of [-1500, 5000]) {
    const id = await send(f, { kind: "start", spec: spec() }, { issued_at_ms: f.clock.wall + offset });
    assert.deepEqual(plain(f.result(id)), { request_id: id, outcome: "refused", reason: NOT_TAKEN, at_ms: f.clock.wall });
  }
  assert.equal(f.omp.calls.length, 0);
  assert.equal(f.state().autopilot, undefined);
  // Exactly the CLI's wait is still in time.
  const id = await send(f, { kind: "start", spec: spec() }, { issued_at_ms: f.clock.wall - 1000 });
  assert.equal(f.result(id).outcome, "accepted");
  assert.equal(f.omp.calls.length, 1);
  // A request the CLI wrote while this step was already running is not from
  // the future: its age is read once the file has been taken.
  const late = await tab(t);
  late.omp.renaming = name => {
    if (name.includes(".taken-")) late.skip(3);
  };
  const written = await send(late, { kind: "start", spec: spec() }, { issued_at_ms: late.clock.wall + 1002 });
  assert.equal(late.result(written).outcome, "accepted");
});

test("a start the CLI withdrew first is never applied", async t => {
  const f = await tab(t);
  f.command({ kind: "start", spec: spec() });
  // The CLI's wait ended: its removal succeeded, so it answers that nothing was started.
  fs.unlinkSync(path.join(f.runtime, "omp-autopilot-request.json"));
  await f.advance(5000);
  assert.equal(f.exists("omp-autopilot-ack.json"), false);
  assert.equal(f.omp.calls.length, 0);
  assert.equal(f.state().autopilot, undefined);
  assert.deepEqual(f.journal(), []);
});

test("a start is checked again against the contract's bounds, strictly", async t => {
  const f = await tab(t);
  const interval = "The repeat interval must be between one minute and 24 hours.";
  const iterations = "The iteration limit must be between 1 and 1000000, or none.";
  const noIterations = "Say whether autopilot has an iteration limit: a number, or none.";
  const time = "The time limit must be between one minute and 365 days, or none.";
  const noTime = "Say whether autopilot has a time limit: a duration, or none.";
  const control = "The prompt contains control characters other than newline and tab.";
  for (const [overrides, reason] of [
    [{ prompt: "  \n" }, "Autopilot needs a prompt."],
    [{ prompt: undefined }, "Autopilot needs a prompt."],
    [{ prompt: "x".repeat(16001) }, "The prompt is longer than 16000 bytes."],
    [{ prompt: "line\r\n" }, control],
    [{ prompt: "bell\u0007" }, control],
    [{ prompt: "c1\u0085" }, control],
    [{ repeat: { kind: "every", seconds: 10 } }, interval],
    [{ repeat: { kind: "every", seconds: 86401 } }, interval],
    [{ repeat: { kind: "hourly" } }, "Say when autopilot repeats."],
    [{ repeat: { kind: "after_each_turn", seconds: 60 } }, "Say when autopilot repeats."],
    [{ iteration_limit: atMost(0) }, iterations],
    [{ iteration_limit: atMost(1000001) }, iterations],
    // A limit left out must never turn into "no limit".
    [{ iteration_limit: undefined }, noIterations],
    [{ iteration_limit: { kind: "none", value: 3 } }, noIterations],
    [{ iteration_limit: { ...atMost(20), soft: true } }, noIterations],
    [{ iteration_limit: 20 }, noIterations],
    [{ time_limit: undefined }, noTime],
    [{ time_limit: atMost(10) }, time],
    [{ time_limit: atMost(31536001) }, time],
    [{ repeat: { kind: "every", seconds: 900 }, time_limit: atMost(600) }, "The time limit is shorter than the repeat interval."],
    [{ token_limit: 1000 }, UNREADABLE],
    [{ end_without_tool_call: undefined }, UNREADABLE],
  ]) {
    const id = await start(f, overrides);
    assert.deepEqual([f.result(id).outcome, f.result(id).reason], ["refused", reason], JSON.stringify(overrides).slice(0, 80));
  }
  for (const [command, envelope, reason] of [
    [{ kind: "start", spec: spec() }, { v: 2 }, "This tab's APAS does not know this request's format."],
    [{ kind: "start", spec: spec() }, { extra: true }, UNREADABLE],
    [{ kind: "start", spec: spec(), force: true }, {}, UNREADABLE],
    [{ kind: "resume" }, {}, "This autopilot is no longer current."],
    [{ kind: "stop", now: true }, {}, UNREADABLE],
  ]) {
    const id = await send(f, command, envelope);
    assert.deepEqual([f.result(id).outcome, f.result(id).reason], ["refused", reason]);
  }
  assert.equal(f.omp.calls.length, 0);
  assert.equal(f.state().autopilot, undefined);
  assert.deepEqual(f.journal(), []);
  // Newline and tab are the two control characters a prompt may hold, and the
  // upper bounds only refuse a mistyped number.
  const wide = { prompt: "first\n\tsecond", iteration_limit: atMost(1000000), time_limit: atMost(31536000), repeat: { kind: "every", seconds: 86400 } };
  const id = await start(f, wide);
  assert.equal(f.result(id).outcome, "accepted");
  assert.deepEqual(f.state().autopilot.spec, spec(wide));
  assert.equal(f.omp.calls[0].text, "first\n\tsecond");
});

test("a pause or stop is applied whatever its version, stays in its file and is applied once", async t => {
  const f = await tab(t);
  // A request that changes nothing is accepted.
  const idle = await stop(f, "stop");
  assert.equal(f.result(idle).outcome, "accepted");
  await start(f);
  await run(f, { tools: 1 });
  const first = f.state().autopilot.autopilot_id;
  const id = await stop(f, "stop", { v: 99, autopilot_id: crypto.randomUUID(), future: { field: true } });
  assert.deepEqual(plain(f.result(id)), { request_id: id, outcome: "accepted", at_ms: f.clock.wall });
  assert.equal(f.state().autopilot, undefined);
  assert.equal(f.state().last_ended.autopilot_id, first);
  // Only the CLI removes the file; its acknowledged request is no longer pending.
  assert.equal(f.read("omp-autopilot-stop.json").request_id, id);
  await start(f);
  await f.advance(4000);
  assert.equal(condition(f), "running");
  assert.equal(f.omp.calls.length, 2);
  // A repeated copy of a start has no further effect either.
  const again = await start(f, { prompt: "second" });
  assert.equal(f.result(again).outcome, "refused");
  f.write("omp-autopilot-request.json", {
    v: 1, launch_id: f.launch.launch_id, request_id: again, issued_at_ms: f.clock.wall, command: { kind: "resume", autopilot_id: f.state().autopilot.autopilot_id },
  });
  await f.advance(1000);
  assert.equal(f.result(again).reason, "This tab is already on autopilot. Stop it first.");
  assert.equal(f.exists("omp-autopilot-request.json"), false);
  const unknown = await stop(f, "stop_harder");
  assert.deepEqual([f.result(unknown).outcome, f.result(unknown).reason], ["refused", "This tab's APAS does not know this request."]);
  assert.equal(condition(f), "running");
  // The acknowledgement file keeps the last few results only.
  for (let extra = 0; extra < 20; extra++) await stop(f, "pause");
  assert.equal(f.read("omp-autopilot-ack.json").acks.length, 16);
});

test("a stop left in its file is not applied again, however many requests are answered meanwhile", async t => {
  const f = await tab(t);
  const id = await stop(f, "stop");
  await start(f);
  // The CLI removes an acknowledged stop only when it next holds its lock.
  for (let other = 0; other < 260; other++) await send(f, { kind: "resume", autopilot_id: crypto.randomUUID() });
  assert.equal(f.read("omp-autopilot-stop.json").request_id, id);
  assert.equal(condition(f), "running");
  // And it still reads as acknowledged, so the CLI does not take it for pending.
  assert.equal(f.result(id).outcome, "accepted");
  assert.equal(f.read("omp-autopilot-ack.json").acks.length, 16);
});

test("a stop written just before an iteration is due is read in the step that would submit it", async t => {
  // Off the poll's one-second grid, so that only the iteration's own timer runs when it is due.
  async function dueSoon() {
    const f = await tab(t);
    await start(f);
    await begin(f);
    await f.advance(300);
    await finish(f, { tools: 1 });
    await f.advance(4900);
    assert.equal(f.omp.calls.length, 1);
    return f;
  }
  const f = await dueSoon();
  const id = f.stop("stop");
  await f.advance(100);
  assert.equal(f.result(id).outcome, "accepted");
  assert.equal(f.state().last_ended.reason, "stopped");
  assert.equal(f.omp.calls.length, 1);
  // The heartbeat is read in that step as well.
  const held = await dueSoon();
  held.controller.report = { server_controls_autopilot: false };
  held.beat();
  await held.advance(100);
  assert.equal(condition(held), "holding:server");
  assert.equal(held.omp.calls.length, 1);
  // Without either, that step submits.
  const free = await dueSoon();
  await free.advance(100);
  assert.equal(free.omp.calls.length, 2);
});

test("pending requests are applied in the order they were issued", async t => {
  await t.test("a stop issued before the start", async t => {
    const f = await tab(t);
    f.stop("stop", { issued_at_ms: f.clock.wall });
    await send(f, { kind: "start", spec: spec() }, { issued_at_ms: f.clock.wall + 1 });
    assert.equal(condition(f), "running");
  });
  await t.test("a stop issued after the start", async t => {
    const f = await tab(t);
    const stopped = f.stop("stop", { issued_at_ms: f.clock.wall + 1 });
    const started = await send(f, { kind: "start", spec: spec() }, { issued_at_ms: f.clock.wall });
    assert.deepEqual([f.result(started).outcome, f.result(stopped).outcome], ["accepted", "accepted"]);
    assert.equal(f.state().autopilot, undefined);
    assert.equal(f.state().last_ended.reason, "stopped");
    assert.equal(f.omp.calls.length, 0);
    assert.deepEqual(kinds(f), ["start", "end"]);
  });
});

test("requests of another launch, and insecure request files, are not applied", async t => {
  const f = await tab(t);
  const foreign = await send(f, { kind: "start", spec: spec() }, { launch_id: crypto.randomUUID() });
  assert.equal(f.exists("omp-autopilot-ack.json"), false);
  assert.equal(f.omp.calls.length, 0);
  await start(f);
  const file = path.join(f.runtime, "omp-autopilot-stop.json");
  const valid = { v: 1, launch_id: f.launch.launch_id, request_id: "web-stop", issued_at_ms: f.clock.wall, action: "stop" };
  const attempts = {
    "another launch": () => f.write("omp-autopilot-stop.json", { ...valid, launch_id: crypto.randomUUID() }),
    symlink() {
      fs.writeFileSync(path.join(f.runtime, "foreign.json"), JSON.stringify(valid), { mode: 0o600 });
      fs.symlinkSync(path.join(f.runtime, "foreign.json"), file);
    },
    oversize: () => fs.writeFileSync(file, JSON.stringify({ ...valid, padding: " ".repeat(65536) }), { mode: 0o600 }),
    "readable by others": () => fs.writeFileSync(file, JSON.stringify(valid), { mode: 0o644 }),
    "no request id": () => f.write("omp-autopilot-stop.json", { ...valid, request_id: "" }),
  };
  for (const [name, attempt] of Object.entries(attempts)) {
    fs.rmSync(file, { force: true });
    attempt();
    await f.advance(2000);
    assert.equal(condition(f), "running", name);
  }
  assert.equal(f.result(foreign), undefined);
  // A provider that was launched again owns these files: this one reads none
  // of them, and submits nothing more.
  await run(f, { tools: 1 });
  fs.rmSync(file, { force: true });
  f.write("omp-autopilot-stop.json", valid);
  f.write("omp-launch.json", { ...f.launch, launch_id: crypto.randomUUID() });
  const before = f.state().counter;
  await f.advance(40000);
  assert.equal(f.result("web-stop"), undefined);
  assert.equal(f.state().counter, before);
  assert.equal(f.omp.calls.length, 1);
});

test("autopilot runs only for the main agent of a TUI session on a supported OMP", async t => {
  for (const config of [{ kind: "subagent" }, { mode: "rpc" }]) {
    await t.test(JSON.stringify(config), async t => {
      const f = await tab(t, config);
      const id = f.command({ kind: "start", spec: spec() });
      await f.advance(5000);
      // No poll was started, so nothing is read, written or submitted.
      assert.deepEqual(f.omp.delays, []);
      assert.equal(f.exists("omp-autopilot.json"), false);
      assert.equal(f.exists("omp-autopilot-request.json"), true);
      assert.equal(f.omp.calls.length, 0);
      assert.equal(typeof id, "string");
    });
  }
  const tooOld = [
    [{ version: "18.18.1" }, /older than 18\.18\.2/],
    [{ version: "17.99.99" }, /older than 18\.18\.2/],
    // The import of the SDK failed.
    [{ version: null }, /does not report its version/],
    [{ prepare: ({ ctx }) => { delete ctx.sessionModes; } }, /no ctx\.sessionModes\.getState/],
    [{ prepare: ({ api }) => { delete api.sendUserMessage; } }, /no pi\.sendUserMessage/],
    [{ prepare: ({ ctx }) => { delete ctx.getAsyncJobSnapshot; } }, /no ctx\.getAsyncJobSnapshot/],
  ];
  for (const [config, reason] of tooOld) {
    await t.test(`unsupported ${reason}`, async t => {
      const f = await tab(t, config);
      await f.advance(1000);
      assert.equal(f.state().support, "unsupported_omp");
      assert.match(f.state().unsupported_reason, reason);
      const id = await start(f);
      assert.deepEqual([f.result(id).outcome, f.result(id).reason], ["refused", "This tab's OMP version cannot run autopilot."]);
      // A stop is still answered: there is nothing to stop.
      assert.equal(f.result(await stop(f, "stop_now")).outcome, "accepted");
      assert.equal(f.omp.calls.length, 0);
      assert.equal(f.omp.statuses.size, 0);
    });
  }
  for (const version of ["18.18.2", "18.19.0", "19.0.0-beta.1"]) {
    const f = await tab(t, { version });
    await f.advance(1000);
    assert.equal(f.state().support, "supported", version);
    assert.equal(f.state().unsupported_reason, undefined);
  }
});

test("a second start is refused, and pause and resume keep the progress and the time limit end", async t => {
  const f = await tab(t);
  await start(f, { iteration_limit: atMost(20), time_limit: atMost(28800) });
  const first = f.state().autopilot;
  assert.equal(first.deadline, iso(Date.parse(first.started_at) + 28800000));
  const again = await start(f, { prompt: "something else" });
  assert.deepEqual([f.result(again).outcome, f.result(again).reason], ["refused", "This tab is already on autopilot. Stop it first."]);
  assert.deepEqual(f.state().autopilot.spec, first.spec);
  await run(f, { tools: 1 });
  const paused = await stop(f, "pause");
  assert.equal(f.result(paused).outcome, "accepted");
  assert.equal(condition(f), "paused:requested");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused");
  await f.advance(120000);
  assert.equal(f.omp.calls.length, 1);
  // A start is refused for a paused autopilot too, and a pause while paused changes nothing.
  assert.equal(f.result(await start(f)).reason, "This tab is already on autopilot. Stop it first.");
  assert.equal(f.result(await stop(f, "pause")).outcome, "accepted");
  const other = await send(f, { kind: "resume", autopilot_id: crypto.randomUUID() });
  assert.deepEqual([f.result(other).outcome, f.result(other).reason], ["refused", "This autopilot is no longer current."]);
  assert.equal(condition(f), "paused:requested");
  const resumed = await send(f, { kind: "resume", autopilot_id: first.autopilot_id });
  assert.equal(f.result(resumed).outcome, "accepted");
  assert.equal(f.omp.calls.length, 2);
  const now = f.state().autopilot;
  assert.deepEqual([now.autopilot_id, now.iterations, now.deadline, now.started_at], [first.autopilot_id, 2, first.deadline, first.started_at]);
  assert.deepEqual(kinds(f), ["start", "iteration", "pause", "resume", "iteration"]);
  assert.deepEqual(f.journal().filter(entry => entry.kind !== "start" && entry.kind !== "iteration").map(({ iterations, reason }) => ({ iterations, reason })),
    [{ iterations: 1, reason: "requested" }, { iterations: 1, reason: undefined }]);
  // A resume of an autopilot that is on is not current either.
  assert.equal(f.result(await send(f, { kind: "resume", autopilot_id: first.autopilot_id })).outcome, "refused");
});

test("stop ends autopilot at once and lets the turn in progress finish", async t => {
  const f = await tab(t);
  await start(f, { iteration_limit: atMost(20) });
  await begin(f);
  const id = await stop(f, "stop");
  assert.equal(f.result(id).outcome, "accepted");
  const report = f.state();
  assert.equal(report.autopilot, undefined);
  assert.deepEqual({ ...report.last_ended, autopilot_id: undefined }, {
    autopilot_id: undefined, spec: spec({ iteration_limit: atMost(20) }), started_at: iso(f.clock.wall - 1000), iterations: 1,
    reason: "stopped", ended_at: iso(f.clock.wall), turn_since: false,
  });
  assert.equal(f.omp.aborts, 0);
  assert.equal(activity(f), "working");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Stopped");
  await finish(f, { tools: 2 });
  await f.advance(60000);
  assert.equal(f.omp.calls.length, 1);
  assert.equal(activity(f), "idle");
  // That turn began before the end. Lists drop the reason once a later turn has run.
  assert.equal(f.state().last_ended.turn_since, false);
  await run(f, { own: false });
  assert.equal(f.state().last_ended.turn_since, true);
  assert.equal(f.omp.statuses.has("apas-autopilot"), false);
  assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
});

test("stop now interrupts the turn in progress and is not taken for a person's interrupt", async t => {
  const f = await tab(t);
  await start(f);
  await begin(f);
  const id = await stop(f, "stop_now");
  assert.equal(f.result(id).outcome, "accepted");
  assert.equal(f.omp.aborts, 1);
  // Reported as stopped when the run it interrupted has ended.
  assert.equal(condition(f), "running");
  await finish(f, interrupted);
  assert.equal(f.state().autopilot, undefined);
  assert.equal(f.state().last_ended.reason, "stopped");
  assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
  await f.advance(60000);
  assert.equal(f.omp.calls.length, 1);

  await t.test("a person's turn, with autopilot paused", async t => {
    const f = await tab(t);
    await start(f);
    await run(f);
    await stop(f, "pause");
    await begin(f, { own: false });
    await stop(f, "stop_now");
    assert.equal(f.omp.aborts, 1);
    await finish(f, interrupted);
    assert.equal(f.state().last_ended.reason, "stopped");
  });
  await t.test("nothing in progress", async t => {
    const f = await tab(t);
    await start(f);
    await run(f);
    await stop(f, "stop_now");
    assert.equal(f.omp.aborts, 0);
    assert.equal(f.state().last_ended.reason, "stopped");
    // And with no autopilot at all, nothing is interrupted.
    await begin(f, { own: false });
    assert.equal(f.result(await stop(f, "stop_now")).outcome, "accepted");
    assert.equal(f.omp.aborts, 0);
  });
  await t.test("the retry or the compaction it cancels is not a person's interrupt either", async t => {
    for (const cancelled of [
      ["auto_retry_end", { success: false, attempt: 1, finalError: "Retry cancelled" }],
      ["auto_compaction_end", { action: "handoff", aborted: true, willRetry: false }],
    ]) {
      const f = await tab(t);
      await start(f);
      await begin(f);
      await f.emit("turn_end", { message: { role: "assistant", ...failing } });
      await stop(f, "stop_now");
      await f.emit(...cancelled);
      assert.equal(condition(f), "running", cancelled[0]);
      await finish(f, { stopReason: null });
      assert.equal(f.state().last_ended.reason, "stopped");
      assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
    }
  });
  await t.test("nor does it pause an autopilot that is started while the aborted run unwinds", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    await stop(f, "stop_now");
    await stop(f, "stop_now");
    assert.equal(f.state().last_ended.reason, "stopped");
    await start(f);
    assert.equal(condition(f), "starting");
    await f.emit("auto_retry_end", { success: false, attempt: 1, finalError: "Retry cancelled" });
    assert.equal(condition(f), "starting");
    await finish(f, interrupted);
    // The abort was this extension's own: the first iteration follows the settle.
    assert.equal(condition(f), "running");
    assert.equal(f.omp.calls.length, 2);
    // A person's Escape in a later run is theirs.
    await run(f, interrupted);
    assert.equal(condition(f), "paused:interrupted");
  });
  await t.test("a turn whose end never reaches the extension is not waited for without end", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    // A /compact typed during the turn cut it short: no turn_end or agent_end follows.
    await stop(f, "stop_now");
    // While OMP is busy the stop waits for the turn, however long.
    await f.advance(20000);
    assert.equal(condition(f), "running");
    f.omp.idle = true;
    await f.advance(4000);
    assert.equal(condition(f), "running");
    await f.advance(2000);
    assert.equal(f.state().autopilot, undefined);
    assert.equal(f.state().last_ended.reason, "stopped");
    assert.equal(f.omp.aborts, 1);
    // An iteration that is still outstanding is waited for, idle or not.
    const outstanding = await tab(t);
    await start(outstanding);
    await stop(outstanding, "stop_now");
    await outstanding.advance(60000);
    assert.equal(condition(outstanding), "running");
  });
});

test("stop now reaches an iteration OMP had accepted but not started", async t => {
  await t.test("its run starts late and is aborted too", async t => {
    const f = await tab(t);
    await start(f);
    await stop(f, "stop_now");
    assert.equal(f.omp.aborts, 1);
    assert.equal(condition(f), "running");
    await begin(f);
    assert.ok(f.omp.aborts >= 2);
    assert.equal(condition(f), "running");
    await finish(f, interrupted);
    assert.equal(f.state().last_ended.reason, "stopped");
    assert.equal(f.state().last_ended.iterations, 1);
  });
  await t.test("OMP records the prompt without a run", async t => {
    const f = await tab(t);
    await start(f);
    await stop(f, "stop_now");
    await f.emit("message_start", { message: { role: "user", attribution: "agent", content: [{ type: "text", text: prompt }], timestamp: f.clock.wall } });
    assert.equal(f.state().last_ended.reason, "stopped");
  });
  await t.test("OMP drops the prompt", async t => {
    const f = await tab(t);
    await start(f);
    await stop(f, "stop_now");
    f.omp.calls[0].reject(dropped());
    await f.advance(0);
    assert.equal(f.state().last_ended.reason, "stopped");
    assert.equal(f.state().last_ended.iterations, 0);
    assert.deepEqual(kinds(f), ["start", "iteration", "not_submitted", "end"]);
  });
  await t.test("a second stop does not wait for the turn again", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    await stop(f, "stop_now");
    assert.equal(condition(f), "running");
    await stop(f, "stop_now");
    assert.equal(f.state().last_ended.reason, "stopped");
    assert.equal(f.omp.aborts, 1);
  });
});

test("as soon as each turn ends: an iteration is due five seconds after the tab settles", async t => {
  const f = await tab(t);
  await start(f);
  await run(f, { tools: 1 });
  const settledAt = f.clock.wall;
  assert.equal(condition(f), "waiting:next_iteration");
  assert.equal(f.state().autopilot.next_iteration_at, iso(settledAt + 5000));
  await f.advance(4999);
  assert.equal(f.omp.calls.length, 1);
  await f.advance(1);
  assert.equal(f.omp.calls.length, 2);
  assert.equal(f.omp.calls[1].at, settledAt + 5000);
  assert.equal(f.omp.calls[1].text, prompt);
  // A person's message is an ordinary turn: it neither replaces the prompt
  // nor counts as an iteration, and the next iteration follows its settle.
  await run(f, { tools: 1 });
  await f.advance(2000);
  await begin(f, { own: false });
  assert.equal(condition(f), "waiting:settle");
  await f.advance(30000);
  assert.equal(f.omp.calls.length, 2);
  await finish(f);
  assert.equal(f.state().autopilot.iterations, 2);
  await f.advance(4999);
  assert.equal(f.omp.calls.length, 2);
  await f.advance(1);
  assert.deepEqual(f.omp.calls.map(call => call.text), [prompt, prompt, prompt]);
  assert.deepEqual(kinds(f), ["start", "iteration", "iteration", "iteration"]);
});

test("a timed repeat runs from submission to submission and never sooner than five seconds after a settle", async t => {
  const f = await tab(t);
  await start(f, { repeat: { kind: "every", seconds: 900 } });
  // The first iteration is not one interval away.
  assert.equal(f.omp.calls.length, 1);
  const first = f.omp.calls[0].at;
  await begin(f);
  await jump(f, 180000);
  await finish(f, { tools: 1 });
  assert.equal(condition(f), "waiting:next_iteration");
  assert.equal(f.state().autopilot.next_iteration_at, iso(first + 900000));
  // A person steers the tab meanwhile; the schedule does not move.
  await jump(f, 120000);
  await run(f, { own: false });
  assert.equal(f.state().autopilot.next_iteration_at, iso(first + 900000));
  await jump(f, first + 900000 - f.clock.wall - 3000);
  await f.advance(2999);
  assert.equal(f.omp.calls.length, 1);
  await f.advance(1);
  assert.equal(f.omp.calls.length, 2);
  assert.equal(f.omp.calls[1].at, first + 900000);
  // A turn that outlasts the interval: five seconds after it settles, and
  // the one after that an interval after this submission.
  await begin(f);
  await jump(f, 1500000);
  assert.equal(condition(f), "running");
  await finish(f, { tools: 1 });
  const settledAt = f.clock.wall;
  await f.advance(4999);
  assert.equal(f.omp.calls.length, 2);
  await f.advance(1);
  assert.equal(f.omp.calls[2].at, settledAt + 5000);
  await run(f, { tools: 1 });
  assert.equal(f.state().autopilot.next_iteration_at, iso(settledAt + 5000 + 900000));
});

test("a start on a working tab submits nothing until that turn ends", async t => {
  const f = await tab(t);
  await begin(f, { own: false });
  const id = await start(f);
  assert.equal(f.result(id).outcome, "accepted");
  assert.equal(condition(f), "starting");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · starting");
  await f.advance(20000);
  assert.equal(f.omp.calls.length, 0);
  assert.equal(f.state().autopilot.iterations, 0);
  await finish(f);
  // The first iteration is due as soon as the tab has settled.
  assert.equal(f.omp.calls.length, 1);
  assert.equal(condition(f), "running");
});

test("nothing is submitted while a message is queued, the agent is busy or a submission is outstanding", async t => {
  const f = await tab(t);
  await start(f);
  await run(f);
  f.omp.pending = true;
  await f.advance(20000);
  assert.equal(f.omp.calls.length, 1);
  f.omp.pending = false;
  f.omp.idle = false;
  await f.advance(20000);
  assert.equal(f.omp.calls.length, 1);
  f.omp.idle = true;
  await f.advance(1000);
  assert.equal(f.omp.calls.length, 2);
  // OMP holds a prompt behind a compaction typed in the terminal, which this
  // extension cannot see: it counts as the turn in progress from its submission.
  await f.advance(120000);
  assert.equal(f.omp.calls.length, 2);
  assert.equal(condition(f), "running");
  // The promise is not the signal: into a live run it resolves at once.
  f.omp.calls[1].resolve();
  await f.advance(10000);
  assert.equal(f.omp.calls.length, 2);
  // Another agent-attributed message is not this submission: a background
  // result has the role custom, and an older prompt an older timestamp.
  await f.emit("message_start", { message: { role: "custom", customType: "async-result", attribution: "agent", timestamp: f.clock.wall } });
  await f.emit("message_start", { message: { role: "user", attribution: "agent", content: [{ type: "text", text: prompt }], timestamp: f.omp.calls[1].at - 1 } });
  await f.emit("message_start", { message: { role: "user", attribution: "user", content: [{ type: "text", text: prompt }], timestamp: f.clock.wall } });
  await f.advance(10000);
  assert.equal(f.omp.calls.length, 2);
  await run(f, { tools: 1 });
  await f.advance(5000);
  assert.equal(f.omp.calls.length, 3);
});

test("a turn that waits for background work is waited for, and a wait that ends without a wake pauses", async t => {
  const f = await tab(t);
  const waiting = { end: { willContinue: true, awaitingAsyncWork: true }, tools: 1 };
  await start(f);
  f.omp.jobs.running = [{ id: "bg_1" }];
  await run(f, waiting);
  await f.advance(60000);
  assert.equal(condition(f), "waiting:background_work");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · iteration 1 · waiting for background work");
  assert.equal(f.omp.calls.length, 1);
  assert.equal(activity(f), "working");
  // The job ends and wakes the tab: its result starts a run of its own.
  f.omp.jobs.running = [];
  f.omp.idle = false;
  await f.emit("agent_start");
  await f.emit("message_start", { message: { role: "custom", customType: "async-result", attribution: "agent", timestamp: f.clock.wall } });
  assert.equal(condition(f), "running");
  await finish(f);
  await f.advance(5000);
  assert.equal(f.omp.calls.length, 2);

  // Esc on the wait row cannot be seen. While the job runs autopilot waits.
  f.omp.jobs.running = [{ id: "bg_2" }];
  await run(f, waiting);
  await f.advance(30000);
  assert.equal(condition(f), "waiting:background_work");
  // The job has finished, or was killed, and nothing is left to deliver:
  // five seconds later, with no run started, the wait is over.
  f.omp.jobs = { running: [], recent: [{ id: "bg_2", status: "cancelled" }], delivery: { queued: 0, delivering: false, pendingJobIds: [] } };
  await f.advance(5000);
  assert.equal(condition(f), "waiting:background_work");
  await f.advance(1000);
  assert.equal(condition(f), "paused:background_wait_ended");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused: the wait for background work was ended in the terminal");
  assert.equal(activity(f), "idle");
  await f.advance(60000);
  assert.equal(f.omp.calls.length, 2);
  assert.deepEqual(f.journal().at(-1), { v: 1, autopilot_id: f.state().autopilot.autopilot_id, session_id: "native-session", kind: "pause", at: f.journal().at(-1).at, iterations: 2, reason: "background_wait_ended" });

  await t.test("a result still being delivered, or a run that starts in the five seconds, is not the end of the wait", async t => {
    const f = await tab(t);
    await start(f);
    f.omp.jobs.running = [{ id: "bg_1" }];
    await run(f, waiting);
    f.omp.jobs = { running: [], recent: [], delivery: { queued: 1, delivering: true, pendingJobIds: ["bg_1"] } };
    await f.advance(20000);
    assert.equal(condition(f), "waiting:background_work");
    f.omp.jobs.delivery = { queued: 0, delivering: false, pendingJobIds: [] };
    await f.advance(3000);
    f.omp.idle = false;
    await f.emit("agent_start");
    await f.advance(10000);
    assert.equal(condition(f), "running");
    await finish(f);
    assert.equal(condition(f), "waiting:next_iteration");
  });
});

test("a dropped submission uses up no iteration, and the prompt is cleared from the input line", async t => {
  const tabbed = "check\tthe build";
  const f = await tab(t);
  await start(f, { prompt: tabbed, iteration_limit: atMost(5) });
  assert.equal(f.state().autopilot.iterations, 1);
  // OMP hands the prompt back before it rejects; its editor stores a tab as three spaces.
  f.omp.editor = "check   the build";
  f.omp.calls[0].reject(dropped());
  await f.advance(0);
  assert.equal(f.omp.editor, "");
  assert.equal(f.state().autopilot.iterations, 0);
  // No refusal reason and no abort of its own: a person's interrupt, not a failure.
  assert.equal(condition(f), "paused:interrupted");
  assert.deepEqual(f.journal().map(({ kind, iteration, iterations, reason }) => plain({ kind, iteration, iterations, reason })), [
    { kind: "start" }, { kind: "iteration", iteration: 1 }, { kind: "not_submitted", iteration: 1 },
    { kind: "pause", iterations: 0, reason: "interrupted" },
  ]);
  await send(f, { kind: "resume", autopilot_id: f.state().autopilot.autopilot_id });
  assert.equal(f.omp.calls.length, 2);
  assert.equal(f.state().autopilot.iterations, 1);
  // What a person typed meanwhile is theirs.
  f.omp.editor = "my own draft";
  f.omp.calls[1].reject(dropped("Session cost limit reached"));
  await f.advance(0);
  assert.equal(f.omp.editor, "my own draft");
  // A refusal reason is a failed iteration: the next one waits a minute.
  assert.equal(condition(f), "starting");
  assert.equal(f.state().autopilot.iterations, 0);
  await f.advance(59000);
  assert.equal(f.omp.calls.length, 2);
  await f.advance(1000);
  assert.equal(f.omp.calls.length, 3);
  // A rejection after OMP recorded the message journals nothing and changes no count.
  await begin(f);
  const entries = f.journal().length;
  f.omp.calls[2].reject(dropped());
  await f.advance(0);
  assert.equal(f.journal().length, entries);
  assert.equal(f.state().autopilot.iterations, 1);
  assert.equal(condition(f), "running");
});

test("an autopilot with no limit goes on until someone stops it", async t => {
  const f = await tab(t);
  await start(f);
  for (let iteration = 1; iteration <= 12; iteration++) {
    assert.equal(f.omp.calls.length, iteration);
    await run(f, { tools: 1 });
    await f.advance(5000);
  }
  // Three days later it is still on, with its count and no limit to show.
  await run(f, { tools: 1 });
  await jump(f, 3 * 86400000);
  await f.advance(1000);
  const view = f.state().autopilot;
  assert.deepEqual([view.iterations, view.spec.iteration_limit, view.spec.time_limit, view.deadline],
    [14, { kind: "none" }, { kind: "none" }, undefined]);
  assert.equal(condition(f), "running");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · iteration 14");
  assert.equal(f.state().last_ended, undefined);
  await run(f, { tools: 1 });
  await stop(f, "stop");
  assert.equal(f.state().last_ended.reason, "stopped");
  assert.equal(f.state().last_ended.iterations, 14);
});

test("an iteration limit ends autopilot when the turn of the last iteration ends", async t => {
  const f = await tab(t);
  await start(f, { iteration_limit: atMost(2) });
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · iteration 1 of 2");
  await run(f, { tools: 1 });
  await f.advance(5000);
  assert.equal(f.omp.calls.length, 2);
  // The limit does not cut the last turn short.
  await begin(f);
  await f.advance(30000);
  assert.equal(condition(f), "running");
  await finish(f, { tools: 1 });
  assert.equal(f.state().autopilot, undefined);
  assert.equal(f.state().last_ended.reason, "iteration_limit");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Ended: iteration limit reached");
  await f.advance(60000);
  assert.equal(f.omp.calls.length, 2);
  // A rule that ends autopilot wins over one that pauses it: nothing is left to resume.
  for (const how of [interrupted, failing, { stopReason: null }]) {
    const f = await tab(t);
    await start(f, { iteration_limit: atMost(1) });
    await run(f, how);
    assert.equal(f.state().last_ended.reason, "iteration_limit", JSON.stringify(how));
    assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
  }
});

test("a time limit ends autopilot whatever it is doing and lets a turn in progress finish", async t => {
  const limited = { time_limit: atMost(600) };
  await t.test("during a turn", async t => {
    const f = await tab(t);
    await start(f, limited);
    const deadline = Date.parse(f.state().autopilot.deadline);
    assert.equal(deadline, Date.parse(f.state().autopilot.started_at) + 600000);
    await begin(f);
    await jump(f, 590000);
    assert.equal(condition(f), "running");
    await f.advance(10000);
    const report = f.state();
    assert.equal(report.autopilot, undefined);
    assert.deepEqual([report.last_ended.reason, report.last_ended.ended_at], ["time_limit", iso(deadline)]);
    // The turn is not interrupted and stays Working until it ends.
    assert.equal(f.omp.aborts, 0);
    assert.equal(activity(f), "working");
    await finish(f, { tools: 1 });
    await f.advance(60000);
    assert.equal(f.omp.calls.length, 1);
  });
  await t.test("while waiting for background work", async t => {
    const f = await tab(t);
    await start(f, limited);
    f.omp.jobs.running = [{ id: "bg_1" }];
    await run(f, { end: { willContinue: true, awaitingAsyncWork: true } });
    await jump(f, 600000);
    assert.equal(f.state().last_ended.reason, "time_limit");
  });
  await t.test("while paused, after which it can no longer be resumed", async t => {
    const f = await tab(t);
    await start(f, limited);
    await run(f);
    await stop(f, "pause");
    const { autopilot_id: id, deadline } = f.state().autopilot;
    // Its timer runs late, as after the machine was suspended: the end is still the limit.
    await jump(f, 600000);
    assert.deepEqual([f.state().last_ended.reason, f.state().last_ended.ended_at], ["time_limit", deadline]);
    assert.deepEqual([f.journal().at(-1).kind, f.journal().at(-1).at], ["end", deadline]);
    const resumed = await send(f, { kind: "resume", autopilot_id: id });
    assert.equal(f.result(resumed).reason, "This autopilot is no longer current.");
  });
  await t.test("while held", async t => {
    const f = await tab(t);
    await start(f, { ...limited, repeat: { kind: "every", seconds: 300 } });
    await run(f);
    f.controller.alive = false;
    await f.advance(70000);
    assert.equal(condition(f), "holding:controller");
    await jump(f, 600000);
    assert.equal(f.state().last_ended.reason, "time_limit");
  });
  await t.test("when it would pass before the next iteration is due", async t => {
    const f = await tab(t);
    await start(f, { repeat: { kind: "every", seconds: 900 }, time_limit: atMost(1200) });
    await run(f, { tools: 1 });
    assert.equal(condition(f), "waiting:next_iteration");
    await jump(f, 900000);
    assert.equal(f.omp.calls.length, 2);
    // The third would be due 1800 s after the start.
    await run(f, { tools: 1 });
    assert.equal(f.state().last_ended.reason, "time_limit_before_next");
    assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Ended: the time limit ends before the next iteration is due");
  });
  await t.test("a far deadline is armed a day at a time, on a timer of its own", async t => {
    const f = await tab(t);
    await start(f, { time_limit: atMost(3 * 86400 + 3600) });
    const deadline = Date.parse(f.state().autopilot.deadline);
    await begin(f);
    // Only the deadline's timer is left to run: the turn never ends and the poll is gone.
    for (const timer of f.managed) if (timer.every !== undefined) f.managed.delete(timer);
    const before = f.omp.delays.length;
    await f.advance(deadline - f.clock.wall - 1);
    assert.notEqual(f.state().autopilot, undefined);
    await f.advance(1);
    assert.equal(f.state().last_ended.reason, "time_limit");
    assert.equal(f.state().last_ended.ended_at, iso(deadline));
    assert.deepEqual(f.omp.delays.slice(before), [86400000, 86400000, 3600000]);
    assert.ok(Math.max(...f.omp.delays) <= 86400000);
  });
});

test("ending on an iteration without a tool call applies only to a turn that completed", async t => {
  const f = await tab(t);
  await start(f, { end_without_tool_call: true });
  await run(f, { tools: 2 });
  assert.equal(condition(f), "waiting:next_iteration");
  await f.advance(5000);
  // A failed iteration is not an iteration with nothing to do.
  await run(f, failing);
  assert.notEqual(f.state().autopilot, undefined);
  await f.advance(60000);
  assert.equal(f.omp.calls.length, 3);
  // Tools a person's turn called are not the iteration's.
  await run(f);
  assert.equal(f.state().autopilot, undefined);
  assert.equal(f.state().last_ended.reason, "no_tool_call");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Ended: the last iteration made no tool call");

  await t.test("an interrupted one pauses instead", async t => {
    const f = await tab(t);
    await start(f, { end_without_tool_call: true });
    await run(f, interrupted);
    assert.equal(condition(f), "paused:interrupted");
  });
  await t.test("the option is off", async t => {
    const f = await tab(t);
    await start(f, { repeat: { kind: "every", seconds: 60 } });
    await run(f);
    assert.equal(condition(f), "waiting:next_iteration");
  });
});

test("a failed iteration waits a minute, and three in a row pause autopilot", async t => {
  const f = await tab(t);
  await start(f);
  await run(f, failing);
  assert.equal(condition(f), "waiting:next_iteration");
  await f.advance(59000);
  assert.equal(f.omp.calls.length, 1);
  await f.advance(1000);
  assert.equal(f.omp.calls.length, 2);
  // A success in between starts the count again.
  await run(f, { tools: 1 });
  await f.advance(5000);
  for (let failure = 1; failure <= 2; failure++) {
    await run(f, failure === 1 ? failing : { stopReason: "aborted", errorMessage: "stream closed" });
    assert.equal(condition(f), "waiting:next_iteration");
    await f.advance(60000);
  }
  assert.equal(f.omp.calls.length, 5);
  // A turn that reached no turn_end at all did not complete either.
  await run(f, { stopReason: null });
  assert.equal(condition(f), "paused:failures");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused after three failed iterations");
  await jump(f, 3600000);
  assert.equal(f.omp.calls.length, 5);
  // So does a resume: one more failure does not pause again.
  await send(f, { kind: "resume", autopilot_id: f.state().autopilot.autopilot_id });
  assert.equal(f.omp.calls.length, 6);
  await run(f, failing);
  assert.equal(condition(f), "waiting:next_iteration");

  await t.test("a retry that fails before its run starts ends no run, and still counts", async t => {
    const f = await tab(t);
    await start(f);
    for (let failure = 1; failure <= 3; failure++) {
      await begin(f);
      await f.emit("turn_end", { message: { role: "assistant", ...failing } });
      await f.emit("auto_retry_start", { attempt: 1, maxAttempts: 10, delayMs: 400 });
      f.omp.idle = true;
      await f.emit("agent_end", { willContinue: true });
      // Neither an agent_start nor a terminal agent_end follows.
      await f.emit("auto_retry_end", { success: false, attempt: 1, finalError: "No API key" });
      await f.advance(1000);
      if (failure === 3) break;
      assert.equal(condition(f), "waiting:next_iteration");
      await f.advance(58000);
      assert.equal(f.omp.calls.length, failure);
      await f.advance(2000);
      assert.equal(f.omp.calls.length, failure + 1);
    }
    assert.equal(condition(f), "paused:failures");
    assert.equal(f.omp.calls.length, 3);
  });
  await t.test("a prompt OMP refuses or cannot take counts as failed, without using an iteration", async t => {
    const f = await tab(t);
    await start(f);
    f.omp.calls[0].reject(new Error("No model selected"));
    await f.advance(0);
    assert.equal(f.state().autopilot.iterations, 0);
    f.omp.throws.send = new Error("extension runtime is gone");
    await f.advance(60000);
    assert.equal(f.state().autopilot.iterations, 0);
    assert.equal(condition(f), "starting");
    f.omp.throws.send = undefined;
    // The journal is what tells an iteration from a person's message: without it nothing is sent.
    f.omp.throws.append = true;
    await f.advance(60000);
    assert.equal(f.omp.calls.length, 1);
    assert.equal(condition(f), "paused:failures");
    assert.deepEqual(kinds(f), ["start", "iteration", "not_submitted", "iteration", "not_submitted"]);
  });
});

test("an interrupt in the terminal pauses autopilot, whoever started the turn", async t => {
  const cases = {
    "Escape during an iteration": async f => run(f, { tools: 1, ...interrupted }),
    "Escape during a person's turn": async f => {
      await run(f, { tools: 1 });
      await run(f, { own: false, ...interrupted });
    },
    // The message that interrupted is handled by a run that follows by itself.
    "a message that interrupts the turn": async f => {
      await run(f, { ...interrupted, end: { willContinue: true } });
      assert.equal(condition(f), "paused:interrupted");
      await run(f, { own: false });
    },
    // The turn's last message keeps the stop reason error; it is not a failed iteration.
    "Escape while the agent waits to retry": async f => {
      await begin(f);
      await f.emit("turn_end", { message: { role: "assistant", ...failing } });
      await f.emit("auto_retry_start", { attempt: 1, maxAttempts: 10, delayMs: 400 });
      await f.emit("auto_retry_end", { success: false, attempt: 1, finalError: "Retry cancelled" });
      await finish(f, { stopReason: null });
    },
    "Escape during an automatic compaction": async f => {
      await begin(f);
      await f.emit("turn_end", { message: { role: "assistant", stopReason: "stop" } });
      await f.emit("auto_compaction_start", { reason: "threshold", action: "handoff" });
      await f.emit("auto_compaction_end", { action: "handoff", aborted: true, willRetry: false });
      await finish(f, { stopReason: null });
    },
    // The compaction ran before the prompt's run: OMP then drops the prompt.
    "an iteration dropped before its turn begins": async f => {
      await f.emit("auto_compaction_start", { reason: "threshold", action: "handoff" });
      await f.emit("auto_compaction_end", { action: "handoff", aborted: true, willRetry: false });
      f.omp.calls[0].reject(dropped());
      await f.advance(0);
    },
  };
  for (const [name, interrupt] of Object.entries(cases)) {
    await t.test(name, async t => {
      const f = await tab(t);
      await start(f, { iteration_limit: atMost(9) });
      await interrupt(f);
      assert.equal(condition(f), "paused:interrupted");
      assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused: the turn was interrupted");
      const sent = f.omp.calls.length;
      await jump(f, 600000);
      await f.advance(5000);
      assert.equal(f.omp.calls.length, sent);
      assert.equal(activity(f), "idle");
      // Not counted as failed: after a resume, two failures do not pause.
      await send(f, { kind: "resume", autopilot_id: f.state().autopilot.autopilot_id });
      for (let failure = 0; failure < 2; failure++) {
        await run(f, failing);
        assert.equal(condition(f), "waiting:next_iteration");
        await f.advance(60000);
      }
    });
  }
  await t.test("a retry that fails, or a compaction that completes, is not an interrupt", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    await f.emit("turn_end", { message: { role: "assistant", ...failing } });
    await f.emit("auto_retry_end", { success: false, attempt: 10, finalError: "429 usage limit reached" });
    await f.emit("auto_compaction_end", { action: "handoff", aborted: false, willRetry: false });
    await finish(f, { stopReason: null });
    assert.equal(condition(f), "waiting:next_iteration");
  });
  await t.test("Escape between iterations interrupts nothing", async t => {
    const f = await tab(t);
    await start(f);
    await run(f, { tools: 1 });
    // No event reaches the extension: Pause and Stop are the controls for that.
    await f.advance(5000);
    assert.equal(f.omp.calls.length, 2);
  });
});

const entry = data => ({
  type: "custom", customType: "apas-autopilot", id: crypto.randomUUID().slice(0, 8),
  data: { v: 1, session_id: "native-session", ...data },
});

test("after an agent restart the journaled autopilot comes back paused and never resumes by itself", async t => {
  const id = crypto.randomUUID();
  const limits = { repeat: { kind: "every", seconds: 900 }, iteration_limit: atMost(20), time_limit: atMost(28800) };
  const started = { autopilot_id: id, kind: "start", at: "2026-11-13T12:00:00.000Z", spec: spec(limits), started_at: "2026-11-13T12:00:00.000Z", deadline: "2026-11-13T20:00:00.000Z" };
  const journal = [
    { type: "model_change", id: "m1" },
    entry(started),
    entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T12:00:00.000Z", iteration: 1 }),
    { type: "message", id: "m2", message: { role: "user", attribution: "agent" } },
    { type: "message", id: "m3", message: { role: "assistant" } },
    entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T13:50:00.000Z", iteration: 2 }),
    // A submission OMP did not accept cancels the entry it follows.
    entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T13:58:00.000Z", iteration: 3 }),
    entry({ autopilot_id: id, kind: "not_submitted", at: "2026-11-13T13:58:01.000Z", iteration: 3 }),
    // OMP appends custom entries of its own, and a fork copies another session's.
    { type: "custom", customType: "session_exit", id: "m4", data: {} },
    { ...entry({ autopilot_id: crypto.randomUUID(), kind: "end", at: "2026-11-13T13:59:00.000Z", iterations: 9, reason: "stopped" }), data: { v: 1, session_id: "another-session", autopilot_id: crypto.randomUUID(), kind: "end", iterations: 9, reason: "stopped" } },
  ];
  const f = await tab(t, { journal });
  assert.deepEqual(f.state().autopilot, {
    autopilot_id: id, spec: spec(limits), started_at: "2026-11-13T12:00:00.000Z", iterations: 2,
    deadline: "2026-11-13T20:00:00.000Z", condition: { kind: "paused", detail: "agent_restarted", since: iso(f.clock.wall) },
  });
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused: the agent was restarted");
  assert.deepEqual(f.journal().at(-1), { v: 1, autopilot_id: id, session_id: "native-session", kind: "pause", at: iso(f.clock.wall), iterations: 2, reason: "agent_restarted" });
  await jump(f, 3600000);
  await f.advance(5000);
  assert.equal(f.omp.calls.length, 0);
  // Resumed, it continues from its count; the interval since its last submission has passed.
  await send(f, { kind: "resume", autopilot_id: id });
  assert.equal(f.omp.calls.length, 1);
  assert.equal(f.state().autopilot.iterations, 3);
  assert.equal(f.journal().at(-1).iteration, 3);
  // Its time limit was armed again, with the same end.
  await begin(f);
  await jump(f, Date.parse(started.deadline) - f.clock.wall);
  assert.deepEqual([f.state().last_ended.reason, f.state().last_ended.ended_at], ["time_limit", started.deadline]);

  await t.test("it is ended when its time limit passed meanwhile", async t => {
    const passed = { ...started, deadline: "2026-11-13T13:00:00.000Z" };
    const f = await tab(t, { journal: [entry(passed), entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T12:00:00.000Z", iteration: 1 })] });
    assert.equal(f.state().autopilot, undefined);
    assert.deepEqual(f.state().last_ended, {
      autopilot_id: id, spec: spec(limits), started_at: started.started_at, iterations: 1, reason: "time_limit",
      ended_at: passed.deadline, turn_since: false,
    });
    assert.deepEqual(plain(f.journal().at(-1)), { v: 1, autopilot_id: id, session_id: "native-session", kind: "end", at: passed.deadline, iterations: 1, reason: "time_limit" });
    assert.equal(f.result(await send(f, { kind: "resume", autopilot_id: id })).reason, "This autopilot is no longer current.");
  });
  await t.test("an ended one is reported as the last autopilot, for a new start", async t => {
    const ended = [
      entry(started), entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T12:00:00.000Z", iteration: 1 }),
      entry({ autopilot_id: id, kind: "end", at: "2026-11-13T12:30:00.000Z", iterations: 1, reason: "no_tool_call" }),
    ];
    const f = await tab(t, { journal: ended });
    assert.deepEqual(f.state().last_ended, {
      autopilot_id: id, spec: spec(limits), started_at: started.started_at, iterations: 1, reason: "no_tool_call",
      ended_at: "2026-11-13T12:30:00.000Z", turn_since: false,
    });
    assert.equal(f.journal().length, 3);
    const later = await tab(t, { journal: [...ended, { type: "message", id: "m9", message: { role: "assistant" } }] });
    assert.equal(later.state().last_ended.turn_since, true);
    assert.equal(later.omp.statuses.has("apas-autopilot"), false);
  });
  await t.test("one that was already paused gets no second pause entry", async t => {
    const paused = [entry(started), entry({ autopilot_id: id, kind: "pause", at: "2026-11-13T12:10:00.000Z", iterations: 0, reason: "requested" })];
    const f = await tab(t, { journal: paused });
    assert.equal(condition(f), "paused:agent_restarted");
    assert.equal(f.journal().length, 2);
  });
  await t.test("a late entry of an earlier autopilot does not hide the one that started after it", async t => {
    const later = crypto.randomUUID();
    // The first was stopped with its prompt still outstanding; OMP dropped that prompt after the second had started.
    const f = await tab(t, { journal: [
      entry(started), entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T12:00:00.000Z", iteration: 1 }),
      entry({ autopilot_id: id, kind: "end", at: "2026-11-13T12:00:05.000Z", iterations: 1, reason: "stopped" }),
      entry({ autopilot_id: later, kind: "start", at: "2026-11-13T12:00:06.000Z", spec: spec(), started_at: "2026-11-13T12:00:06.000Z" }),
      entry({ autopilot_id: id, kind: "not_submitted", at: "2026-11-13T12:00:07.000Z", iteration: 1 }),
    ] });
    assert.deepEqual([f.state().autopilot.autopilot_id, f.state().autopilot.iterations, condition(f)], [later, 0, "paused:agent_restarted"]);
    assert.equal(f.state().last_ended, undefined);
  });
  await t.test("a journal this build cannot read starts nothing", async t => {
    for (const broken of [
      [entry({ ...started, spec: { ...spec(limits), token_limit: 5 } })],
      [entry({ ...started, started_at: "yesterday" })],
      [entry({ ...started, autopilot_id: "not-a-uuid" })],
      [entry({ autopilot_id: id, kind: "iteration", at: "2026-11-13T12:00:00.000Z", iteration: 4 })],
    ]) {
      const f = await tab(t, { journal: broken });
      assert.equal(f.state().autopilot, undefined);
      assert.equal(f.state().last_ended, undefined);
      assert.equal(f.journal().length, 1);
    }
  });
});

test("a stop that arrives before OMP's version is known waits for the autopilot that is restored", async t => {
  const id = crypto.randomUUID();
  const at = "2026-11-13T12:00:00.000Z";
  let known;
  const f = await fixture(t, {
    journal: [entry({ autopilot_id: id, kind: "start", at, spec: spec(), started_at: at })],
    importAfter: new Promise(resolve => { known = resolve; }),
  });
  f.controller.alive = true;
  f.beat();
  const stopped = f.stop("stop");
  await f.advance(3000);
  // Nothing is restored yet: answering now would miss the autopilot that comes back paused.
  assert.equal(f.exists("omp-autopilot.json"), false);
  assert.equal(f.exists("omp-autopilot-ack.json"), false);
  known();
  await f.flush();
  assert.equal(condition(f), "paused:agent_restarted");
  await f.advance(1000);
  assert.equal(f.result(stopped).outcome, "accepted");
  assert.equal(f.state().autopilot, undefined);
  assert.deepEqual([f.state().last_ended.autopilot_id, f.state().last_ended.reason], [id, "stopped"]);
  assert.deepEqual(kinds(f), ["start", "pause", "end"]);
});

test("leaving its conversation ends autopilot, journaled where it will be read back", async t => {
  await t.test("a new conversation, and back", async t => {
    const f = await tab(t);
    await start(f, { iteration_limit: atMost(20) });
    await run(f, { tools: 1 });
    const id = f.state().autopilot.autopilot_id;
    const left = f.ctx.sessionManager.getSessionFile();
    await f.emit("session_before_switch", { reason: "new" });
    // An entry appended here lands in the conversation being left.
    assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
    assert.deepEqual([f.journal().at(-1).reason, f.journal().at(-1).iterations], ["conversation_changed", 1]);
    f.switchSession("fresh-session");
    await f.emit("session_switch", { reason: "new", previousSessionFile: left });
    const report = f.state();
    assert.deepEqual([report.session_id, report.autopilot], ["fresh-session", undefined]);
    // Its values stay offered for a new start until another starts or the agent restarts.
    assert.deepEqual([report.last_ended.autopilot_id, report.last_ended.reason, report.last_ended.spec],
      [id, "conversation_changed", spec({ iteration_limit: atMost(20) })]);
    assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Ended: the conversation was changed in the terminal");
    assert.deepEqual(f.journal(), []);
    await f.advance(60000);
    assert.equal(f.omp.calls.length, 1);
    // Back with /resume: the conversation shows its autopilot as ended; nothing resumes.
    await f.emit("session_before_switch", { reason: "resume", targetSessionFile: left });
    const fresh = f.ctx.sessionManager.getSessionFile();
    f.switchSession("native-session");
    await f.emit("session_switch", { reason: "resume", previousSessionFile: fresh });
    assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
    assert.equal(f.state().last_ended.autopilot_id, id);
    await f.advance(60000);
    assert.equal(f.omp.calls.length, 1);
    // Another start replaces what is reported.
    await start(f);
    assert.equal(f.state().last_ended, undefined);
    assert.notEqual(f.state().autopilot.autopilot_id, id);
  });
  await t.test("a /btw answer promoted to a branch", async t => {
    const f = await tab(t);
    await start(f);
    await run(f, { tools: 1 });
    const left = f.ctx.sessionManager.getSessionFile();
    await f.emit("session_before_branch", { reason: "btw", entryId: "e1" });
    // OMP refuses the branch when a handler appended an entry here.
    assert.deepEqual(kinds(f), ["start", "iteration"]);
    assert.equal(condition(f), "waiting:next_iteration");
    // The new session holds the old branch's entries, still under the old session's id.
    const copied = structuredClone(f.branch());
    f.switchSession("btw-session");
    f.omp.branches.set("btw-session", copied);
    await f.emit("session_branch", { reason: "btw", previousSessionFile: left });
    assert.equal(f.state().autopilot, undefined);
    assert.equal(f.state().last_ended.reason, "conversation_changed");
    assert.equal(f.branch().length, 2);
    await f.advance(60000);
    assert.equal(f.omp.calls.length, 1);
  });
  await t.test("a fork, a branch and a move in the session tree", async t => {
    for (const [name, event] of [
      ["session_before_switch", { reason: "fork" }], ["session_before_branch", { reason: "fork", entryId: "e1" }],
      ["session_before_branch", { reason: "branch", entryId: "e1" }], ["session_before_tree", { preparation: {} }],
    ]) {
      const f = await tab(t);
      await start(f);
      await run(f, { tools: 1 });
      await f.emit(name, event);
      assert.deepEqual(kinds(f), ["start", "iteration", "end"], name);
      assert.equal(f.state().last_ended.reason, "conversation_changed");
      if (name !== "session_before_tree") continue;
      // The entry stays on the branch being left. Where the rewound position
      // still shows a running autopilot, it is ended there too.
      f.branch().pop();
      await f.emit("session_tree", { newLeafId: "e1", oldLeafId: "e2" });
      assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
      assert.equal(f.journal().at(-1).reason, "conversation_changed");
      await f.advance(60000);
      assert.equal(f.omp.calls.length, 1);
    }
  });
  await t.test("arriving where the journal still shows a running autopilot", async t => {
    const f = await tab(t);
    const id = crypto.randomUUID();
    const here = f.ctx.sessionManager.getSessionFile();
    f.omp.branches.set("older-session", [
      { type: "custom", customType: "apas-autopilot", id: "o1", data: { v: 1, session_id: "older-session", autopilot_id: id, kind: "start", at: "2026-11-12T10:00:00.000Z", spec: spec(), started_at: "2026-11-12T10:00:00.000Z" } },
      { type: "custom", customType: "apas-autopilot", id: "o2", data: { v: 1, session_id: "older-session", autopilot_id: id, kind: "iteration", at: "2026-11-12T10:00:00.000Z", iteration: 1 } },
    ]);
    await f.emit("session_before_switch", { reason: "resume", targetSessionFile: path.join(f.sessions, "older-session.jsonl") });
    f.switchSession("older-session");
    await f.emit("session_switch", { reason: "resume", previousSessionFile: here });
    assert.deepEqual(kinds(f), ["start", "iteration", "end"]);
    assert.deepEqual(plain(f.journal().at(-1)), { v: 1, autopilot_id: id, session_id: "older-session", kind: "end", at: iso(f.clock.wall), iterations: 1, reason: "conversation_changed" });
    assert.equal(f.state().autopilot, undefined);
    assert.deepEqual([f.state().last_ended.autopilot_id, f.state().last_ended.reason], [id, "conversation_changed"]);
  });
  await t.test("one that had ended by itself stays reported until another conversation shows its own", async t => {
    const f = await tab(t);
    await start(f, { iteration_limit: atMost(1) });
    await run(f, { tools: 1 });
    const first = f.state().last_ended;
    assert.equal(first.reason, "iteration_limit");
    const left = f.ctx.sessionManager.getSessionFile();
    await f.emit("session_before_switch", { reason: "new" });
    f.switchSession("fresh-session");
    await f.emit("session_switch", { reason: "new", previousSessionFile: left });
    assert.deepEqual(f.state().last_ended, first);
    assert.deepEqual(f.journal(), []);
    // A conversation with an ended autopilot of its own shows that one.
    const other = crypto.randomUUID();
    f.omp.branches.set("older-session", [
      { type: "custom", customType: "apas-autopilot", id: "o1", data: { v: 1, session_id: "older-session", autopilot_id: other, kind: "start", at: "2026-11-12T10:00:00.000Z", spec: spec({ prompt: "older" }), started_at: "2026-11-12T10:00:00.000Z" } },
      { type: "custom", customType: "apas-autopilot", id: "o2", data: { v: 1, session_id: "older-session", autopilot_id: other, kind: "end", at: "2026-11-12T11:00:00.000Z", iterations: 0, reason: "stopped" } },
    ]);
    const fresh = f.ctx.sessionManager.getSessionFile();
    f.switchSession("older-session");
    await f.emit("session_switch", { reason: "resume", previousSessionFile: fresh });
    assert.deepEqual([f.state().last_ended.autopilot_id, f.state().last_ended.reason, f.state().last_ended.spec.prompt], [other, "stopped", "older"]);
    assert.equal(f.journal().length, 2);
  });
  await t.test("a reload keeps the autopilot and resets the outstanding submission", async t => {
    const f = await tab(t);
    await start(f);
    const id = f.state().autopilot.autopilot_id;
    const file = f.ctx.sessionManager.getSessionFile();
    await f.emit("session_before_switch", { reason: "resume", targetSessionFile: file });
    await f.emit("session_switch", { reason: "resume", previousSessionFile: file });
    assert.equal(f.state().autopilot.autopilot_id, id);
    assert.deepEqual(kinds(f), ["start", "iteration"]);
    assert.equal(condition(f), "waiting:next_iteration");
    // The queued prompt was discarded without its call rejecting.
    await f.advance(5000);
    assert.equal(f.omp.calls.length, 2);
    f.omp.calls[0].reject(dropped());
    await f.advance(0);
    assert.equal(condition(f), "running");
    assert.equal(f.state().autopilot.iterations, 2);
  });
});

test("autopilot holds without its controller, through a long outage and on a server that cannot control it", async t => {
  // Nothing comes due while the hold begins, so the tab is seen waiting.
  const timed = { repeat: { kind: "every", seconds: 120 } };
  await t.test("the controller is gone for more than a minute, then replaced", async t => {
    const f = await tab(t);
    await start(f, timed);
    await run(f, { tools: 1 });
    f.controller.alive = false;
    await f.advance(56000);
    assert.equal(condition(f), "waiting:next_iteration");
    await f.advance(8000);
    assert.equal(condition(f), "holding:controller");
    assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · iteration 1 · holding until APAS can control this tab");
    assert.equal(activity(f), "idle");
    // The iteration that came due meanwhile is not submitted.
    await f.advance(120000);
    assert.equal(f.omp.calls.length, 1);
    // A replaced CLI process writes a new id and starts its counter again:
    // compared for difference, never for order.
    Object.assign(f.controller, { writer: crypto.randomUUID(), counter: 0, alive: true });
    await f.advance(3000);
    assert.equal(f.omp.calls.length, 2);
    assert.equal(condition(f), "running");
  });
  await t.test("a heartbeat of another launch is not this tab's controller", async t => {
    const f = await tab(t);
    await start(f, timed);
    await run(f, { tools: 1 });
    f.controller.report = { launch_id: crypto.randomUUID() };
    await f.advance(70000);
    assert.equal(condition(f), "holding:controller");
  });
  await t.test("a turn in progress is not stopped and keeps its condition", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    f.controller.alive = false;
    await f.advance(70000);
    assert.equal(condition(f), "running");
    assert.equal(f.omp.aborts, 0);
    await finish(f, { tools: 1 });
    assert.equal(condition(f), "holding:controller");
    await f.advance(30000);
    assert.equal(f.omp.calls.length, 1);
  });
  await t.test("the server is unreachable for more than ten minutes, or cannot control autopilot", async t => {
    const f = await tab(t);
    await start(f);
    await run(f, { tools: 1 });
    // A short outage of a server that could control the tab holds nothing.
    f.controller.report = { connected: false, unreachable_secs: 600 };
    f.beat();
    await f.advance(5000);
    assert.equal(f.omp.calls.length, 2);
    await run(f, { tools: 1 });
    f.controller.report = { connected: false, unreachable_secs: 601 };
    f.beat();
    await f.advance(20000);
    assert.equal(f.omp.calls.length, 2);
    assert.equal(condition(f), "holding:server");
    assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · iteration 2 · holding until the APAS server can control this tab");
    // Iterations continue as before when the hold ends.
    f.controller.report = {};
    f.beat();
    await f.advance(1000);
    assert.equal(f.omp.calls.length, 3);
    await run(f, { tools: 1 });
    // A server that was rolled back keeps the CLI connected but relays nothing.
    f.controller.report = { server_controls_autopilot: false };
    f.beat();
    await f.advance(20000);
    assert.equal(f.omp.calls.length, 3);
    assert.equal(condition(f), "holding:server");
    // That server going away ends nothing: the hold stays until a server
    // that can control the tab has registered, however short the outage.
    f.controller.report = { connected: false, unreachable_secs: 15, server_controls_autopilot: false };
    f.beat();
    await f.advance(20000);
    assert.equal(f.omp.calls.length, 3);
    assert.equal(condition(f), "holding:server");
    f.controller.report = {};
    f.beat();
    await f.advance(1000);
    assert.equal(f.omp.calls.length, 4);
    await run(f, { tools: 1 });
    // A heartbeat that does not say for how long the server has been gone cannot show a short outage.
    f.controller.report = { connected: false, unreachable_secs: undefined };
    f.beat();
    await f.advance(20000);
    assert.equal(f.omp.calls.length, 4);
    assert.equal(condition(f), "holding:server");
  });
  await t.test("a hold that lasts four hours becomes a pause", async t => {
    const f = await tab(t);
    await start(f, timed);
    await run(f, { tools: 1 });
    f.controller.alive = false;
    await f.advance(65000);
    assert.equal(condition(f), "holding:controller");
    await jump(f, 4 * 3600000 - 10000);
    assert.equal(condition(f), "holding:controller");
    await f.advance(10000);
    assert.equal(condition(f), "paused:no_control");
    assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused: APAS could not control this tab for more than four hours");
    // Contact returns: work does not start again by itself.
    f.controller.alive = true;
    await jump(f, 3600000);
    await f.advance(10000);
    assert.equal(f.omp.calls.length, 1);
    assert.equal(f.journal().at(-1).reason, "no_control");
  });
  await t.test("a hold that ends within four hours is not a pause", async t => {
    const f = await tab(t);
    await start(f, timed);
    await run(f, { tools: 1 });
    f.controller.alive = false;
    await f.advance(65000);
    await jump(f, 3 * 3600000);
    assert.equal(condition(f), "holding:controller");
    f.controller.alive = true;
    await f.advance(3000);
    assert.equal(f.omp.calls.length, 2);
    // A later hold is counted from its own start.
    await run(f, { tools: 1 });
    f.controller.alive = false;
    await f.advance(65000);
    await jump(f, 3 * 3600000);
    assert.equal(condition(f), "holding:controller");
  });
  await t.test("a hold between two timed iterations", async t => {
    const f = await tab(t);
    await start(f, { repeat: { kind: "every", seconds: 21600 } });
    const midnight = f.omp.calls[0].at;
    await run(f, { tools: 1 });
    // 01:00: the machine loses the server. Its CLI stays and says for how long.
    await jump(f, midnight + 3600000 - f.clock.wall);
    f.controller.report = { connected: false, unreachable_secs: 5 };
    f.beat();
    await f.advance(1000);
    assert.equal(condition(f), "waiting:next_iteration");
    // 01:10: held, although no iteration is due.
    await jump(f, 600000);
    f.controller.report = { connected: false, unreachable_secs: 605 };
    f.beat();
    await f.advance(1000);
    assert.equal(condition(f), "holding:server");
    // 05:10: paused.
    f.controller.report = { connected: false, unreachable_secs: 15000 };
    await jump(f, 4 * 3600000 - 5000);
    assert.equal(condition(f), "holding:server");
    await f.advance(5000);
    assert.equal(condition(f), "paused:no_control");
    // 05:30: reconnected. The iteration due at 06:00 is not submitted.
    f.controller.report = {};
    await jump(f, midnight + 6 * 3600000 + 60000 - f.clock.wall);
    await f.advance(5000);
    assert.equal(f.omp.calls.length, 1);
    assert.equal(condition(f), "paused:no_control");
  });
});

test("the state report is rewritten on every change and at least every thirty seconds, within its size bound", async t => {
  const f = await tab(t);
  const first = f.state().counter;
  // Nothing changed: the same report is left alone, then refreshed.
  await f.advance(29000);
  assert.equal(f.state().counter, first);
  await f.advance(1000);
  assert.equal(f.state().counter, first + 1);
  // The largest prompt, of characters that double when encoded, and text of OMP's own beside it.
  const quotes = '"'.repeat(16000);
  f.omp.modes.loop = { status: "paused" };
  f.omp.modes.goal = { status: "paused", objective: "😀".repeat(600), tokenBudget: 1.5 };
  await f.emit("mode_changed", {});
  const objective = f.state().omp_modes.goal.objective;
  // Cut by code point, with a marker, so the report stays readable to the CLI.
  assert.deepEqual([Array.from(objective).length, objective.at(-1), objective.isWellFormed()], [500, "…", true]);
  assert.deepEqual(f.state().omp_modes, { goal: { status: "paused", objective }, loop: { status: "paused" } });
  f.omp.modes.goal = { status: "inactive" };
  f.omp.modes.loop = { status: "inactive" };
  await f.emit("mode_changed", {});
  const counter = f.state().counter;
  await start(f, { prompt: quotes, iteration_limit: atMost(1000000), time_limit: atMost(31536000) });
  assert.ok(f.state().counter > counter);
  assert.equal(f.state().autopilot.spec.prompt, quotes);
  assert.ok(fs.statSync(path.join(f.runtime, "omp-autopilot.json")).size <= 65536);
  await begin(f);
  await stop(f, "stop");
  assert.equal(f.state().last_ended.spec.prompt, quotes);
  assert.ok(fs.statSync(path.join(f.runtime, "omp-autopilot.json")).size <= 65536);
  // A report that cannot be written is tried again, with a counter that still moves on.
  const before = f.state().counter;
  f.fail("omp-autopilot.json");
  await finish(f);
  await f.advance(40000);
  assert.equal(f.state().counter, before);
  f.fail(undefined);
  await f.advance(1000);
  assert.ok(f.state().counter > before);
  assert.equal(f.state().last_ended.turn_since, false);
});

test("the terminal's status line shows the condition", async t => {
  const f = await tab(t);
  const line = () => f.omp.statuses.get("apas-autopilot");
  assert.equal(line(), undefined);
  await start(f, { repeat: { kind: "every", seconds: 900 }, iteration_limit: atMost(20) });
  assert.equal(line(), "Autopilot · iteration 1 of 20");
  await run(f, { tools: 1 });
  const next = new Date(Date.parse(f.state().autopilot.next_iteration_at));
  const two = value => String(value).padStart(2, "0");
  assert.equal(line(), `Autopilot · iteration 1 of 20 · next ${two(next.getHours())}:${two(next.getMinutes())}`);
  await begin(f, { own: false });
  assert.equal(line(), "Autopilot · iteration 1 of 20 · waiting for the tab to settle");
  await finish(f);
  await stop(f, "stop");
  assert.equal(line(), "Autopilot · Stopped");
});

test("OMP's own goal and loop modes are reported, and autopilot stays off a tab they drive", async t => {
  const f = await tab(t);
  const refusals = {
    goal: "This tab is in OMP's own goal mode. Stop that first.",
    loop: "This tab is in OMP's own loop mode. Stop that first.",
    plan: "This tab is in OMP's plan mode. Leave plan mode first.",
  };
  const autoresearch = "This tab is running OMP's autoresearch. Type /autoresearch off in the terminal first.";
  async function mode(name, value) {
    f.omp.modes[name] = value;
    await f.emit("mode_changed", { state: f.omp.modes });
  }
  assert.equal(f.state().omp_modes, undefined);
  // getState() gives the status, the objective and the budget.
  await mode("goal", { status: "active", objective: "Ship the release", tokenBudget: 500000 });
  assert.deepEqual(f.state().omp_modes, { goal: { status: "active", objective: "Ship the release", token_budget: 500000 } });
  // Tokens used and the real status come only with goal_updated.
  await f.emit("goal_updated", { goal: { id: "g1", objective: "Ship the release", status: "budget_limited", tokensUsed: 12303 } });
  assert.deepEqual(f.state().omp_modes.goal, {
    status: "active", objective: "Ship the release", token_budget: 500000, tokens_used: 12303, goal_status: "budget_limited",
  });
  await mode("goal", { status: "paused", objective: "Ship the release" });
  assert.equal(f.state().omp_modes.goal.status, "paused");
  // A goal that is gone takes what was known about it along.
  await mode("goal", { status: "inactive" });
  assert.equal(f.state().omp_modes, undefined);
  await mode("goal", { status: "active", objective: "Another" });
  assert.deepEqual(f.state().omp_modes, { goal: { status: "active", objective: "Another" } });
  await mode("goal", { status: "inactive" });
  await mode("loop", { status: "active" });
  assert.deepEqual(f.state().omp_modes, { loop: { status: "active" } });
  await mode("loop", { status: "inactive" });
  // Plan mode and autoresearch are not shown.
  await mode("plan", { status: "active", planFilePath: "local://PLAN.md" });
  assert.equal(f.state().omp_modes, undefined);
  await mode("plan", { status: "inactive" });

  // Any state other than off refuses a start: /plan typed again only pauses plan mode.
  for (const [name, reason] of Object.entries(refusals)) {
    for (const status of ["active", "paused"]) {
      await mode(name, { status });
      const id = await start(f);
      assert.deepEqual([f.result(id).outcome, f.result(id).reason], ["refused", reason], `${name} ${status}`);
    }
    await mode(name, { status: "inactive" });
  }
  f.omp.tools = ["read", "init_experiment", "run_experiment"];
  assert.equal(f.result(await start(f)).reason, autoresearch);
  f.omp.tools = ["read", "bash"];
  assert.equal(f.omp.calls.length, 0);

  // Switched on while the tab is on autopilot, each pauses it; a resume is refused meanwhile.
  await start(f);
  const id = f.state().autopilot.autopilot_id;
  await run(f, { tools: 1 });
  for (const name of ["goal", "loop", "plan"]) {
    await mode(name, { status: "active" });
    assert.equal(condition(f), `paused:omp_${name}_mode`);
    const resumed = await send(f, { kind: "resume", autopilot_id: id });
    assert.equal(f.result(resumed).reason, refusals[name]);
    await mode(name, { status: "inactive" });
    assert.equal(f.result(await send(f, { kind: "resume", autopilot_id: id })).outcome, "accepted");
    await f.advance(5000);
    await run(f, { tools: 1 });
  }
  assert.match(f.omp.statuses.get("apas-autopilot"), /^Autopilot · iteration 4 · next \d\d:\d\d$/);
  // Autoresearch emits no event: its experiment tool is seen before the next submission.
  f.omp.tools = ["read", "init_experiment"];
  await f.advance(5000);
  assert.equal(condition(f), "paused:omp_autoresearch");
  assert.equal(f.omp.statuses.get("apas-autopilot"), "Autopilot · Paused: OMP's autoresearch was switched on");
  assert.equal(f.result(await send(f, { kind: "resume", autopilot_id: id })).reason, autoresearch);
  assert.equal(f.omp.calls.length, 4);
  assert.deepEqual(f.journal().filter(entry => entry.kind === "pause").map(entry => entry.reason),
    ["omp_goal_mode", "omp_loop_mode", "omp_plan_mode", "omp_autoresearch"]);
});

test("a tab on autopilot stays Working while its next iteration is close, and Idle when it is not", async t => {
  await t.test("as soon as each turn ends: no Idle between two iterations", async t => {
    const f = await tab(t);
    await start(f);
    await run(f, { tools: 1 });
    f.omp.activities.length = 0;
    await f.advance(5000);
    await run(f, { tools: 1 });
    await f.advance(5000);
    await run(f, { tools: 1 });
    assert.equal(f.omp.calls.length, 3);
    assert.equal(activity(f), "working");
    assert.equal(f.omp.activities.includes("idle"), false);
    // Paused, the tab needs someone and is idle.
    await stop(f, "pause");
    assert.equal(activity(f), "idle");
  });
  await t.test("a submission whose run does not start falls back to Idle after 15 seconds", async t => {
    const f = await tab(t);
    await start(f);
    assert.equal(activity(f), "working");
    await f.advance(14000);
    assert.equal(activity(f), "working");
    await f.advance(2000);
    assert.equal(activity(f), "idle");
    assert.equal(condition(f), "running");
    await begin(f);
    assert.equal(activity(f), "working");
  });
  await t.test("a later iteration: Idle until it is ten seconds away", async t => {
    const f = await tab(t);
    await start(f, { repeat: { kind: "every", seconds: 600 } });
    const first = f.omp.calls[0].at;
    await run(f, { tools: 1 });
    assert.equal(activity(f), "idle");
    assert.equal(condition(f), "waiting:next_iteration");
    await jump(f, first + 600000 - 12000 - f.clock.wall);
    assert.equal(activity(f), "idle");
    await f.advance(2000);
    assert.equal(activity(f), "working");
    await f.advance(10000);
    assert.equal(f.omp.calls.length, 2);
    assert.equal(activity(f), "working");
  });
  await t.test("a held tab is Idle", async t => {
    const f = await tab(t);
    await start(f);
    await run(f, { tools: 1 });
    f.controller.report = { server_controls_autopilot: false };
    f.beat();
    await f.advance(3000);
    assert.equal(condition(f), "holding:server");
    assert.equal(activity(f), "idle");
  });
  await t.test("one of OMP's own modes continues after a gap", async t => {
    const f = await tab(t);
    f.omp.modes.goal = { status: "active", objective: "Ship it" };
    await f.emit("mode_changed", {});
    assert.equal(activity(f), "idle");
    await run(f, { own: false });
    f.omp.activities.length = 0;
    // OMP submits again about 0.8 s after the turn, without an event in between.
    await f.advance(1000);
    await run(f, { own: false });
    assert.equal(f.omp.activities.includes("idle"), false);
    // A goal that hit its budget still reads active: no run starts, and the tab is idle.
    await f.advance(14000);
    assert.equal(activity(f), "working");
    await f.advance(2000);
    assert.equal(activity(f), "idle");
    // A paused mode is not continuing.
    f.omp.modes.goal = { status: "inactive" };
    f.omp.modes.loop = { status: "paused" };
    await f.emit("mode_changed", {});
    await run(f, { own: false });
    assert.equal(activity(f), "idle");
    // And a mode that is switched off during its gap does not keep the tab Working.
    f.omp.modes.loop = { status: "active" };
    await f.emit("mode_changed", {});
    await run(f, { own: false });
    assert.equal(activity(f), "working");
    f.omp.modes.loop = { status: "inactive" };
    await f.emit("mode_changed", {});
    assert.equal(activity(f), "idle");
  });
});

test("a fault inside autopilot leaves activity reporting and the question bridge working", async t => {
  const faults = {
    "its setup throws": [{ prepare: ({ ctx }) => { ctx.setInterval = () => { throw new Error("injected timer failure"); }; } }],
    "OMP does not know its events": [{ prepare: ({ api }) => {
      const on = api.on;
      api.on = (name, callback) => {
        if (/^(mode_changed|goal_updated|turn_end|message_start|session_before_)/.test(name)) throw new Error(`unknown event ${name}`);
        on(name, callback);
      };
    } }],
    "the status line throws": [{}, f => { f.omp.throws.setStatus = true; }],
    "OMP's mode state throws": [{}, f => { f.omp.throws.modes = true; }],
    "the journal throws": [{}, f => { f.omp.throws.append = true; }],
    "the state report cannot be written": [{}, f => f.fail("omp-autopilot.json")],
    "the acknowledgement cannot be written": [{}, f => f.fail("omp-autopilot-ack.json")],
    "the job snapshot throws": [{}, f => { f.omp.throws.jobs = true; }, { willContinue: true, awaitingAsyncWork: true }],
  };
  for (const [name, [config, arrange, end]] of Object.entries(faults)) {
    await t.test(name, async t => {
      const f = await tab(t, config);
      arrange?.(f);
      f.command({ kind: "start", spec: spec() });
      await f.advance(2000);
      assert.equal(activity(f), f.omp.calls.length > 0 ? "working" : "idle");
      await begin(f);
      assert.equal(activity(f), "working");
      // A question is asked and answered from the web during the turn.
      const { result } = await f.start();
      assert.equal(activity(f), "pending_answer");
      const request = f.request();
      await f.tick();
      assert.deepEqual(plain(await result), { kind: "submit", results: answers });
      assert.equal(f.read("omp-question-ack.json").nonce, request.nonce);
      await f.emit("tool_execution_end", { toolName: "ask", toolCallId: "call-native" });
      assert.equal(activity(f), "working");
      await finish(f, end ? { end } : {});
      if (end) {
        // Waiting for background work is Working, as before autopilot; the poll's fault changes nothing.
        await f.advance(20000);
        assert.equal(activity(f), "working");
        f.omp.idle = false;
        await f.emit("agent_start");
        await finish(f);
      }
      f.stop("stop");
      await f.advance(20000);
      assert.equal(activity(f), "idle");
      assert.deepEqual(Object.keys(f.read("omp-activity.json")).sort(),
        ["activity", "launch_id", "pid", "process_start", "session_id", "transcript_path"]);
      // Without its poll autopilot stays off altogether: nothing claims a state it cannot serve.
      if (name === "its setup throws") assert.equal(f.exists("omp-autopilot.json"), false);
    });
  }
});

test("what a stop leaves behind does not leak into what follows", async t => {
  await t.test("a prompt OMP drops after its autopilot was stopped", async t => {
    const f = await tab(t);
    await start(f);
    const first = f.state().autopilot.autopilot_id;
    await stop(f, "stop");
    assert.equal(f.state().last_ended.iterations, 1);
    // Another autopilot waits for the tab's outstanding submission like any other.
    await start(f);
    await f.advance(20000);
    assert.equal(f.omp.calls.length, 1);
    assert.equal(condition(f), "starting");
    f.omp.editor = prompt;
    f.omp.calls[0].reject(dropped());
    await f.advance(0);
    await f.advance(1000);
    assert.equal(f.omp.editor, "");
    assert.equal(f.omp.calls.length, 2);
    assert.equal(condition(f), "running");
    // The entry cancels the stopped autopilot's iteration, so no later message pairs with it.
    assert.deepEqual(f.journal().filter(entry => entry.autopilot_id === first).map(entry => entry.kind),
      ["start", "iteration", "end", "not_submitted"]);
    // After a restart that conversation shows the first one as ended, without the dropped iteration.
    const again = await tab(t, { journal: f.branch().filter(entry => entry.data.autopilot_id === first) });
    assert.deepEqual([again.state().last_ended.reason, again.state().last_ended.iterations], ["stopped", 0]);
  });
  await t.test("a dropped prompt is not counted for the autopilot that was stopped meanwhile", async t => {
    const f = await tab(t);
    await start(f);
    await stop(f, "stop");
    assert.equal(f.state().last_ended.iterations, 1);
    f.omp.calls[0].reject(dropped());
    await f.advance(0);
    assert.equal(f.state().last_ended.iterations, 0);
    assert.deepEqual(kinds(f), ["start", "iteration", "end", "not_submitted"]);
  });
  await t.test("a pause during the last iteration still ends at the limit", async t => {
    const f = await tab(t);
    await start(f, { iteration_limit: atMost(1) });
    await begin(f);
    await stop(f, "pause");
    assert.equal(condition(f), "paused:requested");
    // A pause lets the turn in progress continue.
    assert.equal(f.omp.aborts, 0);
    await finish(f, { tools: 1 });
    assert.equal(f.state().last_ended.reason, "iteration_limit");
  });
  await t.test("a /btw answer promoted to a branch while Stop now waits for its turn", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    await stop(f, "stop_now");
    const left = f.ctx.sessionManager.getSessionFile();
    await f.emit("session_before_branch", { reason: "btw", entryId: "e1" });
    const copied = structuredClone(f.branch());
    f.switchSession("btw-session");
    f.omp.branches.set("btw-session", copied);
    await f.emit("session_branch", { reason: "btw", previousSessionFile: left });
    assert.equal(f.state().last_ended.reason, "stopped");
    // Nothing is appended to a conversation that holds none of its entries under its own id.
    assert.equal(f.branch().length, 2);
  });
  await t.test("a reload while Stop now waits for its turn", async t => {
    const f = await tab(t);
    await start(f);
    await begin(f);
    await stop(f, "stop_now");
    const file = f.ctx.sessionManager.getSessionFile();
    await f.emit("session_switch", { reason: "resume", previousSessionFile: file });
    assert.equal(f.state().last_ended.reason, "stopped");
  });
  await t.test("a time limit that passes while Stop now waits", async t => {
    const f = await tab(t, {});
    await start(f, { time_limit: atMost(60) });
    await begin(f);
    await stop(f, "stop_now");
    await jump(f, 60000);
    assert.equal(f.state().last_ended.reason, "stopped");
  });
});

test("events of a subagent, a start without a controller and a shutdown", async t => {
  const f = await fixture(t);
  await f.flush();
  // No heartbeat was ever written: the start is taken, and held from the start.
  const id = await start(f);
  assert.equal(f.result(id).outcome, "accepted");
  assert.equal(condition(f), "holding:controller");
  assert.equal(f.omp.calls.length, 0);
  f.controller.alive = true;
  await f.advance(3000);
  assert.equal(f.omp.calls.length, 1);
  const sub = { ...f.ctx, agent: { kind: "subagent" } };
  // A subagent's run is not the tab's: it neither records the submission nor interrupts anything.
  await f.emit("agent_start", {}, sub);
  await f.emit("message_start", { message: { role: "user", attribution: "agent", content: [{ type: "text", text: prompt }], timestamp: f.clock.wall } }, sub);
  await f.emit("turn_end", { message: { role: "assistant", ...interrupted } }, sub);
  await f.emit("auto_retry_end", { success: false, finalError: "Retry cancelled" }, sub);
  await f.emit("agent_end", {}, sub);
  await f.emit("session_before_switch", { reason: "new" }, sub);
  assert.equal(condition(f), "running");
  assert.deepEqual(kinds(f), ["start", "iteration"]);
  await run(f, { tools: 1 });
  assert.equal(condition(f), "waiting:next_iteration");
  // After shutdown nothing is read, written or submitted any more.
  await f.emit("session_shutdown");
  const counter = f.state().counter;
  f.stop("stop");
  await f.advance(60000);
  assert.equal(f.omp.calls.length, 1);
  assert.equal(f.state().counter, counter);
  assert.equal(f.exists("omp-autopilot-ack.json") && f.result(f.read("omp-autopilot-stop.json").request_id) !== undefined, false);
});

test("the extension registers no handler whose mere presence changes what OMP does", async t => {
  const f = await tab(t);
  // OMP consults input, session_stop and user_bash handlers, and a
  // session_before_compact handler switches off its background compaction.
  // The three other before-events only make OMP emit them; tool_call is the
  // question bridge's and session_tree was registered before autopilot.
  assert.deepEqual(f.events().sort(), [
    "agent_end", "agent_start", "auto_compaction_end", "auto_compaction_start", "auto_retry_end", "auto_retry_start",
    "goal_updated", "message_start", "mode_changed", "session_before_branch", "session_before_switch",
    "session_before_tree", "session_branch", "session_shutdown", "session_start", "session_switch", "session_tree",
    "tool_call", "tool_execution_end", "tool_execution_start", "turn_end",
  ]);
});

test("when several rules apply at the end of one turn, an end wins over a pause, in a fixed order", async t => {
  // The last of two iterations ends: interrupted, without a tool call, past
  // its time limit and with a Stop now waiting, in every combination.
  async function lastTurn(t, { stopping = false, pastDeadline = false, limit = 2, how = interrupted, options = {} } = {}) {
    const f = await tab(t);
    await start(f, { iteration_limit: atMost(limit), time_limit: atMost(3600), end_without_tool_call: true, ...options });
    await run(f, { tools: 1 });
    await jump(f, options.repeat ? options.repeat.seconds * 1000 : 5000);
    assert.equal(f.omp.calls.length, 2);
    await begin(f);
    if (stopping) await stop(f, "stop_now");
    if (pastDeadline) {
      // The limit passes in the same moment the turn ends, before its own timer has run.
      f.managed.clear();
      f.skip(3600000);
    }
    await finish(f, how);
    return f.state();
  }
  const reason = report => report.last_ended?.reason ?? `${report.autopilot.condition.kind}:${report.autopilot.condition.detail}`;
  assert.equal(reason(await lastTurn(t, { stopping: true, pastDeadline: true })), "stopped");
  assert.equal(reason(await lastTurn(t, { pastDeadline: true })), "time_limit");
  assert.equal(reason(await lastTurn(t)), "iteration_limit");
  assert.equal(reason(await lastTurn(t, { how: {} })), "iteration_limit");
  // Nothing ends it: the interrupt pauses, and an interrupted turn is never "no tool call".
  assert.equal(reason(await lastTurn(t, { limit: 9 })), "paused:interrupted");
  // A failed one is not either, and one failure does not pause.
  assert.equal(reason(await lastTurn(t, { limit: 9, how: failing })), "waiting:next_iteration");
  assert.equal(reason(await lastTurn(t, { limit: 9, how: {} })), "no_tool_call");
  // The next iteration could not start before the time limit: that ends it, interrupted or not.
  const short = { limit: 9, options: { repeat: { kind: "every", seconds: 3000 } } };
  assert.equal(reason(await lastTurn(t, short)), "time_limit_before_next");
  assert.equal(reason(await lastTurn(t, { ...short, how: { tools: 1 } })), "time_limit_before_next");
});

test("what the tab reports reads as the contract generated from shared/src/autopilot.rs", async t => {
  // The schema is generated from the Rust types. ajv comes with the protocol
  // package, whose test script runs this file.
  let conforms;
  try {
    const protocol = new URL("../../../packages/protocol/", import.meta.url);
    const Ajv = createRequire(new URL("package.json", protocol))("ajv/dist/2020").default;
    const { $defs } = JSON.parse(fs.readFileSync(new URL("schema/server-to-web.schema.json", protocol), "utf8"));
    const ajv = new Ajv({ allErrors: true, strict: false });
    ajv.addFormat("uuid", /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    ajv.addFormat("uint64", { type: "number", validate: value => Number.isSafeInteger(value) && value >= 0 });
    conforms = ajv.compile({ $ref: "#/$defs/PaneAutopilotState", $defs });
  } catch (error) {
    t.skip(`the protocol package's schema or its ajv is not available: ${error.message}`);
    return;
  }
  const rfc3339 = /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/;
  function check(f, kind) {
    const { autopilot, last_ended, omp_modes } = f.state();
    const state = plain({ autopilot, last_ended, omp_modes });
    assert.equal(conforms(state), true, `${kind}: ${JSON.stringify(conforms.errors)}`);
    assert.equal(autopilot?.condition.kind ?? (last_ended ? "ended" : "none"), kind);
    for (const time of [autopilot?.started_at, autopilot?.deadline, autopilot?.next_iteration_at, autopilot?.condition.since,
      last_ended?.started_at, last_ended?.ended_at]) {
      if (time !== undefined) assert.match(time, rfc3339, kind);
    }
    return state;
  }
  const f = await tab(t);
  check(f, "none");
  await start(f, { repeat: { kind: "every", seconds: 900 }, iteration_limit: atMost(20), time_limit: atMost(28800), end_without_tool_call: true });
  const running = check(f, "running");
  // The spec inside a report is the strict part: the check would see a field the contract lacks.
  assert.equal(conforms({ autopilot: { ...running.autopilot, spec: { ...running.autopilot.spec, token_limit: 1 } } }), false);
  assert.equal(conforms({ autopilot: { ...running.autopilot, iterations: undefined } }), false);
  await run(f, { tools: 1 });
  assert.notEqual(check(f, "waiting").autopilot.next_iteration_at, undefined);
  f.controller.alive = false;
  await f.advance(70000);
  check(f, "holding");
  f.omp.modes.goal = { status: "active", objective: "Ship the release", tokenBudget: 500000 };
  f.omp.modes.loop = { status: "paused" };
  await f.emit("goal_updated", { goal: { status: "active", tokensUsed: 12303 } });
  await f.emit("mode_changed", {});
  assert.deepEqual(Object.keys(check(f, "paused").omp_modes), ["goal", "loop"]);
  await stop(f, "stop");
  check(f, "ended");
  // Without either limit.
  const free = await tab(t);
  await start(free);
  assert.equal(check(free, "running").autopilot.deadline, undefined);
});
