import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UsageLimitsDisplay } from "./UsageLimits";
import type { UsageLimits } from "@/lib/store";

const NOW = new Date("2026-06-17T12:00:00Z");

function renderUsage(limits: UsageLimits, compact = false) {
  return render(<UsageLimitsDisplay limits={limits} compact={compact} />);
}

describe("UsageLimitsDisplay", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
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
