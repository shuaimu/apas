import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { PaneWorkSummary, PaneWorkSummaryAvailability } from "@/lib/store";
import {
  canRetrySummary,
  PaneWorkSummaryList,
  summaryAvailabilityMessage,
  summaryStatusLabel,
} from "./PaneWorkSummaryContent";

const SID = "11111111-1111-4111-8111-111111111111";

function summary(status: PaneWorkSummary["status"], index: number): PaneWorkSummary {
  return {
    protocolVersion: 1,
    sessionId: SID,
    paneId: 4,
    windowStart: `2026-08-${String(10 - index).padStart(2, "0")}T03:00:00Z`,
    windowEnd: `2026-08-${String(10 - index).padStart(2, "0")}T06:00:00Z`,
    windowKind: status === "partial" ? "current" : "completed",
    status,
    summary: ["complete", "partial", "stale"].includes(status) ? `${status} text` : undefined,
    sourceDigest: `${status}-${index}`,
    sourceMessageCount: index + 1,
    sourceThrough: status === "partial" ? "2026-08-09T04:30:00Z" : undefined,
    provider: status === "complete" ? "codex" : undefined,
    model: status === "complete" ? "gpt-test" : undefined,
    attempts: 1,
    error: status === "failed" ? "Provider failed" : undefined,
  };
}

describe("PaneWorkSummaryContent", () => {
  it("covers every status and allows retry only for failed windows", () => {
    const statuses: PaneWorkSummary["status"][] = [
      "complete", "partial", "queued", "generating", "stale", "failed", "source_expired",
    ];
    const summaries = statuses.map(summary);
    const onRetry = vi.fn();
    render(<PaneWorkSummaryList cache={{ summaries, availability: "available", loading: false }} onRetry={onRetry} />);

    for (const item of summaries) expect(screen.getByText(summaryStatusLabel(item))).toBeTruthy();
    expect(screen.getByText(/Through/)).toBeTruthy();
    expect(screen.getByText(/via codex · gpt-test/)).toBeTruthy();
    expect(screen.getByText(/Waiting for the project summarizer/)).toBeTruthy();
    expect(screen.getByText(/Summarizing retained activity/)).toBeTruthy();
    expect(screen.getByText(/Later activity changed/)).toBeTruthy();
    expect(screen.getByText(/cannot be reconstructed/)).toBeTruthy();
    expect(summaries.map(canRetrySummary)).toEqual([false, false, false, false, false, true, false]);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledWith(summaries[5].windowStart);
  });

  it("provides explicit copy for every unavailable state", () => {
    const unavailable: PaneWorkSummaryAvailability[] = [
      "cli_update_required", "summarizer_disabled", "summarizer_unavailable", "unknown",
    ];
    for (const availability of unavailable) {
      expect(summaryAvailabilityMessage(availability)).toBeTruthy();
    }
    expect(summaryAvailabilityMessage("available")).toBeNull();
  });
});
