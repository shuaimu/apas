import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useStore } from "@/lib/store";
import { AllowedTabTypesCard } from "./AllowedTabTypesCard";

const initialStore = useStore.getState();
const sessionId = "session-a";

function seed(allowedLaunchProfiles: string[], noncompliantPaneIds: number[] = []) {
  act(() => {
    useStore.setState({
      sessionId,
      projectPolicies: {
        [sessionId]: {
          teamAvailable: true,
          allowedLaunchProfiles,
          version: 4,
          projectSuspended: false,
          noncompliantPaneIds,
        },
      },
    });
  });
}

afterEach(() => {
  act(() => useStore.setState(initialStore, true));
});

describe("AllowedTabTypesCard", () => {
  it("renders the exact server-authoritative launch profiles", () => {
    seed([
      "agent:claude:official:default",
      "terminal:codex:official:default",
    ]);
    render(<AllowedTabTypesCard />);

    expect(screen.getByText("agent:claude:official:default")).toBeTruthy();
    expect(screen.getByText("terminal:codex:official:default")).toBeTruthy();
    expect(screen.getByText("Read-only cluster policy")).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("reports that no panes may launch for an empty allowlist", () => {
    seed([]);
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/No new panes may be launched/)).toBeTruthy();
  });

  it("reports running panes that became noncompliant", () => {
    seed(["agent:codex:official:default"], [2, 9]);
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/Running panes 2, 9 are noncompliant/)).toBeTruthy();
  });

  it("waits for an authoritative snapshot instead of showing legacy controls", () => {
    act(() => useStore.setState({ sessionId, projectPolicies: {} }));
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/Waiting for an authoritative policy snapshot/)).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });
});
