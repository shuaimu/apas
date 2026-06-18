import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ResizeHandle } from "./ResizeHandle";

function renderHandle(
  direction: "horizontal" | "vertical",
  onResize = vi.fn(),
  onResizeEnd = vi.fn(),
) {
  render(
    <ResizeHandle
      direction={direction}
      onResize={onResize}
      onResizeEnd={onResizeEnd}
    />,
  );

  return {
    handle: screen.getByTitle("Drag to resize"),
    onResize,
    onResizeEnd,
  };
}

describe("ResizeHandle", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  });

  it("reports successive horizontal mouse deltas and cleans up after mouseup", () => {
    const { handle, onResize, onResizeEnd } = renderHandle("horizontal");

    fireEvent.mouseDown(handle, { clientX: 100 });

    expect(document.body.style.userSelect).toBe("none");
    expect(document.body.style.cursor).toBe("col-resize");

    fireEvent.mouseMove(document, { clientX: 115 });
    fireEvent.mouseMove(document, { clientX: 105 });

    expect(onResize).toHaveBeenNthCalledWith(1, 15);
    expect(onResize).toHaveBeenNthCalledWith(2, -10);
    expect(onResizeEnd).not.toHaveBeenCalled();

    fireEvent.mouseUp(document);

    expect(onResizeEnd).toHaveBeenCalledOnce();
    expect(document.body.style.userSelect).toBe("");
    expect(document.body.style.cursor).toBe("");

    fireEvent.mouseMove(document, { clientX: 130 });
    expect(onResize).toHaveBeenCalledTimes(2);
  });

  it("uses vertical touch coordinates for resize deltas", () => {
    const { handle, onResize, onResizeEnd } = renderHandle("vertical");

    fireEvent.touchStart(handle, {
      touches: [{ clientX: 10, clientY: 40 }],
    });

    expect(document.body.style.userSelect).toBe("none");
    expect(document.body.style.cursor).toBe("row-resize");

    fireEvent.touchMove(document, {
      touches: [{ clientX: 200, clientY: 55 }],
    });
    fireEvent.touchMove(document, {
      touches: [{ clientX: 400, clientY: 50 }],
    });

    expect(onResize).toHaveBeenNthCalledWith(1, 15);
    expect(onResize).toHaveBeenNthCalledWith(2, -5);

    fireEvent.touchEnd(document);

    expect(onResizeEnd).toHaveBeenCalledOnce();
    expect(document.body.style.userSelect).toBe("");
    expect(document.body.style.cursor).toBe("");
  });

  it("removes document listeners and restores body styles when unmounted during a drag", () => {
    const addEventListener = vi.spyOn(document, "addEventListener");
    const removeEventListener = vi.spyOn(document, "removeEventListener");
    const onResize = vi.fn();
    const onResizeEnd = vi.fn();
    const { unmount } = render(
      <ResizeHandle
        direction="horizontal"
        onResize={onResize}
        onResizeEnd={onResizeEnd}
      />,
    );

    fireEvent.mouseDown(screen.getByTitle("Drag to resize"), { clientX: 24 });

    expect(addEventListener).toHaveBeenCalledWith("mousemove", expect.any(Function));
    expect(addEventListener).toHaveBeenCalledWith("mouseup", expect.any(Function));
    expect(addEventListener).toHaveBeenCalledWith("touchmove", expect.any(Function));
    expect(addEventListener).toHaveBeenCalledWith("touchend", expect.any(Function));
    expect(document.body.style.userSelect).toBe("none");
    expect(document.body.style.cursor).toBe("col-resize");

    unmount();

    expect(removeEventListener).toHaveBeenCalledWith("mousemove", expect.any(Function));
    expect(removeEventListener).toHaveBeenCalledWith("mouseup", expect.any(Function));
    expect(removeEventListener).toHaveBeenCalledWith("touchmove", expect.any(Function));
    expect(removeEventListener).toHaveBeenCalledWith("touchend", expect.any(Function));
    expect(document.body.style.userSelect).toBe("");
    expect(document.body.style.cursor).toBe("");

    fireEvent.mouseMove(document, { clientX: 40 });
    expect(onResize).not.toHaveBeenCalled();
    expect(onResizeEnd).not.toHaveBeenCalled();
  });
});
