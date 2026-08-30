import { describe, expect, it } from "vitest";
import type { SessionPaneSummary, UsageLimitsByProvider } from "@/lib/store";
import {
  activeUsageLimit,
  paneUsageLimit,
  usageLimitedLabel,
  usageLimitResetLabel,
} from "./usageLimitStatus";

const NOW = Date.parse("2026-08-20T20:00:00Z");

function claudePane(overrides: Partial<SessionPaneSummary> = {}): SessionPaneSummary {
  return {
    pane_id: 197,
    label: "Claude 4",
    kind: "terminal",
    provider: "claude",
    is_working: false,
    ...overrides,
  };
}

describe("usage limit availability", () => {
  it("returns the provider-confirmed limit while its reset is in the future", () => {
    const limits = {
      sevenDay: { utilization: 1 },
      usageLimited: {
        window: "weekly",
        resetsAt: "2026-08-23T13:00:00Z",
      },
    };

    expect(activeUsageLimit(limits, NOW)).toEqual(limits.usageLimited);
    expect(usageLimitedLabel(limits.usageLimited)).toBe("Weekly usage limited");
    expect(usageLimitResetLabel(limits.usageLimited, NOW)).toBe("Resets in 2d 17h");
  });

  it("does not infer blocking from a full meter without an explicit limit", () => {
    expect(activeUsageLimit({ sevenDay: { utilization: 1 } }, NOW)).toBeNull();
  });

  it("expires a stale blocking snapshot at the provider reset", () => {
    const limits = {
      usageLimited: {
        window: "weekly",
        resetsAt: "2026-08-20T19:59:59Z",
      },
    };
    expect(activeUsageLimit(limits, NOW)).toBeNull();
  });

  it("uses live CLI availability ahead of a stale pane-list snapshot", () => {
    const pane = claudePane({
      usage_limited: {
        window: "weekly",
        resets_at: "2026-08-23T13:00:00Z",
      },
    });
    const usage = new Map<string, UsageLimitsByProvider>([
      ["cli-1", { claude: { sevenDay: { utilization: 1 } } }],
    ]);

    expect(paneUsageLimit({ cliClientId: "cli-1" }, pane, usage, NOW)).toBeNull();
  });

  it("applies a Fable-scoped limit only to Fable panes", () => {
    const limited = {
      window: "weekly",
      resetsAt: "2026-08-23T13:00:00Z",
      model: "Fable",
    };
    const usage = new Map<string, UsageLimitsByProvider>([
      ["cli-1", { claude: { sevenDay: { utilization: 0.86 }, usageLimited: limited } }],
    ]);

    expect(paneUsageLimit(
      { cliClientId: "cli-1" },
      claudePane({ model: "claude-opus-4-6" }),
      usage,
      NOW,
    )).toBeNull();
    expect(paneUsageLimit(
      { cliClientId: "cli-1" },
      claudePane({ model: "claude-fable-5" }),
      usage,
      NOW,
    )).toEqual({ ...limited, allModelsUtilization: 0.86 });
    expect(usageLimitedLabel({ ...limited, allModelsUtilization: 0.86 }))
      .toBe("Fable weekly usage limited; all models at 86%");
  });

  it("keeps the scoped label concise when account utilization is unavailable", () => {
    expect(usageLimitedLabel({ window: "weekly", model: "Fable" }))
      .toBe("Fable weekly usage limited");
  });

  it("uses the pane snapshot when detailed account usage is private or absent", () => {
    const pane = claudePane({
      usage_limited: {
        window: "weekly",
        resets_at: "2026-08-23T13:00:00Z",
      },
    });

    expect(paneUsageLimit({}, pane, new Map(), NOW)).toEqual({
      window: "weekly",
      resetsAt: "2026-08-23T13:00:00Z",
    });
  });
});
