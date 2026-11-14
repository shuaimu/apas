import { act, fireEvent, render, screen, within } from "@testing-library/react";
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

describe("UsageStatsRollup rows", () => {
  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();
  function usage(paneId: number, lastActiveMinutesAgo: number | undefined, over: Partial<UsageCounters> = {}) {
    return {
      pane_id: paneId,
      lifetime: counters(over),
      last_7d: counters(),
      today: counters(),
      last_active: lastActiveMinutesAgo === undefined ? undefined : ago(lastActiveMinutesAgo),
    };
  }
  function rowNames(): string[] {
    return within(screen.getByRole("table")).getAllByRole("rowheader").map((cell) => cell.textContent ?? "");
  }

  function seedMixed() {
    const stats: ProjectUsageStats = {
      panes: [
        usage(178, 60 * 24 * 120, { input_tokens: 16_711_800_000, prompts: 1083, cost_usd: 2900 }),
        usage(7, 30, { input_tokens: 1_000 }),
        usage(4, 60 * 24 * 110, { input_tokens: 5_226_700_000, prompts: 1110, cost_usd: 10.5 }),
        usage(9, 2, { input_tokens: 44_800 }),
        usage(0, undefined, { input_tokens: 50 }),
        usage(8, undefined, { input_tokens: 9_999 }),
      ],
      lifetime: counters({ input_tokens: 25_666_100_000, output_tokens: 1_029_800_000, cache_read_tokens: 530_306_100_000, cost_usd: 3009.01 }),
      last_7d: counters(),
      today: counters(),
    };
    seed(stats, [
      pane(7, "claude", { label: "Claude 2" }),
      pane(8, "omp", { label: "Never ran" }),
      pane(9, "omp", { label: "autopilot-check" }),
    ]);
  }

  it("lists open tabs first by last activity and folds closed tabs into one line", () => {
    seedMixed();
    render(<UsageStatsRollup />);

    // Open tabs, most recently active first; one that never reported activity
    // last; then the usage no tab accounts for; then the closed tabs' line.
    expect(rowNames()).toEqual([
      "autopilot-checkOMP",
      "Claude 2Claude",
      "Never ranOMP",
      "Unattributed",
      "Show 2 closed tabs",
    ]);
    expect(screen.queryByText("Tab 178")).toBeNull();

    // The folded line carries the closed tabs' sum, so the rows still add up
    // to the totals above.
    const folded = screen.getByRole("button", { name: "Show 2 closed tabs" }).closest("tr")!;
    expect(folded.textContent).toContain("2,193");
    expect(folded.textContent).toContain("21.9B");
    expect(folded.textContent).toContain("$2,910.50");
  });

  it("shows the closed tabs on request, most recently active first", () => {
    seedMixed();
    render(<UsageStatsRollup />);

    const toggle = screen.getByRole("button", { name: "Show 2 closed tabs" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);

    expect(screen.getByRole("button", { name: "Hide 2 closed tabs" }).getAttribute("aria-expanded")).toBe("true");
    expect(rowNames().slice(-2)).toEqual(["Tab 4", "Tab 178"]);

    fireEvent.click(screen.getByRole("button", { name: "Hide 2 closed tabs" }));
    expect(screen.queryByText("Tab 4")).toBeNull();
  });

  it("keeps the totals of every tab, in a unit that fits them", () => {
    seedMixed();
    render(<UsageStatsRollup />);

    // "Cost" is also a column of the table below.
    const totals = within(screen.getByText("Total tokens").closest("dl")!);
    const total = (label: string) => totals.getByText(label).parentElement!.textContent;
    expect(total("Input tokens")).toBe("Input tokens25.7B");
    expect(total("Output tokens")).toBe("Output tokens1.03B");
    expect(total("Total tokens")).toBe("Total tokens557.0B");
    expect(total("Cost")).toBe("Cost$3,009.01");
  });

  it("words a tab's last activity as its card does, and never as a negative time", () => {
    const stats: ProjectUsageStats = {
      panes: [
        // The server's clock can be ahead of this browser's.
        { ...usage(7, 0), last_active: new Date(Date.now() + 90_000).toISOString() },
        { ...usage(8, 0), last_active: new Date(Date.now() - 44_000).toISOString() },
        usage(9, 5),
      ],
      lifetime: counters(),
      last_7d: counters(),
      today: counters(),
    };
    seed(stats, [pane(7, "claude", { label: "Ahead" }), pane(8, "omp", { label: "Recent" }), pane(9, "omp", { label: "Earlier" })]);
    render(<UsageStatsRollup />);

    const lastActive = (name: string) =>
      screen.getByRole("rowheader", { name: new RegExp(`^${name}`) }).closest("tr")!.lastElementChild!.textContent;
    expect(lastActive("Ahead")).toBe("just now");
    expect(lastActive("Recent")).toBe("just now");
    expect(lastActive("Earlier")).toBe("5m ago");
  });

  it("still names a retired Pi tab's provider Pi, not its wire value", () => {
    const stats: ProjectUsageStats = {
      panes: [usage(7, 30, { input_tokens: 1_000 })],
      lifetime: counters({ input_tokens: 1_000 }),
      last_7d: counters(),
      today: counters(),
    };
    seed(stats, [pane(7, "pi", { label: "Old Pi" })]);
    render(<UsageStatsRollup />);

    expect(rowNames()).toEqual(["Old PiPi"]);
  });

  it("lists closed tabs outright when the project has no other row", () => {
    const stats: ProjectUsageStats = {
      panes: [usage(503, 60 * 24 * 39, { input_tokens: 27_300 })],
      lifetime: counters({ input_tokens: 27_300 }),
      last_7d: counters(),
      today: counters(),
    };
    seed(stats, []);
    render(<UsageStatsRollup />);

    expect(rowNames()).toEqual(["1 closed tab", "Tab 503"]);
    expect(screen.queryByRole("button", { name: /closed tab/ })).toBeNull();
  });
});
