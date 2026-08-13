import { describe, expect, it } from "vitest";
import { ALL_TAB_TYPES, isTabTypeAllowed, tabTypeKey } from "./tabTypes";

describe("tab type keys", () => {
  it("are kind:provider, matching shared::tab_type_key", () => {
    expect(tabTypeKey("agent", "claude")).toBe("agent:claude");
    expect(tabTypeKey("terminal", "codex")).toBe("terminal:codex");
    expect(tabTypeKey("agent", "cursor-agent")).toBe("agent:cursor-agent");
  });

  it("an empty deny list allows everything", () => {
    for (const t of ALL_TAB_TYPES) {
      expect(isTabTypeAllowed([], t.kind, t.provider), t.key).toBe(true);
    }
  });

  it("denying one type leaves its sibling alone", () => {
    const deny = ["terminal:claude"];
    expect(isTabTypeAllowed(deny, "terminal", "claude")).toBe(false);
    expect(isTabTypeAllowed(deny, "agent", "claude")).toBe(true);
    expect(isTabTypeAllowed(deny, "terminal", "codex")).toBe(true);
  });

  it("tolerates whitespace and case, matching the CLI's comparison", () => {
    expect(isTabTypeAllowed(["  Agent:Claude "], "agent", "claude")).toBe(false);
  });

  it("offers only the supported terminal hosts for new user panes", () => {
    expect(ALL_TAB_TYPES.map((type) => type.key)).toEqual([
      "terminal:claude",
      "terminal:codex",
      "terminal:opencode",
    ]);
  });
});
