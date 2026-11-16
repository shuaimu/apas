import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useConfirmDialog, type ConfirmRequest } from "./ConfirmDialog";

afterEach(cleanup);

function Harness({ request }: { request: ConfirmRequest }) {
  const { requestConfirm, confirmDialog } = useConfirmDialog();
  return (
    <main>
      <button type="button" onClick={() => requestConfirm(request)}>Open</button>
      {confirmDialog}
    </main>
  );
}

function open(onConfirm = vi.fn(), extra: Partial<ConfirmRequest> = {}) {
  render(<Harness request={{ title: "Stop it?", body: <p>It stops.</p>, confirmLabel: "Stop", tone: "danger", onConfirm, ...extra }} />);
  const opener = screen.getByRole("button", { name: "Open" });
  opener.focus();
  fireEvent.click(opener);
  return { opener, onConfirm };
}

describe("useConfirmDialog", () => {
  it("opens on Cancel, named by its title or an explicit label", () => {
    open(vi.fn(), { ariaLabel: "Stop the thing" });
    const dialog = screen.getByRole("dialog", { name: "Stop the thing" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
  });

  it("cancels on Escape and returns focus to the opener", () => {
    const { opener, onConfirm } = open();
    fireEvent.keyDown(screen.getByRole("dialog", { name: "Stop it?" }), { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onConfirm).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(opener);
  });

  it("keeps focus inside when its text is clicked, so Escape still cancels", () => {
    const { onConfirm } = open();
    const dialog = screen.getByRole("dialog");
    expect(dialog.getAttribute("tabindex")).toBe("-1");
    dialog.focus();
    expect(dialog.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("cancels on a press on the backdrop, not on the panel", () => {
    const { onConfirm } = open();
    const dialog = screen.getByRole("dialog");
    fireEvent.mouseDown(dialog);
    expect(screen.getByRole("dialog")).toBeTruthy();
    fireEvent.mouseDown(dialog.parentElement!);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("keeps Tab and Shift+Tab inside the dialog", () => {
    open();
    const dialog = screen.getByRole("dialog");
    const cancel = screen.getByRole("button", { name: "Cancel" });
    const confirm = screen.getByRole("button", { name: "Stop" });
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(confirm);
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
    fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(confirm);
  });

  // Focus is back on the opener before the action runs, so whatever the
  // action opens next starts from there.
  it("closes, returns focus, then runs the action", () => {
    let focusWhenRun: Element | null = null;
    const onConfirm = vi.fn(() => { focusWhenRun = document.activeElement; });
    const { opener } = open(onConfirm);
    act(() => { fireEvent.click(screen.getByRole("button", { name: "Stop" })); });
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(focusWhenRun).toBe(opener);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
