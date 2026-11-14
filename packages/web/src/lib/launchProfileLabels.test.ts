import { describe, expect, it } from "vitest";
import { launchProfileKey } from "@/lib/store";
import type { Provider } from "@/lib/store";
import { PROVIDER_MODEL_OPTIONS } from "@/lib/providerOptions";
import { launchProfileLabel, providerDisplayName, providerModelDisplayName } from "./launchProfileLabels";

describe("launchProfileLabel", () => {
  it("names every profile the catalog offers the way the catalog does", () => {
    for (const option of PROVIDER_MODEL_OPTIONS) {
      const key = launchProfileKey("terminal", option.provider as Provider, option.model);
      expect(launchProfileLabel(key)).toEqual({ key, label: option.label, known: true });
    }
  });

  it("reads the keys a policy carries", () => {
    expect(launchProfileLabel("terminal:claude:official:default").label).toBe("Claude / Official");
    expect(launchProfileLabel("terminal:claude:deepseek:deepseek-flash").label).toBe("Claude / DeepSeek 4.1 Flash");
    expect(launchProfileLabel("terminal:codex:official:default").label).toBe("Codex / Official");
    expect(launchProfileLabel("terminal:omp:official:default").label).toBe("OMP");
  });

  it("accepts the key a model had before it was renamed, in any case", () => {
    expect(launchProfileLabel("Terminal:Claude:DeepSeek:deepseek-v4-flash").label).toBe("Claude / DeepSeek 4.1 Flash");
  });

  it("marks a structured agent profile as one", () => {
    expect(launchProfileLabel("agent:claude:official:default").label).toBe("Claude / Official (structured agent)");
  });

  it("keeps the raw key for a profile the catalog does not know", () => {
    expect(launchProfileLabel("terminal:nova:official:default")).toEqual({
      key: "terminal:nova:official:default",
      label: "terminal:nova:official:default",
      known: false,
    });
    expect(launchProfileLabel("terminal:claude:minimax:m2").known).toBe(false);
  });
});

describe("providerDisplayName", () => {
  it("uses the catalog's spelling and keeps an unknown provider's own name", () => {
    expect(providerDisplayName("claude")).toBe("Claude");
    expect(providerDisplayName("omp")).toBe("OMP");
    expect(providerDisplayName("cursor-agent")).toBe("Cursor");
    expect(providerDisplayName("deepseek")).toBe("DeepSeek");
    expect(providerDisplayName("nova")).toBe("nova");
  });

  it("still names a retired provider that an old roster carries", () => {
    // The catalog no longer has a Pi group; the tab stays as read-only history.
    expect(providerDisplayName("pi")).toBe("Pi");
  });
});

describe("providerModelDisplayName", () => {
  it("names what a tab runs", () => {
    expect(providerModelDisplayName("claude", undefined)).toBe("Claude");
    expect(providerModelDisplayName("claude", "sonnet")).toBe("Claude");
    expect(providerModelDisplayName("claude", "deepseek-flash")).toBe("Claude / DeepSeek 4.1 Flash");
    expect(providerModelDisplayName("codex", null)).toBe("Codex");
    expect(providerModelDisplayName("omp", null)).toBe("OMP");
  });

  it("has no name for a combination the catalog does not offer", () => {
    expect(providerModelDisplayName("claude", "glm-5.1")).toBeNull();
    expect(providerModelDisplayName("nova", null)).toBeNull();
  });
});
