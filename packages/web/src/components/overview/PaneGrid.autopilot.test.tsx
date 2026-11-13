import { act, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore, type PaneConfig } from "@/lib/store";
import { paneAutopilotKey, TAB_AUTOPILOT_CAPABILITY, type PaneAutopilotEntry } from "@/lib/autopilot";
import { PaneGrid } from "./PaneGrid";

const SID = "20000000-0000-4000-8000-000000000001";
const initialStore = useStore.getInitialState();

function tab(pane_id: number, label: string, overrides: Partial<PaneConfig> = {}): PaneConfig {
  return {
    pane_id,
    provider: "omp",
    mode: "interactive",
    kind: "terminal",
    session_id: `pane-${pane_id}`,
    is_paused: false,
    label,
    ...overrides,
  };
}

function entry(kind: string, detail?: string): PaneAutopilotEntry {
  return {
    support: "supported",
    state: {
      autopilot: {
        autopilot_id: "60000000-0000-4000-8000-000000000001",
        spec: {
          prompt: "Check the build and fix what fails.",
          repeat: { kind: "after_each_turn" },
          iteration_limit: { kind: "none" },
          time_limit: { kind: "none" },
          end_without_tool_call: true,
        },
        started_at: "2026-11-13T14:00:00Z",
        iterations: 3,
        condition: { kind, detail },
      },
    },
  };
}

function seed(panes: PaneConfig[], entries: Record<number, PaneAutopilotEntry>, negotiated = true) {
  act(() => {
    useStore.setState({
      sessionId: SID,
      negotiatedCapabilities: new Set(negotiated ? [TAB_AUTOPILOT_CAPABILITY] : []),
      paneConfigs: panes,
      paneStatuses: { "7": "Working" },
      pausedPanes: [],
      paneMessages: {},
      paneDiffs: {},
      paneAutopilot: Object.fromEntries(Object.entries(entries)
        .map(([paneId, value]) => [paneAutopilotKey(SID, Number(paneId)), value])),
      sessions: [{
        id: SID,
        status: "active",
        panes: [
          // Only the session list knows this tab's autopilot.
          { pane_id: 9, kind: "terminal", provider: "omp", autopilot: { support: "supported", condition: "ended", detail: "time_limit" } },
        ],
      }],
    });
  });
}

function card(label: string): HTMLElement {
  return screen.getByTitle(`Open ${label}`);
}

function renderGrid() {
  return render(<PaneGrid onOpenPane={vi.fn()} onOpenDiff={vi.fn()} onOpenRole={vi.fn()} onRemovePane={vi.fn()} />);
}

afterEach(() => {
  act(() => {
    useStore.setState(initialStore, true);
  });
  vi.clearAllMocks();
});

describe("PaneGrid and autopilot", () => {
  it("shows the autopilot condition beside the status on each tab's card", () => {
    seed(
      [tab(7, "OMP 1"), tab(8, "OMP 2"), tab(9, "OMP 3"), tab(10, "OMP 4"), tab(11, "Claude", { provider: "claude" })],
      { 7: entry("running"), 8: entry("paused", "failures"), 10: { support: "supported" } },
    );
    renderGrid();

    // Beside the status, never in its place.
    expect(within(card("OMP 1")).getByText("Working")).toBeTruthy();
    expect(within(card("OMP 1")).getByTestId("pane-card-autopilot").textContent)
      .toBe("Autopilot · Running an iteration");
    expect(within(card("OMP 2")).getByTestId("pane-card-autopilot").textContent)
      .toBe("Autopilot · Paused after three failed iterations");
    expect(within(card("OMP 3")).getByTestId("pane-card-autopilot").textContent)
      .toBe("Autopilot · Ended: time limit reached");
    expect(within(card("OMP 4")).queryByTestId("pane-card-autopilot")).toBeNull();
    expect(within(card("Claude")).queryByTestId("pane-card-autopilot")).toBeNull();
  });

  it("shows nothing about autopilot on a server without it", () => {
    seed([tab(7, "OMP 1")], { 7: entry("running") }, false);
    renderGrid();
    expect(screen.queryByTestId("pane-card-autopilot")).toBeNull();
  });
});
