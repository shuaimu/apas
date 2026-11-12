import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  validateClientMessage,
  validateCodeEvent,
  validateServerMessage,
  validateJobContract,
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
