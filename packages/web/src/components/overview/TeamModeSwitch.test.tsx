import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { TeamModeSwitch } from "./TeamModeSwitch";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

const OFF = { autoApproveTodos: false, autoMergePrs: false, teamEnabled: false };
const ON = { ...OFF, teamEnabled: true };

function switchEl(): HTMLButtonElement {
  return screen.getByRole("switch") as HTMLButtonElement;
}

function seedSession(overrides: Record<string, unknown> = {}) {
  return [
    {
      id: "session-a",
      projectId: "session-a",
      status: "active",
      isShared: false,
      ...overrides,
    },
  ] as StoreState["sessions"];
}

function seed(flags = OFF, sessionOverrides: Record<string, unknown> = {}, mock?: unknown) {
  act(() => {
    useStore.setState({
      sessionId: "session-a",
      sessions: seedSession(sessionOverrides),
      projectFlags: { "session-a": flags },
      ...(mock ? { updateProjectFlags: mock as StoreState["updateProjectFlags"] } : {}),
    });
  });
}

describe("TeamModeSwitch", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState({
        sessionId: null,
        sessions: [],
        projectFlags: {},
        updateProjectFlags: initialStore.updateProjectFlags as StoreState["updateProjectFlags"],
      });
    });
  });

  it("reports the current state to assistive tech and in text", () => {
    seed(ON);
    render(<TeamModeSwitch />);
    expect(switchEl().getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("On")).toBeTruthy();
  });

  it("shows off, and explains what is unavailable", () => {
    // With team mode off this component is the only thing on the page that
    // says why everything else is missing, so the explanation has to be here.
    seed(OFF);
    render(<TeamModeSwitch />);
    expect(switchEl().getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText("Off")).toBeTruthy();
    expect(screen.getByText(/unavailable for this project/)).toBeTruthy();
  });

  it("turning ON needs no confirmation and preserves the other flags", () => {
    const updateProjectFlags = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    seed({ autoApproveTodos: true, autoMergePrs: true, teamEnabled: false }, {}, updateProjectFlags);

    render(<TeamModeSwitch />);
    fireEvent.click(switchEl());

    expect(confirm).not.toHaveBeenCalled();
    // The wire message carries all three flags; dropping the autonomy ones
    // here would silently reset them.
    expect(updateProjectFlags).toHaveBeenCalledWith({
      autoApproveTodos: true,
      autoMergePrs: true,
      teamEnabled: true,
    });
  });

  it("turning OFF asks first, because it stops running panes", () => {
    const updateProjectFlags = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    seed(ON, {}, updateProjectFlags);

    render(<TeamModeSwitch />);
    fireEvent.click(switchEl());

    expect(confirm).toHaveBeenCalled();
    expect(updateProjectFlags).not.toHaveBeenCalled();
  });

  it("turning OFF proceeds once confirmed", () => {
    const updateProjectFlags = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    seed(ON, {}, updateProjectFlags);

    render(<TeamModeSwitch />);
    fireEvent.click(switchEl());

    expect(updateProjectFlags).toHaveBeenCalledWith({ ...OFF, teamEnabled: false });
  });

  describe("owner/admin gating", () => {
    it.each([
      ["owner", true],
      ["admin", true],
      ["user", false],
    ])("shared project role %s can toggle: %s", (role, expected) => {
      seed(OFF, { isShared: true, shareRole: role });
      render(<TeamModeSwitch />);
      expect(switchEl().disabled).toBe(!expected);
    });

    it("a plain user sees state but cannot change it", () => {
      const updateProjectFlags = vi.fn();
      seed(ON, { isShared: true, shareRole: "user" }, updateProjectFlags);

      render(<TeamModeSwitch />);
      fireEvent.click(switchEl());

      expect(updateProjectFlags).not.toHaveBeenCalled();
      expect(screen.getByText(/Only the project owner or an admin/)).toBeTruthy();
      // Still readable — a user should know whether the team may run.
      expect(screen.getByText("On")).toBeTruthy();
    });

    it("fails closed while the session list is still loading", () => {
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: [],
          projectFlags: { "session-a": OFF },
        });
      });
      render(<TeamModeSwitch />);
      expect(switchEl().disabled).toBe(true);
    });
  });
});
