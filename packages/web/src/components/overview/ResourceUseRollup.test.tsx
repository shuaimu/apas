import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  useStore,
  type PaneConfig,
  type Provider,
  type UsageLimitsByProvider,
} from "@/lib/store";
import { ResourceUseRollup } from "./ResourceUseRollup";

const initialStore = useStore.getState();
const NOW = new Date("2026-06-16T12:00:00Z");

function pane(paneId: number, provider: Provider): PaneConfig {
  return {
    pane_id: paneId,
    provider,
    mode: "deadloop",
    session_id: `pane-${paneId}`,
    is_paused: false,
  };
}

function usageByClient(
  entries: Array<[string, UsageLimitsByProvider]>,
): Map<string, UsageLimitsByProvider> {
  return new Map(entries);
}

function seedRollup({
  cliClientId = "cli-a",
  paneConfigs = [],
  usageLimits = usageByClient([]),
}: {
  cliClientId?: string | null;
  paneConfigs?: PaneConfig[];
  usageLimits?: Map<string, UsageLimitsByProvider>;
}) {
  act(() => {
    useStore.setState({
      cliClientId,
      paneConfigs,
      usageLimits,
    });
  });
}

function providerCard(label: string): HTMLElement {
  const labelNode = screen.getByText(label);
  const card = labelNode.parentElement;
  if (!card) throw new Error(`Missing provider card for ${label}`);
  return card;
}

describe("ResourceUseRollup", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    seedRollup({});
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("shows the empty state when the active project has no usage data", () => {
    seedRollup({
      cliClientId: "cli-a",
      paneConfigs: [pane(1, "claude")],
      usageLimits: usageByClient([
        [
          "cli-b",
          {
            claude: {
              sevenDay: { utilization: 0.9 },
            },
          },
        ],
      ]),
    });

    render(<ResourceUseRollup />);

    expect(screen.getByText(/No usage telemetry yet/i)).toBeTruthy();
    expect(screen.queryByText("Claude")).toBeNull();
  });

  it("shows the empty state without an active cli client", () => {
    seedRollup({
      cliClientId: null,
      paneConfigs: [pane(1, "claude")],
      usageLimits: usageByClient([
        [
          "cli-a",
          {
            claude: {
              sevenDay: { utilization: 0.6 },
            },
          },
        ],
      ]),
    });

    render(<ResourceUseRollup />);

    expect(screen.getByText(/No usage telemetry yet/i)).toBeTruthy();
    expect(screen.queryByText("Claude")).toBeNull();
  });

  it("shows the empty state when cached provider entries have no limit windows", () => {
    seedRollup({
      cliClientId: "cli-a",
      paneConfigs: [pane(1, "claude"), pane(2, "codex")],
      usageLimits: usageByClient([
        [
          "cli-a",
          {
            claude: { fetchedAt: "2026-06-16T11:59:00Z" },
            codex: {},
          },
        ],
      ]),
    });

    render(<ResourceUseRollup />);

    expect(screen.getByText(/No usage telemetry yet/i)).toBeTruthy();
    expect(screen.queryByText("Claude")).toBeNull();
    expect(screen.queryByText("Codex")).toBeNull();
  });

  it("deduplicates active pane providers and displays compact usage totals", () => {
    seedRollup({
      cliClientId: "cli-a",
      paneConfigs: [
        pane(1, "claude"),
        pane(2, "claude"),
        pane(3, "codex"),
        pane(4, "opencode"),
      ],
      usageLimits: usageByClient([
        [
          "cli-a",
          {
            claude: {
              sevenDay: {
                utilization: 0.42,
                resetsAt: "2026-06-20T12:00:00Z",
              },
            },
            codex: {
              fiveHour: {
                utilization: 0.8,
                resetsAt: "2026-06-16T17:00:00Z",
              },
            },
            deepseek: {
              sevenDay: { utilization: 0.95 },
            },
          },
        ],
      ]),
    });

    render(<ResourceUseRollup />);

    expect(screen.getAllByText("Claude")).toHaveLength(1);
    expect(within(providerCard("Claude")).getByText("42%")).toBeTruthy();
    expect(within(providerCard("Codex")).getByText("80%")).toBeTruthy();
    expect(screen.queryByText("DeepSeek")).toBeNull();
    expect(screen.queryByText("OpenCode")).toBeNull();
  });

  it("renders the cursor-agent provider with the human-readable Cursor label", () => {
    seedRollup({
      cliClientId: "cli-a",
      paneConfigs: [pane(1, "cursor-agent")],
      usageLimits: usageByClient([
        [
          "cli-a",
          {
            "cursor-agent": {
              sevenDay: { utilization: 0.67 },
            },
          },
        ],
      ]),
    });

    render(<ResourceUseRollup />);

    expect(within(providerCard("Cursor")).getByText("67%")).toBeTruthy();
    expect(screen.queryByText("cursor-agent")).toBeNull();
  });

  it("stays scoped to the active cli client when usage data exists for another project", () => {
    seedRollup({
      cliClientId: "cli-a",
      paneConfigs: [pane(1, "claude"), pane(2, "deepseek")],
      usageLimits: usageByClient([
        [
          "cli-a",
          {
            claude: {
              sevenDay: { utilization: 0.25 },
            },
          },
        ],
        [
          "cli-b",
          {
            deepseek: {
              sevenDay: { utilization: 0.91 },
            },
          },
        ],
      ]),
    });

    render(<ResourceUseRollup />);

    expect(within(providerCard("Claude")).getByText("25%")).toBeTruthy();
    expect(screen.queryByText("DeepSeek")).toBeNull();

    act(() => {
      useStore.setState({ cliClientId: "cli-b" });
    });

    expect(within(providerCard("DeepSeek")).getByText("91%")).toBeTruthy();
    expect(screen.queryByText("Claude")).toBeNull();
  });
});
