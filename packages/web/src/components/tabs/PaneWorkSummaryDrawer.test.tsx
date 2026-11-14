import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneWorkSummaryKey, useStore } from "@/lib/store";
import { formatSummaryWindow, PaneWorkSummaryDrawer } from "./PaneWorkSummaryDrawer";

const SID = "11111111-1111-4111-8111-111111111111";
const initialStore = useStore.getInitialState();

describe("PaneWorkSummaryDrawer", () => {
  beforeEach(() => {
    localStorage.clear();
    act(() => useStore.setState(initialStore, true));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    localStorage.clear();
  });

  it("widens when dragged left, keeps room for the conversation, and remembers the width", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ width: 1000 } as DOMRect);
    const list = vi.fn(() => true);
    act(() => useStore.setState({ listPaneWorkSummaries: list }));
    const props = { sessionId: SID, paneId: 4, paneLabel: "Worker", onClose: vi.fn() };
    const view = render(<PaneWorkSummaryDrawer {...props} />);
    const drawer = screen.getByRole("complementary");
    const handle = screen.getByTitle("Drag to resize");

    fireEvent.mouseDown(handle, { clientX: 700 });
    fireEvent.mouseMove(document, { clientX: 500 });
    expect(drawer.style.width).toBe("616px");
    fireEvent.mouseMove(document, { clientX: 100 });
    expect(drawer.style.width).toBe("680px");
    fireEvent.mouseMove(document, { clientX: 1200 });
    expect(drawer.style.width).toBe("280px");
    fireEvent.mouseMove(document, { clientX: 950 });
    fireEvent.mouseUp(document);
    expect(drawer.style.width).toBe("530px");
    expect(document.body.style.userSelect).toBe("");
    expect(list).toHaveBeenCalledTimes(1);

    view.rerender(<PaneWorkSummaryDrawer {...props} paneId={5} paneLabel="Five" />);
    expect(screen.getByRole("complementary").style.width).toBe("530px");
    view.unmount();
    render(<PaneWorkSummaryDrawer {...props} />);
    expect(screen.getByRole("complementary").style.width).toBe("530px");
  });

  it("supports keyboard sizing and fits a shrinking workspace without losing the preferred width", () => {
    let availableWidth = 1200;
    let observeResize = () => {};
    const disconnect = vi.fn();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => ({ width: availableWidth } as DOMRect));
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { observeResize = callback; }
      observe() {}
      disconnect = disconnect;
    });
    const view = render(<PaneWorkSummaryDrawer sessionId={SID} paneId={4} paneLabel="Worker" onClose={vi.fn()} />);
    const separator = screen.getByRole("separator", { name: "Resize work summary" });
    const drawer = screen.getByRole("complementary");
    expect(separator.tabIndex).toBe(0);
    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(drawer.style.width).toBe("432px");
    fireEvent.keyDown(separator, { key: "ArrowRight", shiftKey: true });
    expect(drawer.style.width).toBe("384px");
    fireEvent.keyDown(separator, { key: "End" });
    expect(drawer.style.width).toBe("800px");
    expect(separator.getAttribute("aria-valuenow")).toBe("800");

    availableWidth = 500;
    act(() => observeResize());
    expect(drawer.style.width).toBe("250px");
    expect(separator.getAttribute("aria-valuemax")).toBe("250");
    availableWidth = 1200;
    act(() => observeResize());
    expect(drawer.style.width).toBe("800px");
    fireEvent.keyDown(separator, { key: "Home" });
    expect(drawer.style.width).toBe("280px");
    view.unmount();
    expect(disconnect).toHaveBeenCalledOnce();
  });

  it("ignores invalid saved widths and allows resizing when storage is blocked", () => {
    localStorage.setItem("apas_layout_global_summary_width", "Infinity");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Storage blocked"); });
    render(<PaneWorkSummaryDrawer sessionId={SID} paneId={4} paneLabel="Worker" onClose={vi.fn()} />);
    const drawer = screen.getByRole("complementary");
    expect(drawer.style.width).toBe("416px");
    fireEvent.keyDown(screen.getByRole("separator"), { key: "ArrowLeft" });
    expect(drawer.style.width).toBe("432px");
  });

  it("requests only its pane and renders cached status", () => {
    const listPaneWorkSummaries = vi.fn(() => true);
    const refreshPaneWorkSummary = vi.fn(() => true);
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
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(refreshPaneWorkSummary).toHaveBeenCalledWith(SID, 4);
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
