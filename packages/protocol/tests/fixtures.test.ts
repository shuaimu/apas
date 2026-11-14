import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  validateClientMessage,
  validateCodeEvent,
  validateServerMessage,
  validateJobContract,
  validateWorkerContract,
} from "../src/validators.js";

const fixturesDirectory = fileURLToPath(new URL("../fixtures", import.meta.url));

describe("Rust protocol golden fixtures", () => {
  for (const name of readdirSync(fixturesDirectory).filter((value) => value.endsWith(".json"))) {
    it(`accepts ${name}`, () => {
      const value: unknown = JSON.parse(readFileSync(`${fixturesDirectory}/${name}`, "utf8"));
      const result = name.startsWith("web-")
        ? validateClientMessage(value)
        : name.startsWith("server-")
          ? validateServerMessage(value)
          : name === "job-receipt.json"
            ? validateJobContract("receipt", value)
            : name === "job-events.json"
              ? validateJobContract("events", value)
              : name === "job-error.json"
                ? validateJobContract("error", value)
                : name === "code-event.json"
                  ? validateCodeEvent(value)
                  : (() => { throw new Error(`Unclassified protocol fixture: ${name}`); })();
      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    });
  }

  it("keeps an older client connected: its schema accepts a pane summary with the autopilot block", async () => {
    const { default: Ajv2020 } = await import("ajv/dist/2020.js");
    const legacySchema = JSON.parse(readFileSync(
      fileURLToPath(new URL("./legacy/server-to-web.pre-autopilot.schema.json", import.meta.url)), "utf8"));
    const validate = new Ajv2020({ allErrors: true, strict: false }).compile(legacySchema);
    const sessions = JSON.parse(readFileSync(`${fixturesDirectory}/server-sessions-autopilot.json`, "utf8"));
    expect(sessions.sessions[0].panes[0].autopilot.support).toBe("supported");
    expect(validate(sessions)).toBe(true);
    // The new frames are what that schema cannot read, which is why the
    // server sends them only to viewers that negotiated tab_autopilot_v1.
    const state = JSON.parse(readFileSync(`${fixturesDirectory}/server-pane-autopilot-state.json`, "utf8"));
    expect(validate(state)).toBe(false);
    const receipt = JSON.parse(readFileSync(`${fixturesDirectory}/server-pane-autopilot-receipt.json`, "utf8"));
    expect(validate(receipt)).toBe(false);
  });

  it("parses an autopilot start that says nothing, and refuses an invented outcome", () => {
    const start = JSON.parse(readFileSync(`${fixturesDirectory}/web-control-pane-autopilot-start.json`, "utf8"));
    expect(validateClientMessage({ ...start, action: { kind: "start", spec: {} } }).valid).toBe(true);
    expect(validateClientMessage({ ...start, action: { kind: "restart" } }).valid).toBe(false);
    const receipt = JSON.parse(readFileSync(`${fixturesDirectory}/server-pane-autopilot-receipt.json`, "utf8"));
    expect(validateServerMessage({ ...receipt, outcome: "probably" }).valid).toBe(false);
  });

  it("keeps older viewers connected: a schema without saved prompts cannot read their frames", async () => {
    const { default: Ajv2020 } = await import("ajv/dist/2020.js");
    const legacySchema = JSON.parse(readFileSync(
      fileURLToPath(new URL("./legacy/server-to-web.pre-autopilot.schema.json", import.meta.url)), "utf8"));
    const validate = new Ajv2020({ allErrors: true, strict: false }).compile(legacySchema);
    // Which is why the server sends them only to viewers that negotiated
    // project_autopilot_prompts_v1.
    for (const name of [
      "server-project-autopilot-prompts.json",
      "server-project-autopilot-prompts-unreadable.json",
      "server-autopilot-prompts-receipt.json",
    ]) {
      const frame = JSON.parse(readFileSync(`${fixturesDirectory}/${name}`, "utf8"));
      expect(validate(frame), name).toBe(false);
    }
  });

  it("parses each saved-prompt change and refuses an invented one", () => {
    const save = JSON.parse(readFileSync(`${fixturesDirectory}/web-update-autopilot-prompts-save.json`, "utf8"));
    const { replace: _replace, ...withoutReplace } = save.change;
    expect(validateClientMessage({ ...save, change: withoutReplace }).valid).toBe(true);
    expect(validateClientMessage({ ...save, change: { kind: "set_default" } }).valid).toBe(true);
    expect(validateClientMessage({ ...save, change: { kind: "rename", name: "x" } }).valid).toBe(false);
    expect(validateClientMessage({ ...save, change: { kind: "delete" } }).valid).toBe(false);
    const list = JSON.parse(readFileSync(`${fixturesDirectory}/server-project-autopilot-prompts.json`, "utf8"));
    const { updated_at: _updatedAt, ...undated } = list.prompts[0];
    expect(validateServerMessage({ ...list, prompts: [undated] }).valid).toBe(false);
  });

  it("still parses the retired Pi provider, in a frame from an older client and in a stored roster", () => {
    // Pi can no longer be created, and the server refuses a request for it.
    // The value stays in the schema so the frame is refused with a sentence
    // instead of dropping the socket, and so an old roster still loads.
    const add = JSON.parse(readFileSync(`${fixturesDirectory}/web-add-pane-targeted.json`, "utf8"));
    expect(validateClientMessage({ ...add, provider: "pi" }).valid).toBe(true);
    expect(validateClientMessage({ ...add, provider: "omp" }).valid).toBe(true);
    expect(validateClientMessage({ ...add, provider: "invented" }).valid).toBe(false);

    const roster = JSON.parse(readFileSync(`${fixturesDirectory}/server-pane-list-instances.json`, "utf8"));
    expect(roster.panes.length).toBeGreaterThan(0);
    const withPi = { ...roster, panes: roster.panes.map((pane: object) => ({ ...pane, provider: "pi" })) };
    expect(validateServerMessage(withPi).errors).toEqual([]);
  });

  it("rejects an unknown client mutation", () => {
    expect(validateClientMessage({ type: "invented_mutation" }).valid).toBe(false);
  });

  it("accepts an input refusal acknowledgement but not an invented mutation kind", () => {
    const refusal = JSON.parse(readFileSync(`${fixturesDirectory}/server-mutation-ack-input.json`, "utf8"));
    expect(validateServerMessage({ ...refusal, mutation: "approval" }).valid).toBe(true);
    expect(validateServerMessage({ ...refusal, mutation: "invented" }).valid).toBe(false);
  });

  it("rejects a job mutation with an unsafe sequence number", () => {
    expect(validateJobContract("mutation_request", {
      request_id: "7c6d2400-7b4b-455d-b732-7f066dab7a24",
      expected_version: Number.MAX_SAFE_INTEGER + 1,
    }).valid).toBe(false);
  });

  it("rejects unsafe worker versions and accidental mutation fields", () => {
    const request = {
      request_id: "7c6d2400-7b4b-455d-b732-7f066dab7a24",
      expected_version: 12,
    };
    expect(validateWorkerContract("mutation_request", request).valid).toBe(true);
    expect(validateWorkerContract("mutation_request", {
      ...request, expected_version: Number.MAX_SAFE_INTEGER + 1,
    }).valid).toBe(false);
    expect(validateWorkerContract("mutation_request", {
      ...request, approve_recurring: true,
    }).valid).toBe(false);
  });

  it("requires an execution reservation and refuses model delivery assertions", () => {
    const result = {
      version: 1,
      worker_id: "7c6d2400-7b4b-455d-b732-7f066dab7a24",
      reservation_id: "7c6d2400-7b4b-455d-b732-7f066dab7a25",
      mandate_revision: 1, backlog_revision: 0, schedule_generation: 1,
      purpose: "inspection", disposition: "no_work", summary: "No eligible maintenance work.",
      checklist: [], upserts: [], selected_item_id: null,
    };
    expect(validateWorkerContract("execution_result", result).valid).toBe(true);
    const { reservation_id: _reservation, ...unbound } = result;
    expect(validateWorkerContract("execution_result", unbound).valid).toBe(false);
    expect(validateWorkerContract("execution_result", {
      ...result, pull_request_merged: true,
    }).valid).toBe(false);
    expect(validateWorkerContract("execution_result", {
      ...result, disposition: "published",
    }).valid).toBe(false);
  });

  it("enforces signed priority boundaries on backlog edits", () => {
    const request = {
      request_id: "7c6d2400-7b4b-455d-b732-7f066dab7a24",
      expected_version: 1,
      item_id: "7c6d2400-7b4b-455d-b732-7f066dab7a25",
      expected_item_revision: null,
      item: {
        title: "Fix the documented boundary", instruction: "Correct the checked range.",
        acceptance_criteria: ["Boundary inputs produce the documented result."],
        scope_rationale: "Within the approved correctness mandate.",
        evidence: [], priority: 0, dependencies: [],
      },
    };
    for (const priority of [-2_147_483_648, 2_147_483_647]) {
      expect(validateWorkerContract("item_request", {
        ...request, item: { ...request.item, priority },
      }).valid).toBe(true);
    }
    for (const priority of [-2_147_483_649, 2_147_483_648, 1.5]) {
      expect(validateWorkerContract("item_request", {
        ...request, item: { ...request.item, priority },
      }).valid).toBe(false);
    }
  });

  it("rejects an unknown job receipt state instead of implying acceptance", () => {
    expect(validateJobContract("receipt", {
      request_id: "7c6d2400-7b4b-455d-b732-7f066dab7a24",
      operation: "cancel",
      status: "probably_stopped",
      job_id: null,
      run_id: null,
      error_code: null,
      message: null,
    }).valid).toBe(false);
  });

  it("requires full descriptions only for the flat project list contract", () => {
    const id = "7c6d2400-7b4b-455d-b732-7f066dab7a24";
    const summary = {
      id, project_id: "project-a", project_name: "Project A", title: "Short title",
      state: "running", state_version: 4, current_run_id: id, cluster_owner_user_id: id,
      machine_id: id, hostname: "host", created_at: "2026-11-09T00:00:00Z",
      updated_at: "2026-11-09T00:00:00Z", attention_count: 0,
      usage: { elapsed_seconds: 1, input_tokens: 1, output_tokens: 1, spend_micro_usd: 1, estimated: false },
      limits: { elapsed_seconds: 120, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000,
        memory_bytes: 536_870_912, workspace_bytes: 67_108_864, output_bytes: 4096,
        pids: 16, model_concurrency: 2, model_output_tokens: 1024 },
      allowed_actions: { admit: false, answer: false, cancel: true, retry: false,
        retry_publication: false, manage_policy: false },
    };
    const legacy = { jobs: [summary], next_cursor: null };
    expect(validateJobContract("list", legacy).valid).toBe(true);
    expect(validateJobContract("project_list", legacy).valid).toBe(false);
    const description = `${"完整任务说明。".repeat(30)}\nPreserve the approved instruction.`;
    expect(validateJobContract("project_list", {
      jobs: [{ ...summary, description }], next_cursor: "older",
    }).errors).toEqual([]);
    expect(validateJobContract("project_list", {
      jobs: [{ ...summary, description: null }], next_cursor: null,
    }).valid).toBe(false);
  });
});
