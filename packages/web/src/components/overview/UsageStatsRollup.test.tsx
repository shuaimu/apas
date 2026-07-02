import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  useStore,
  type PaneConfig,
  type Provider,
  type ProjectUsageStats,
  type UsageCounters,
} from "@/lib/store";
import { UsageStatsRollup } from "./UsageStatsRollup";

const initialStore = useStore.getState();

function counters(over: Partial<UsageCounters> = {}): UsageCounters {
  return {
    prompts: 0,
    responses: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    cost_usd: 0,
    ...over,
  };
}

function pane(paneId: number, provider: Provider, over: Partial<PaneConfig> = {}): PaneConfig {
  return {
    pane_id: paneId,
    provider,
    mode: "deadloop",
    session_id: "s",
    is_paused: false,
    ...over,
  };
}

function seed(stats: ProjectUsageStats | undefined, paneConfigs: PaneConfig[] = []) {
  act(() => {
    useStore.setState({
      sessionId: "S",
      usageStats: stats ? { S: stats } : {},
      paneConfigs,
    });
  });
}

describe("UsageStatsRollup", () => {
  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("shows the empty state when the session has no usage stats", () => {
    seed(undefined);
    render(<UsageStatsRollup />);
    expect(screen.getByText(/No usage recorded yet/i)).toBeTruthy();
  });

  it("renders project totals and a per-pane row for the lifetime window", () => {
    const lifetime = counters({
      prompts: 5,
      responses: 4,
      input_tokens: 12000,
      output_tokens: 3000,
      cost_usd: 2.5,
    });
    const stats: ProjectUsageStats = {
      panes: [{ pane_id: 178, lifetime, last_7d: counters(), today: counters() }],
      lifetime,
      last_7d: counters(),
      today: counters(),
    };
    seed(stats, [pane(178, "claude", { label: "Tech Lead", role: "tech lead" })]);
    render(<UsageStatsRollup />);

    expect(screen.getByText("Tech Lead")).toBeTruthy();
    // 12000 + 3000 = 15000 tokens -> "15k" (project total card + the pane row).
    expect(screen.getAllByText("15k").length).toBeGreaterThanOrEqual(2);
    // Real cost rendered (not a dash).
    expect(screen.getAllByText("$2.50").length).toBeGreaterThanOrEqual(2);
  });

  it("switches the displayed numbers when the time window toggles", () => {
    const stats: ProjectUsageStats = {
      panes: [
        {
          pane_id: 1,
          lifetime: counters({ input_tokens: 9000 }),
          last_7d: counters({ input_tokens: 3000 }),
          today: counters({ input_tokens: 1000 }),
        },
      ],
      lifetime: counters({ input_tokens: 9000 }),
      last_7d: counters({ input_tokens: 3000 }),
      today: counters({ input_tokens: 1000 }),
    };
    seed(stats, [pane(1, "claude")]);
    render(<UsageStatsRollup />);

    // Defaults to lifetime ("All time").
    expect(screen.getAllByText("9k").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByText("Today"));
    expect(screen.getAllByText("1k").length).toBeGreaterThan(0);
    expect(screen.queryByText("9k")).toBeNull();
  });

  it("labels the unattributed bucket and dashes zero cost", () => {
    const stats: ProjectUsageStats = {
      panes: [{ pane_id: 0, lifetime: counters({ input_tokens: 50 }), last_7d: counters(), today: counters() }],
      lifetime: counters({ input_tokens: 50 }),
      last_7d: counters(),
      today: counters(),
    };
    seed(stats, []);
    render(<UsageStatsRollup />);

    expect(screen.getByText("Unattributed")).toBeTruthy();
    // Zero cost renders as an em dash, not "$0".
    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });
});
