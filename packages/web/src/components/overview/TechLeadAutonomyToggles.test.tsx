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

describe("TechLeadAutonomyToggles", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState({
        sessionId: null,
        projectFlags: {},
        updateProjectFlags: initialStore.updateProjectFlags as StoreState["updateProjectFlags"],
      });
    });
  });

  it("defaults both autonomy toggles off when the active session has no flags", () => {
    act(() => {
      useStore.setState({ sessionId: "session-without-flags", projectFlags: {} });
    });

    render(<TechLeadAutonomyToggles />);

    expect(autoApproveCheckbox().checked).toBe(false);
    expect(autoMergeCheckbox().checked).toBe(false);
  });

  it("reflects ProjectFlags for the active session only", () => {
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        projectFlags: {
          "session-a": { autoApproveTodos: true, autoMergePrs: false },
          "session-b": { autoApproveTodos: false, autoMergePrs: true },
        },
      });
    });

    render(<TechLeadAutonomyToggles />);

    expect(autoApproveCheckbox().checked).toBe(true);
    expect(autoMergeCheckbox().checked).toBe(false);
  });

  it("sends both current booleans when either checkbox changes", () => {
    const updateProjectFlags = vi.fn();
    act(() => {
      useStore.setState({
        sessionId: "session-a",
        projectFlags: {
          "session-a": { autoApproveTodos: true, autoMergePrs: false },
        },
        updateProjectFlags: updateProjectFlags as StoreState["updateProjectFlags"],
      });
    });

    render(<TechLeadAutonomyToggles />);

    fireEvent.click(autoMergeCheckbox());
    expect(updateProjectFlags).toHaveBeenCalledWith({
      autoApproveTodos: true,
      autoMergePrs: true,
    });

    fireEvent.click(autoApproveCheckbox());
    expect(updateProjectFlags).toHaveBeenLastCalledWith({
      autoApproveTodos: false,
      autoMergePrs: false,
    });
  });
});
