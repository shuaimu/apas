// OMP 18.3.2 and 18.6.0: extensibility/shared-events.ts AgentEndEvent,
// extensions/types.ts ExtensionContext and ToolExecution{Start,End}Event.
// In particular, agent_end.willContinue is decided AFTER automatic recovery,
// session_stop hooks, and pending async-wake routing. turn_end is not a settle.
import { lstatSync, readFileSync, realpathSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join } from "node:path";

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
    const launch = JSON.parse(readFileSync(launchPath, "utf8"));
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
    if (!owned) return;
    try {
      const report = JSON.parse(readFileSync(reportPath, "utf8"));
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
        settled = true;
        publish(ctx, "idle");
      }
    };
    // agent_end may still be unwinding its own prompt. Never overwrite a
    // newer agent_start with that older terminal notification.
    ctx.setTimeout(settle, 0);
  }

  function reset(_event, ctx) {
    revision++;
    asks.clear();
    settled = ctx.isIdle() && !ctx.hasPendingMessages();
    publish(ctx, settled ? "idle" : "working");
  }

  for (const event of ["session_start", "session_switch", "session_branch", "session_tree"]) {
    omp.on(event, reset);
  }
  omp.on("agent_start", (_event, ctx) => {
    asks.clear();
    active(ctx);
  });
  omp.on("tool_execution_start", (event, ctx) => {
    if (event.toolName === "ask") asks.add(event.toolCallId);
    active(ctx);
  });
  omp.on("tool_execution_end", (event, ctx) => {
    asks.delete(event.toolCallId);
    active(ctx);
  });
  omp.on("agent_end", (event, ctx) => {
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
  omp.on("session_shutdown", () => {
    revision++;
    clear();
    process.removeListener("exit", clear);
    exitRegistered = false;
  });
}
