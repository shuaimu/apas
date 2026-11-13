import { describe, expect, it } from "vitest";
import { parseWorkerDestination } from "./workerDestination";

describe("exact worker destinations", () => {
  it("retains the requested historical execution and question", () => {
    expect(parseWorkerDestination(["worker", "runs", "older", "questions", "dialog"])).toEqual({ kind: "worker", id: "worker", runId: "older", questionId: "dialog" });
    expect(parseWorkerDestination(["worker", "runs", "older", "review"])).toEqual({ kind: "worker", id: "worker", runId: "older", review: true });
  });
  it("never substitutes latest-run evidence for malformed or run-less links", () => {
    for (const path of [["worker", "review"], ["worker", "questions", "dialog"], ["worker", "runs"], ["worker", "runs", "older", "unknown"], ["new", "runs", "run"]]) {
      expect(parseWorkerDestination(path)).toEqual({ kind: "invalid" });
    }
  });
  it("separates inventory, unapproved composition and persistent identity", () => {
    expect(parseWorkerDestination([])).toEqual({ kind: "list" });
    expect(parseWorkerDestination(["new"])).toEqual({ kind: "new" });
    expect(parseWorkerDestination(["worker"])).toEqual({ kind: "worker", id: "worker" });
  });
});
