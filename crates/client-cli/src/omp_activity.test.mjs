import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as crypto from "node:crypto";
import { EventEmitter } from "node:events";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";

// No provider, network, terminal keys, or real timers. The real extension runs
// against private files and a controlled native picker/promise in an ESM VM.
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

async function fixture(t, { mode = "tui", kind = "main", capability = true } = {}) {
  const root = fs.mkdtempSync(path.join(tmpdir(), "apas-omp-bridge-"));
  const runtime = path.join(root, "runtime");
  const sessions = path.join(root, "sessions");
  fs.mkdirSync(runtime, { mode: 0o700 });
  fs.mkdirSync(sessions);
  const launch = { launch_id: crypto.randomUUID(), session_dir: sessions };
  const timers = new Map();
  const handlers = new Map();
  const natives = [];
  let session = "native-session";
  let failure;
  const proc = Object.assign(new EventEmitter(), {
    platform: "linux", pid: process.pid, geteuid: () => process.geteuid(),
    env: { APAS_OMP_RUNTIME: runtime, APAS_OMP_LAUNCH: launch.launch_id },
  });
  const native = (items, options) => new Promise((resolve, reject) => {
    natives.push({ items, options, resolve, reject });
    options?.signal?.addEventListener("abort", () => resolve(undefined), { once: true });
  });
  const ui = capability ? { askDialog: native } : {};
  const ctx = {
    agent: { kind }, mode, hasUI: mode === "tui", ui,
    sessionManager: {
      getSessionId: () => session,
      getSessionFile: () => path.join(sessions, `${session}.jsonl`),
      getSessionDir: () => sessions,
    },
    isIdle: () => true, hasPendingMessages: () => false,
    setTimeout: callback => addTimer(callback, false),
  };
  function addTimer(callback, repeat) {
    const handle = { unref() {} };
    timers.set(handle, { callback, repeat });
    return handle;
  }
  const context = createContext({
    process: proc, Buffer, AbortController, AbortSignal, structuredClone,
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
        return fs.renameSync(from, to);
      },
    },
    "node:path": path,
    "node:crypto": crypto,
  };
  const extension = new SourceTextModule(source, { context });
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
  extension.namespace.default({ on(name, callback) {
    handlers.set(name, [...(handlers.get(name) ?? []), callback]);
  } });
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
  return {
    ctx, ui, runtime, launch, natives, read, write, emit,
    exists: name => fs.existsSync(path.join(runtime, name)),
    fail: name => { failure = name; },
    switchSession: value => { session = value; },
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
