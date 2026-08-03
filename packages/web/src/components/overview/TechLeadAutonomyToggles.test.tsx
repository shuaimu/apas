import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { TechLeadAutonomyToggles } from "./TechLeadAutonomyToggles";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

function autoApproveCheckbox(): HTMLInputElement {
  return screen.getByLabelText(/Auto-approve TODOs/) as HTMLInputElement;
}

function autoMergeCheckbox(): HTMLInputElement {
  return screen.getByLabelText(/Auto-merge PRs/) as HTMLInputElement;
}

function teamModeCheckbox(): HTMLInputElement {
  return screen.getByLabelText(/Team mode/) as HTMLInputElement;
}

/**
 * These settings are owner/admin-only, and the component derives that from the
 * session list — so every test has to seed a session or the controls render
 * disabled. `isShared: false` means "nobody else is on this project", which is
 * the un-shared owner case.
 */
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

const OFF = { autoApproveTodos: false, autoMergePrs: false, teamEnabled: false };

describe("TechLeadAutonomyToggles", () => {
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

  it("defaults every toggle off when the active session has no flags", () => {
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        sessions: seedSession(),
        projectFlags: {},
      });
    });

    render(<TechLeadAutonomyToggles />);

    expect(teamModeCheckbox().checked).toBe(false);
    expect(autoApproveCheckbox().checked).toBe(false);
    expect(autoMergeCheckbox().checked).toBe(false);
  });

  it("reflects ProjectFlags for the active session only", () => {
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        sessions: seedSession(),
        projectFlags: {
          "session-a": { autoApproveTodos: true, autoMergePrs: false, teamEnabled: true },
          "session-b": { autoApproveTodos: false, autoMergePrs: true, teamEnabled: false },
        },
      });
    });

    render(<TechLeadAutonomyToggles />);

    expect(autoApproveCheckbox().checked).toBe(true);
    expect(autoMergeCheckbox().checked).toBe(false);
    expect(teamModeCheckbox().checked).toBe(true);
  });

  it("explains the auto-merge repository requirement and safety gates", () => {
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        sessions: seedSession(),
        projectFlags: {},
      });
    });

    render(<TechLeadAutonomyToggles />);

    const note = screen.getByText(/Requires GitHub auto-merge enabled/);
    expect(note.textContent).toContain("target repository");
    expect(note.textContent).toContain("Reviewer approval");
    expect(note.textContent).toContain("mergeability");
    expect(note.textContent).toContain("green/no-pending CI");
  });

  it("sends every current boolean when any checkbox changes", () => {
    const updateProjectFlags = vi.fn();
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        sessions: seedSession(),
        projectFlags: {
          "session-a": { autoApproveTodos: true, autoMergePrs: false, teamEnabled: true },
        },
        updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
      });
    });

    render(<TechLeadAutonomyToggles />);

    fireEvent.click(autoMergeCheckbox());
    expect(updateProjectFlags).toHaveBeenCalledWith({
      autoApproveTodos: true,
      autoMergePrs: true,
      teamEnabled: true,
    });

    fireEvent.click(autoApproveCheckbox());
    expect(updateProjectFlags).toHaveBeenLastCalledWith({
      autoApproveTodos: false,
      autoMergePrs: false,
      teamEnabled: true,
    });
  });

  describe("owner/admin gating", () => {
    it.each([
      ["owner", true],
      ["admin", true],
      ["user", false],
    ])("shared project role %s can manage: %s", (role, expected) => {
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: seedSession({ isShared: true, shareRole: role }),
          projectFlags: { "session-a": OFF },
        });
      });

      render(<TechLeadAutonomyToggles />);

      expect(teamModeCheckbox().disabled).toBe(!expected);
      expect(autoApproveCheckbox().disabled).toBe(!expected);
      expect(autoMergeCheckbox().disabled).toBe(!expected);
    });

    it("sends nothing when a plain user clicks a toggle", () => {
      const updateProjectFlags = vi.fn();
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: seedSession({ isShared: true, shareRole: "user" }),
          projectFlags: { "session-a": OFF },
          updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
        });
      });

      render(<TechLeadAutonomyToggles />);
      fireEvent.click(teamModeCheckbox());

      // The server rejects it too, but a control that silently no-ops is a
      // worse bug than a disabled one — assert the click really is inert.
      expect(updateProjectFlags).not.toHaveBeenCalled();
      expect(screen.getByText(/only the project owner or an admin/)).toBeTruthy();
    });

    it("hides the controls from a viewer whose session isn't loaded yet", () => {
      // Fails closed: briefly hiding a control from its owner is recoverable,
      // offering one to a plain user is not.
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: [],
          projectFlags: { "session-a": OFF },
        });
      });

      render(<TechLeadAutonomyToggles />);

      expect(teamModeCheckbox().disabled).toBe(true);
    });
  });

  describe("turning team mode off", () => {
    it("confirms first, because it stops running panes", () => {
      const updateProjectFlags = vi.fn();
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: seedSession(),
          projectFlags: {
            "session-a": { ...OFF, teamEnabled: true },
          },
          updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
        });
      });

      render(<TechLeadAutonomyToggles />);
      fireEvent.click(teamModeCheckbox());

      expect(confirm).toHaveBeenCalled();
      expect(updateProjectFlags).not.toHaveBeenCalled();
    });

    it("sends teamEnabled false once confirmed", () => {
      const updateProjectFlags = vi.fn();
      vi.spyOn(window, "confirm").mockReturnValue(true);
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: seedSession(),
          projectFlags: {
            "session-a": { ...OFF, teamEnabled: true },
          },
          updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
        });
      });

      render(<TechLeadAutonomyToggles />);
      fireEvent.click(teamModeCheckbox());

      expect(updateProjectFlags).toHaveBeenCalledWith({ ...OFF, teamEnabled: false });
    });

    it("turning it ON needs no confirmation — nothing is destroyed", () => {
      const updateProjectFlags = vi.fn();
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
      act(() => {
        useStore.setState({
          sessionId: "session-a",
          sessions: seedSession(),
          projectFlags: { "session-a": OFF },
          updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
        });
      });

      render(<TechLeadAutonomyToggles />);
      fireEvent.click(teamModeCheckbox());

      expect(confirm).not.toHaveBeenCalled();
      expect(updateProjectFlags).toHaveBeenCalledWith({ ...OFF, teamEnabled: true });
    });
  });
});
