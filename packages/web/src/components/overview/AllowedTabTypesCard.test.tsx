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
  it("names each server-authoritative launch profile and keeps its key as the tooltip", () => {
    seed([
      "agent:claude:official:default",
      "terminal:codex:official:default",
      "terminal:claude:deepseek:deepseek-flash",
    ]);
    render(<AllowedTabTypesCard />);

    expect(screen.getByTitle("agent:claude:official:default").textContent).toBe("Claude / Official (structured agent)");
    expect(screen.getByTitle("terminal:codex:official:default").textContent).toBe("Codex / Official");
    expect(screen.getByTitle("terminal:claude:deepseek:deepseek-flash").textContent).toBe("Claude / DeepSeek 4.1 Flash");
    expect(screen.getByText("Read-only cluster policy")).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows a profile the catalog cannot name by its key, and leaves out a retired one", () => {
    seed([
      "terminal:nova:official:default",
      "terminal:claude:minimax:m2",
      "terminal:omp:official:default",
    ]);
    render(<AllowedTabTypesCard />);

    expect(screen.getByText("terminal:nova:official:default")).toBeTruthy();
    expect(screen.getByTitle("terminal:omp:official:default").textContent).toBe("OMP");
    expect(screen.queryByText(/minimax/)).toBeNull();
    expect(screen.queryByTitle(/minimax/)).toBeNull();
  });

  it("reports that no tabs may launch for an empty allowlist", () => {
    seed([]);
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/No new tabs may be launched/)).toBeTruthy();
  });

  it("names panes outside the policy without claiming they are stuck", () => {
    // The allowlist governs what may be created. These panes run and relaunch;
    // all that is true of them is that their combination cannot be chosen for
    // something new.
    seed(["agent:codex:official:default"], [2, 9]);
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/Tabs 2, 9 use a combination this policy no longer allows/)).toBeTruthy();
    expect(screen.getByText(/can be relaunched/)).toBeTruthy();
    expect(screen.queryByText(/cannot be relaunched/)).toBeNull();
  });

  it("waits for an authoritative snapshot instead of showing legacy controls", () => {
    act(() => useStore.setState({ sessionId, projectPolicies: {} }));
    render(<AllowedTabTypesCard />);
    expect(screen.getByText(/Waiting for an authoritative policy snapshot/)).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });
});
