import { describe, expect, it } from "vitest";
import {
  CLAUDE_FABLE_MODEL,
  findProviderModelOption,
  isFableModel,
  providerModelValue,
  PROVIDER_MODEL_OPTIONS,
} from "./providerOptions";

describe("providerOptions", () => {
  it("exposes Claude Fable through shared provider options", () => {
    const option = findProviderModelOption("claude/fable");

    expect(option).toEqual(
      expect.objectContaining({
        value: "claude/fable",
        label: "Claude / Fable",
        provider: "claude",
        model: CLAUDE_FABLE_MODEL,
      }),
    );
    expect(PROVIDER_MODEL_OPTIONS).toContainEqual(option);
  });

  it("maps existing Fable model metadata to the shared option value", () => {
    expect(isFableModel(CLAUDE_FABLE_MODEL)).toBe(true);
    expect(isFableModel("Fable")).toBe(true);
    expect(providerModelValue("claude", CLAUDE_FABLE_MODEL)).toBe("claude/fable");
    expect(providerModelValue("claude", "Fable")).toBe("claude/fable");
  });
});
