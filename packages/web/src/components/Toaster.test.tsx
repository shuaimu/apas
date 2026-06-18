import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore, type Toast, type ToastKind } from "@/lib/store";
import { Toaster } from "./Toaster";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();

function toast(id: string, kind: ToastKind, message: string): Toast {
  return { id, kind, message };
}

function renderToaster(toasts: Toast[]) {
  act(() => {
    useStore.setState({ toasts });
  });

  return render(<Toaster />);
}

describe("Toaster", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders success, error, and info toasts as status entries", () => {
    renderToaster([
      toast("success", "success", "Project saved"),
      toast("error", "error", "Failed to start worker"),
      toast("info", "info", "Review requested"),
    ]);

    const statuses = screen.getAllByRole("status");

    expect(statuses).toHaveLength(3);
    expect(statuses[0].textContent).toContain("Project saved");
    expect(statuses[0].className).toContain("bg-green-500");
    expect(statuses[1].textContent).toContain("Failed to start worker");
    expect(statuses[1].className).toContain("bg-red-500");
    expect(statuses[2].textContent).toContain("Review requested");
    expect(statuses[2].className).toContain("bg-gray-800");
  });

  it("dismisses only the clicked toast through store state", () => {
    renderToaster([
      toast("first", "success", "First toast"),
      toast("second", "error", "Second toast"),
      toast("third", "info", "Third toast"),
    ]);

    fireEvent.click(screen.getAllByRole("button", { name: "Dismiss" })[1]);

    expect(useStore.getState().toasts.map((item) => item.id)).toEqual([
      "first",
      "third",
    ]);
    expect(screen.getByText("First toast")).toBeTruthy();
    expect(screen.queryByText("Second toast")).toBeNull();
    expect(screen.getByText("Third toast")).toBeTruthy();
  });

  it("auto-dismisses mounted toasts after 3000ms", () => {
    vi.useFakeTimers();
    renderToaster([toast("auto", "info", "Auto dismiss me")]);

    act(() => {
      vi.advanceTimersByTime(2999);
    });
    expect(screen.getByText("Auto dismiss me")).toBeTruthy();
    expect(useStore.getState().toasts).toHaveLength(1);

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(useStore.getState().toasts).toEqual([]);
    expect(screen.queryByText("Auto dismiss me")).toBeNull();
  });

  it("clears auto-dismiss timers on unmount", () => {
    vi.useFakeTimers();
    const dismissToast = vi.fn();

    act(() => {
      useStore.setState({
        dismissToast: dismissToast as StoreState["dismissToast"],
        toasts: [toast("cleanup", "success", "Unmount before timeout")],
      });
    });

    const { unmount } = render(<Toaster />);

    unmount();
    act(() => {
      vi.advanceTimersByTime(3000);
    });

    expect(dismissToast).not.toHaveBeenCalled();
  });
});
