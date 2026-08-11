import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneWorkSummaryKey, useStore } from "@/lib/store";
import { formatSummaryWindow, PaneWorkSummaryDrawer } from "./PaneWorkSummaryDrawer";

const SID = "11111111-1111-4111-8111-111111111111";
const initialStore = useStore.getInitialState();

describe("PaneWorkSummaryDrawer", () => {
  beforeEach(() => {
    act(() => useStore.setState(initialStore, true));
  });

  afterEach(() => vi.restoreAllMocks());

  it("requests only its pane and renders cached status", () => {
    const listPaneWorkSummaries = vi.fn(() => true);
    act(() => useStore.setState({
      listPaneWorkSummaries,
      paneWorkSummaries: {
        [paneWorkSummaryKey(SID, 4)]: {
          availability: "available",
          loading: false,
          summaries: [{
            protocolVersion: 1,
            sessionId: SID,
            paneId: 4,
            windowStart: "2026-08-11T03:00:00Z",
            windowEnd: "2026-08-11T06:00:00Z",
            windowKind: "completed",
            status: "complete",
            summary: "Implemented the desktop drawer and verified its pane-scoped behavior.",
            sourceDigest: "digest",
            sourceMessageCount: 5,
            attempts: 1,
          }],
        },
      },
    }));
    render(<PaneWorkSummaryDrawer sessionId={SID} paneId={4} paneLabel="Worker" onClose={vi.fn()} />);
    expect(listPaneWorkSummaries).toHaveBeenCalledWith(SID, 4, true);
    expect(screen.getByText(/Implemented the desktop drawer/)).toBeTruthy();
    expect(screen.getByText("Complete")).toBeTruthy();
  });

  it("formats shared UTC boundaries in an explicit local time zone", () => {
    expect(formatSummaryWindow(
      "2026-08-11T03:00:00Z",
      "2026-08-11T06:00:00Z",
      "en-US",
      "America/New_York",
    )).toContain("Aug 10");
  });

  it("switches pane content atomically and retries only the failed window", () => {
    const refreshPaneWorkSummary = vi.fn(() => true);
    const listPaneWorkSummaries = vi.fn(() => true);
    act(() => useStore.setState({
      listPaneWorkSummaries,
      refreshPaneWorkSummary,
      paneWorkSummaries: {
        [paneWorkSummaryKey(SID, 4)]: {
          availability: "available",
          loading: false,
          summaries: [{
            protocolVersion: 1,
            sessionId: SID,
            paneId: 4,
            windowStart: "2026-08-11T03:00:00Z",
            windowEnd: "2026-08-11T06:00:00Z",
            windowKind: "completed",
            status: "failed",
            sourceDigest: "failed",
            sourceMessageCount: 4,
            attempts: 3,
            error: "Provider quota exceeded",
          }],
        },
        [paneWorkSummaryKey(SID, 5)]: {
          availability: "cli_update_required",
          loading: false,
          summaries: [],
        },
      },
    }));
    const view = render(
      <PaneWorkSummaryDrawer sessionId={SID} paneId={4} paneLabel="Four" onClose={vi.fn()} />,
    );
    expect(screen.getByText("Provider quota exceeded")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(refreshPaneWorkSummary).toHaveBeenCalledWith(
      SID,
      4,
      "2026-08-11T03:00:00Z",
    );

    view.rerender(
      <PaneWorkSummaryDrawer sessionId={SID} paneId={5} paneLabel="Five" onClose={vi.fn()} />,
    );
    expect(screen.queryByText("Provider quota exceeded")).toBeNull();
    expect(screen.getByText(/needs an update/)).toBeTruthy();
    expect(listPaneWorkSummaries).toHaveBeenLastCalledWith(SID, 5, true);
  });
});
