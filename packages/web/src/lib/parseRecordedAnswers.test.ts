import { describe, expect, it } from "vitest";
import { parseRecordedAnswers } from "./store";

describe("parseRecordedAnswers", () => {
  it("recovers the answer a provider recorded as prose", () => {
    // The exact shape a claude transcript writes for an answered question.
    expect(
      parseRecordedAnswers('The user answered: "Pick a fruit"="Banana"'),
    ).toEqual({ "Pick a fruit": "Banana" });
  });

  it("recovers every answer of a multi-question call", () => {
    expect(
      parseRecordedAnswers(
        'Your questions have been answered: "Deploy now?"="Hold", "Branch?"="Merge to master"',
      ),
    ).toEqual({ "Deploy now?": "Hold", "Branch?": "Merge to master" });
  });

  it("survives quotes inside a question or an answer", () => {
    expect(
      parseRecordedAnswers(
        'The user answered: "Use \\"strict\\" mode?"="Yes, \\"strict\\""',
      ),
    ).toEqual({ 'Use "strict" mode?': 'Yes, "strict"' });
  });

  it("returns nothing for a cancelled question or unrelated tool output", () => {
    // A cancel carries no answer, and inventing one would settle a card that
    // is still open.
    expect(parseRecordedAnswers("The question was cancelled")).toBeUndefined();
    expect(parseRecordedAnswers("file listing")).toBeUndefined();
    expect(parseRecordedAnswers(undefined)).toBeUndefined();
    expect(parseRecordedAnswers({ answers: {} })).toBeUndefined();
  });
});
