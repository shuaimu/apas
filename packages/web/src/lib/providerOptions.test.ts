import { describe, expect, it } from "vitest";
import {
  CLAUDE_FABLE_MODEL,
  findProviderModelOption,
  isFableModel,
  isRetiredLaunchProfileKey,
  isRetiredProviderModel,
  providerModelValue,
  PROVIDER_MODEL_OPTIONS,
  UNSUPPORTED_PROVIDER_MODEL_OPTION,
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

  it("excludes retired providers and classifies historical values as unsupported", () => {
    expect(PROVIDER_MODEL_OPTIONS.some((option) =>
      /minimax|glm/i.test(`${option.provider} ${option.model ?? ""} ${option.label}`)
    )).toBe(false);

    for (const [provider, model] of [
      ["minimax", null],
      ["glm", null],
      ["claude", "MiniMax-M2.7"],
      ["claude", "m2.7"],
      ["claude", "glm-5.1"],
    ] as const) {
      expect(isRetiredProviderModel(provider, model)).toBe(true);
      expect(providerModelValue(provider, model)).toBe("unsupported");
    }

    expect(findProviderModelOption("unsupported")).toEqual(
      UNSUPPORTED_PROVIDER_MODEL_OPTION,
    );
    expect(findProviderModelOption("unknown/provider")).toEqual(
      UNSUPPORTED_PROVIDER_MODEL_OPTION,
    );
    expect(isRetiredLaunchProfileKey("agent:claude:glm:glm-5.1")).toBe(true);
    expect(isRetiredLaunchProfileKey("agent:minimax:official:default")).toBe(true);
    expect(isRetiredLaunchProfileKey("agent:codex:official:default")).toBe(false);
  });
});
