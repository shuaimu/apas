import { describe, expect, it } from "vitest";
import { ALL_TAB_TYPES, isTabTypeAllowed, tabTypeKey } from "./tabTypes";
import { PROVIDER_MODEL_GROUPS } from "./providerOptions";

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

  it("covers every provider the add-tab menu can actually send", () => {
    // The menu's agent providers are exactly the group providers. If a group
    // is added without a matching tab type, its tabs become unrestrictable —
    // an admin would untick everything and that provider would still appear.
    const menuProviders = new Set(
      PROVIDER_MODEL_GROUPS.flatMap((g) => g.options.map((o) => o.provider)),
    );
    const covered = new Set(
      ALL_TAB_TYPES.filter((t) => t.kind === "agent").map((t) => t.provider),
    );
    for (const p of menuProviders) {
      expect(covered.has(p), `no tab type covers menu provider "${p}"`).toBe(true);
    }
    // And nothing extra, which would be a checkbox with no menu entry behind it.
    for (const p of covered) {
      expect(menuProviders.has(p), `tab type "agent:${p}" has no menu entry`).toBe(true);
    }
  });
});
