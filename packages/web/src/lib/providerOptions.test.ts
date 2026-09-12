import { describe, expect, it } from "vitest";
import {
  CLAUDE_FABLE_MODEL,
  DEEPSEEK_DEFAULT_MODEL,
  DEEPSEEK_FLASH_MODEL,
  DEEPSEEK_LEGACY_FLASH_MODEL,
  DEEPSEEK_RETIRED_PRO_MODEL,
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

  it("offers exactly one DeepSeek backend, and it is Flash", () => {
    expect(DEEPSEEK_DEFAULT_MODEL).toBe(DEEPSEEK_FLASH_MODEL);
    expect(findProviderModelOption("claude/deepseek-flash")).toEqual(
      expect.objectContaining({
        label: "Claude / DeepSeek 4.1 Flash",
        provider: "claude",
        model: DEEPSEEK_FLASH_MODEL,
      }),
    );
    // Pro is withdrawn, so its option is gone rather than merely relabelled.
    expect(findProviderModelOption("claude/deepseek")).toEqual(
      expect.objectContaining({ value: "unsupported" }),
    );
    expect(
      PROVIDER_MODEL_OPTIONS.filter((option) => option.value.startsWith("claude/deepseek")),
    ).toHaveLength(1);
  });

  it("carries the legacy Flash id forward and retires Pro rather than remapping it", () => {
    // Same offering under an older id: existing panes keep working.
    expect(providerModelValue("claude", DEEPSEEK_FLASH_MODEL)).toBe("claude/deepseek-flash");
    expect(providerModelValue("claude", DEEPSEEK_LEGACY_FLASH_MODEL)).toBe("claude/deepseek-flash");
    expect(providerModelValue("deepseek", DEEPSEEK_LEGACY_FLASH_MODEL)).toBe("claude/deepseek-flash");
    expect(providerModelValue("deepseek", null)).toBe("claude/deepseek-flash");

    // A withdrawn model must read as unsupported, never silently become Flash:
    // that would move a live pane onto a different model and a different price.
    expect(isRetiredProviderModel("claude", DEEPSEEK_RETIRED_PRO_MODEL)).toBe(true);
    expect(providerModelValue("claude", DEEPSEEK_RETIRED_PRO_MODEL)).toBe("unsupported");
    expect(providerModelValue("deepseek", DEEPSEEK_RETIRED_PRO_MODEL)).toBe("unsupported");
    expect(isRetiredProviderModel("claude", DEEPSEEK_FLASH_MODEL)).toBe(false);
    expect(isRetiredProviderModel("claude", DEEPSEEK_LEGACY_FLASH_MODEL)).toBe(false);

    expect(providerModelValue("claude", "deepseek-chat")).toBe("unsupported");
    expect(providerModelValue("deepseek", "deepseek-chat")).toBe("unsupported");
  });

  it("exposes Pi as its own official terminal backend", () => {
    expect(findProviderModelOption("pi/official")).toEqual(
      expect.objectContaining({ label: "Pi", provider: "pi" }),
    );
    expect(providerModelValue("pi", null)).toBe("pi/official");
    expect(providerModelValue("pi", "default")).toBe("pi/official");
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
