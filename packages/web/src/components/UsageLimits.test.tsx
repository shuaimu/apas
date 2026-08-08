import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AllProvidersUsage,
  UsageLimitsDisplay,
  UsageLimitsPanel,
} from "./UsageLimits";
import {
  useStore,
  type CliClient,
  type SessionInfo,
  type UsageLimits,
  type UsageLimitsByProvider,
} from "@/lib/store";

const initialStore = useStore.getState();
const NOW = new Date("2026-06-17T12:00:00Z");

function renderUsage(limits: UsageLimits, compact = false) {
  return render(<UsageLimitsDisplay limits={limits} compact={compact} />);
}

function usage(utilization: number, fetchedAt?: string): UsageLimits {
  return {
    sevenDay: { utilization },
    fetchedAt,
  };
}

function usageByClient(
  entries: Array<[string, UsageLimitsByProvider]>,
): Map<string, UsageLimitsByProvider> {
  return new Map(entries);
}

function seedStore({
  sessionId = null,
  cliClients = [],
  sessions = [],
  usageLimits = usageByClient([]),
}: {
  sessionId?: string | null;
  cliClients?: CliClient[];
  sessions?: SessionInfo[];
  usageLimits?: Map<string, UsageLimitsByProvider>;
}) {
  act(() => {
    useStore.setState({
      sessionId,
      cliClients,
      sessions,
      usageLimits,
    });
  });
}

function cliClient(
  id: string,
  activeSession?: string,
): CliClient {
  return {
    id,
    status: "online",
    activeSession,
  };
}

function session(
  id: string,
  cliClientId?: string,
): SessionInfo {
  return {
    id,
    status: "active",
    cliClientId,
  };
}

function providerCard(label: string): HTMLElement {
  const labelNode = screen.getByText(label);
  const card = labelNode.parentElement;
  if (!card) throw new Error(`Missing provider card for ${label}`);
  return card;
}

describe("UsageLimitsDisplay", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    seedStore({});
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders full-mode weekly and five-hour windows with reset metadata", () => {
    renderUsage({
      sevenDay: {
        utilization: 0.61,
        resetsAt: "2026-06-23T15:00:00Z",
      },
      fiveHour: {
        utilization: 0.8,
        resetsAt: "2026-06-17T14:30:00Z",
      },
    });

    expect(screen.getByText("Weekly")).toBeTruthy();
    expect(screen.getByText("5-Hour")).toBeTruthy();
    expect(screen.getByText(/61%/)).toBeTruthy();
    expect(screen.getByText(/6d 3h/)).toBeTruthy();
    expect(screen.getByText(/80%/)).toBeTruthy();
    expect(screen.getByText(/2h 30m/)).toBeTruthy();
  });

  it("omits reset metadata for invalid or missing reset timestamps", () => {
    const { container } = renderUsage({
      sevenDay: {
        utilization: 0.5,
        resetsAt: "not-a-date",
      },
      fiveHour: {
        utilization: 0.25,
      },
    });

    expect(screen.getByText("50%")).toBeTruthy();
    expect(screen.getByText("25%")).toBeTruthy();
    expect(container.textContent).not.toContain("resets");
    expect(container.textContent).not.toContain("resetting");
  });

  it("does not round sub-100 utilization up to 100 percent", () => {
    renderUsage({
      sevenDay: {
        utilization: 0.9999,
      },
    });

    expect(screen.getByText("99.9%")).toBeTruthy();
    expect(screen.queryByText("100%")).toBeNull();
  });

  it("prefers weekly usage in compact mode when both windows exist", () => {
    const { container } = renderUsage(
      {
        sevenDay: {
          utilization: 0.3,
        },
        fiveHour: {
          utilization: 0.9,
        },
      },
      true,
    );

    expect(container.textContent).toContain("30%");
    expect(container.textContent).not.toContain("90%");
  });

  it("shows compact reset text at the 50 percent threshold", () => {
    const limits: UsageLimits = {
      sevenDay: {
        utilization: 0.49,
        resetsAt: "2026-06-17T15:15:00Z",
      },
    };
    const { container, rerender } = renderUsage(limits, true);

    expect(container.textContent).toContain("49%");
    expect(container.textContent).not.toContain("3h 15m");

    rerender(
      <UsageLimitsDisplay
        limits={{
          sevenDay: {
            utilization: 0.5,
            resetsAt: "2026-06-17T15:15:00Z",
          },
        }}
        compact
      />,
    );

    expect(container.textContent).toContain("50%");
    expect(container.textContent).toContain("3h 15m");
  });

  it("surfaces active resetting state in compact mode", () => {
    const { container } = renderUsage(
      {
        sevenDay: {
          utilization: 0.1,
          resetsAt: "2026-06-17T11:59:00Z",
        },
      },
      true,
    );

    expect(container.textContent).toContain("10%");
    expect(container.textContent).toContain("resetting...");
  });
});

describe("AllProvidersUsage", () => {
  beforeEach(() => {
    seedStore({
      usageLimits: usageByClient([
        [
          "cli-old",
          {
            claude: usage(0.2, "2026-06-17T10:00:00Z"),
            codex: {},
          },
        ],
        [
          "cli-new",
          {
            claude: usage(0.7, "2026-06-17T11:00:00Z"),
            deepseek: {
              fiveHour: { utilization: 0.5 },
              fetchedAt: "2026-06-17T09:00:00Z",
            },
          },
        ],
        [
          "cli-mixed",
          ({
            glm: usage(0.4, "2026-06-17T11:30:00Z"),
            minimax: {},
          } as unknown as UsageLimitsByProvider),
        ],
      ]),
    });
  });

  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("aggregates newest non-empty provider windows across CLI clients", () => {
    render(<AllProvidersUsage />);

    expect(within(providerCard("Claude")).getByText("70%")).toBeTruthy();
    expect(within(providerCard("DeepSeek")).getByText("50%")).toBeTruthy();
    expect(screen.queryByText("20%")).toBeNull();
    expect(screen.queryByText("Codex")).toBeNull();
    expect(screen.queryByText("MiniMax")).toBeNull();
    expect(screen.queryByText("GLM")).toBeNull();
  });
});

describe("UsageLimitsPanel", () => {
  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("resolves the current CLI client from an active session", () => {
    seedStore({
      sessionId: "session-active",
      cliClients: [
        cliClient("cli-current", "session-active"),
        cliClient("cli-other", "session-other"),
      ],
      usageLimits: usageByClient([
        [
          "cli-current",
          ({
            glm: usage(0.42),
            deepseek: usage(0.8),
          } as unknown as UsageLimitsByProvider),
        ],
      ]),
    });

    render(<UsageLimitsPanel />);

    expect(screen.getByText("DeepSeek Usage")).toBeTruthy();
    expect(screen.getByText("80%")).toBeTruthy();
    expect(screen.queryByText("GLM Usage")).toBeNull();
  });

  it("falls back to the persisted session CLI client id", () => {
    seedStore({
      sessionId: "session-fallback",
      cliClients: [cliClient("cli-other", "session-other")],
      sessions: [session("session-fallback", "cli-from-session")],
      usageLimits: usageByClient([
        [
          "cli-from-session",
          {
            codex: usage(0.63),
          },
        ],
      ]),
    });

    render(<UsageLimitsPanel />);

    expect(screen.getByText("Codex Usage")).toBeTruthy();
    expect(screen.getByText("63%")).toBeTruthy();
  });

  it("returns null when no CLI client can be resolved", () => {
    seedStore({
      sessionId: "session-missing-client",
      usageLimits: usageByClient([
        [
          "cli-unused",
          {
            claude: usage(0.99),
          },
        ],
      ]),
    });

    const { container } = render(<UsageLimitsPanel />);

    expect(container.textContent).toBe("");
  });

  it.each([
    [
      "DeepSeek",
      {
        deepseek: usage(0.22),
        codex: usage(0.44),
        claude: usage(0.55),
      },
      "DeepSeek Usage",
      "22%",
    ],
    [
      "Codex",
      {
        codex: usage(0.44),
        claude: usage(0.55),
      },
      "Codex Usage",
      "44%",
    ],
    [
      "Claude",
      {
        claude: usage(0.55),
      },
      "Claude Usage",
      "55%",
    ],
  ] satisfies Array<[string, UsageLimitsByProvider, string, string]>)(
    "chooses %s by provider priority",
    (_provider, limitsByProvider, expectedLabel, expectedPercent) => {
      seedStore({
        sessionId: "session-priority",
        cliClients: [cliClient("cli-priority", "session-priority")],
        usageLimits: usageByClient([["cli-priority", limitsByProvider]]),
      });

      render(<UsageLimitsPanel />);

      expect(screen.getByText(expectedLabel)).toBeTruthy();
      expect(screen.getByText(expectedPercent)).toBeTruthy();
    },
  );
});
