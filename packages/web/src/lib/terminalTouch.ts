import type { Terminal } from "@xterm/xterm";

/** xterm 6.0 loses native touch scrolling. Keep this adapter on public APIs;
 * let xterm encode application mouse events according to the restored modes. */
export function installTerminalTouch(term: Terminal): () => void {
  const element = term.element;
  if (!element) return () => {};
  let start:
    | { x: number; y: number; lastY: number; remainder: number }
    | undefined;
  const onStart = (event: TouchEvent) => {
    const touch = event.touches[0];
    start =
      event.touches.length === 1
        ? {
            x: touch.clientX,
            y: touch.clientY,
            lastY: touch.clientY,
            remainder: 0,
          }
        : undefined;
  };
  const onMove = (event: TouchEvent) => {
    if (!start || event.touches.length !== 1) {
      start = undefined;
      return;
    }
    const touch = event.touches[0];
    if (
      Math.abs(touch.clientY - start.y) < 6 ||
      Math.abs(touch.clientX - start.x) > Math.abs(touch.clientY - start.y)
    )
      return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const screen = element.querySelector(".xterm-screen") ?? element;
    const lineHeight = screen.getBoundingClientRect().height / term.rows || 18;
    const delta = start.remainder + (start.lastY - touch.clientY) / lineHeight;
    const lines = Math.trunc(delta);
    start.remainder = delta - lines;
    start.lastY = touch.clientY;
    if (!lines) return;
    if (
      term.buffer.active.type === "normal" &&
      term.modes.mouseTrackingMode === "none"
    ) {
      term.scrollLines(lines);
    } else {
      screen.dispatchEvent(
        new WheelEvent("wheel", {
          bubbles: true,
          cancelable: true,
          clientX: touch.clientX,
          clientY: touch.clientY,
          deltaY: lines,
          deltaMode: 1,
        }),
      );
    }
  };
  const onEnd = () => {
    start = undefined;
  };
  element.addEventListener("touchstart", onStart, {
    passive: true,
    capture: true,
  });
  element.addEventListener("touchmove", onMove, {
    passive: false,
    capture: true,
  });
  element.addEventListener("touchend", onEnd, true);
  element.addEventListener("touchcancel", onEnd, true);
  return () => {
    element.removeEventListener("touchstart", onStart, true);
    element.removeEventListener("touchmove", onMove, true);
    element.removeEventListener("touchend", onEnd, true);
    element.removeEventListener("touchcancel", onEnd, true);
  };
}
