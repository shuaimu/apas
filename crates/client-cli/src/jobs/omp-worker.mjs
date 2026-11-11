// Installed in the immutable worker image, never loaded from the repository.
// Native API references are pinned to OMP 18.18.0. An incompatible image fails
// before prompt admission; this program does not synthesize model responses.
import { readFile, mkdir, writeFile } from "node:fs/promises";

const root = "/opt/omp/packages/coding-agent";
const metadata = JSON.parse(await readFile(`${root}/package.json`, "utf8"));
if (metadata.version !== "18.18.0") throw new Error("Unqualified OMP version");
if (process.argv.slice(2).join(" ") === "--version") {
  process.stdout.write(`${metadata.version}\n`);
  process.exit(0);
}
if (process.argv.length !== 2) throw new Error("Worker bootstrap accepts no caller overrides");
const profile = JSON.parse(await readFile("/run/apas-bridge/worker-profile.json", "utf8"));
if (!profile.model || !["openai-responses", "anthropic-messages"].includes(profile.api)
    || !Number.isSafeInteger(profile.maxOutputTokens) || profile.maxOutputTokens <= 0
    || !Number.isSafeInteger(profile.contextWindow) || profile.contextWindow < profile.maxOutputTokens
    || typeof profile.reasoning !== "boolean"
    || !Number.isSafeInteger(profile.concurrency) || profile.concurrency <= 0
    || !Number.isSafeInteger(profile.elapsedSeconds) || profile.elapsedSeconds <= 0) {
  throw new Error("Invalid approved worker profile");
}
// The isolated HOME/session mount is the only provider store consulted. No
// host OAuth, global OMP config, repository config or injected caller env wins.
process.env.PI_CODING_AGENT_DIR = "/session/agent";
process.env.PI_NO_TITLE = "1";
process.env.PI_NO_PTY = "1";
process.env.PI_CACHE_RETENTION = "none";
process.env.PI_OPENAI_STATEFUL = "0";
const selector = `apas-job/${profile.model}`;
await mkdir("/session/agent", { recursive: true, mode: 0o700 });
await writeFile("/session/agent/models.yml", JSON.stringify({
  providers: {
    "apas-job": {
      baseUrl: "http://127.0.0.1:17840/v1",
      api: profile.api,
      apiKey: "apas-run-scoped-bridge",
      models: [{ id: profile.model, name: profile.model, api: profile.api,
        reasoning: profile.reasoning, input: ["text"], supportsTools: true,
        contextWindow: profile.contextWindow, maxTokens: profile.maxOutputTokens,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
        compat: { alwaysSendMaxTokens: true, supportsStore: false, statefulResponses: false } }],
    },
  },
}), { mode: 0o600, flag: "wx" });
const { Settings } = await import(`${root}/src/config/settings.ts`);
await import(`${root}/src/config/all-settings.ts`);
const { cfgAskTimeout } = await import(`${root}/src/modes/settings.ts`);
const { parseArgs } = await import(`${root}/src/cli/args.ts`);
const { runRootCommand } = await import(`${root}/src/main.ts`);
const { createAgentSession } = await import(`${root}/src/sdk.ts`);
const settings = Settings.isolated({
  "ask.timeout": 0,
  "async.enabled": true,
  "async.maxJobs": profile.concurrency,
  "task.maxConcurrency": profile.concurrency,
  "task.maxRuntimeMs": profile.elapsedSeconds * 1000,
  "task.isolation.enabled": false,
  "task.perCallModel": "off",
  "task.agentModelOverrides": {},
  "task.speculativeLaunch": false,
  "providers.cacheRetention": "none",
  "providers.openaiWebsockets": "off",
  "providers.openaiResponsesWebsockets": false,
  "modelRoles": { default: selector, smol: selector, slow: selector, plan: selector },
});
if (cfgAskTimeout.get(settings) !== 0) throw new Error("Effective ask timeout must be zero");
const args = ["--mode", "rpc-ui", "--session-dir", "/session/history", "--model", selector,
  "--no-extensions", "--no-skills", "--no-rules"];
process.stdout.write(`${JSON.stringify({ type: "apas_profile_ready", version: 1,
  ompVersion: metadata.version, askTimeout: 0, isolatedSettings: true,
  childQuestionEscalation: true })}\n`);
await runRootCommand(parseArgs(args), args, {
  settings,
  createAgentSession: async options => createAgentSession({
    ...options,
    settings,
    assertDispatchModel: model => {
      if (model.provider !== "apas-job" || model.id !== profile.model
          || model.api !== profile.api || model.baseUrl !== "http://127.0.0.1:17840/v1") {
        throw new Error("Model dispatch escaped approved run profile");
      }
    },
    disableModelFallback: true,
    disableServerSideFallback: true,
  }),
});
