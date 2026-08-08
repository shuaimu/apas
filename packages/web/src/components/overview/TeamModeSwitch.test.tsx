import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useStore } from "@/lib/store";
import { TeamModeSwitch } from "./TeamModeSwitch";

const initialStore = useStore.getState();
const sessionId = "session-a";

function seed(teamAvailable: boolean, projectSuspended = false, version = 7) {
  act(() => {
    useStore.setState({
      sessionId,
      projectPolicies: {
        [sessionId]: {
          teamAvailable,
          allowedLaunchProfiles: [],
          version,
          projectSuspended,
          noncompliantPaneIds: [],
        },
      },
    });
  });
}

afterEach(() => {
  act(() => useStore.setState(initialStore, true));
});

describe("TeamModeSwitch", () => {
  it("shows an authoritative available policy as read-only", () => {
    seed(true);
    render(<TeamModeSwitch />);

    expect(screen.getByText("Available")).toBeTruthy();
    expect(screen.getByText(/allowed by the current cluster policy/)).toBeTruthy();
    expect(screen.getByText(/Cluster policy v7/)).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows unavailable when team launch is disabled", () => {
    seed(false);
    render(<TeamModeSwitch />);

    expect(screen.getByText("Unavailable")).toBeTruthy();
    expect(screen.getByText(/disabled by the current cluster policy/)).toBeTruthy();
  });

  it("explains project suspension even if team launch is otherwise allowed", () => {
    seed(true, true);
    render(<TeamModeSwitch />);

    expect(screen.getByText("Unavailable")).toBeTruthy();
    expect(screen.getByText(/project is suspended/)).toBeTruthy();
  });

  it("fails closed while the policy snapshot is pending", () => {
    act(() => useStore.setState({ sessionId, projectPolicies: {} }));
    render(<TeamModeSwitch />);

    expect(screen.getByText("Checking")).toBeTruthy();
    expect(screen.getByText("Waiting for the project host")).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();
  });
});
