import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { SessionPaneSummary } from "@/lib/store";
import { PaneStatusDots } from "./PaneStatusDots";

function pane(overrides: Partial<SessionPaneSummary> & Pick<SessionPaneSummary, "pane_id">): SessionPaneSummary {
  return { kind: "terminal", provider: "omp", ...overrides };
}

function dot(label: string): HTMLElement {
  return screen.getByRole("img", { name: label });
}

describe("PaneStatusDots and autopilot", () => {
  it("keeps the dot's colour and status, and adds a ring and \", on autopilot\"", () => {
    render(
      <PaneStatusDots
        isActive
        panes={[
          pane({
            pane_id: 1,
            label: "Nightly",
            is_working: false,
            autopilot: { support: "supported", condition: "waiting", detail: "next_iteration" },
          }),
          pane({
            pane_id: 2,
            label: "Builder",
            is_working: true,
            autopilot: { support: "supported", condition: "running" },
          }),
          pane({
            pane_id: 3,
            label: "Asker",
            awaiting_answer: true,
            autopilot: { support: "supported", condition: "running" },
          }),
        ]}
      />,
    );

    const idle = dot("Nightly: Idle, on autopilot");
    expect(idle.className).toContain("bg-gray-400");
    expect(idle.className).toContain("outline-indigo-500");
    expect(idle.getAttribute("data-autopilot")).toBe("on");
    expect(dot("Builder: Working, on autopilot").className).toContain("bg-blue-500");
    // Pending answer keeps precedence as the status; autopilot is beside it.
    expect(dot("Asker: Pending answer, on autopilot").className).toContain("bg-red-500");
  });

  it("leaves a tab that is not on autopilot exactly as it was", () => {
    render(
      <PaneStatusDots
        isActive
        panes={[
          pane({ pane_id: 1, label: "Plain", is_working: false }),
          pane({ pane_id: 2, label: "Rebootable", autopilot: { support: "needs_agent_reboot" } }),
          pane({ pane_id: 3, label: "Paused", autopilot: { support: "supported", condition: "paused", detail: "failures" } }),
          pane({ pane_id: 4, label: "Done", autopilot: { support: "supported", condition: "ended", detail: "iteration_limit" } }),
          pane({ pane_id: 5, autopilot: null }),
        ]}
      />,
    );
    for (const label of ["Plain: Idle", "Rebootable: Idle", "Paused: Idle", "Done: Idle", "Terminal 5: Idle"]) {
      const element = dot(label);
      expect(element.className).toBe("h-1.5 w-1.5 shrink-0 rounded-full bg-gray-400");
      expect(element.hasAttribute("data-autopilot")).toBe(false);
    }
  });

  it("says nothing about autopilot for a tab that is not running", () => {
    const onAutopilot = pane({
      pane_id: 1,
      label: "Nightly",
      hostname: "zoo-006",
      machine_id: "m6",
      autopilot: { support: "supported", condition: "running" },
    });
    const { unmount } = render(<PaneStatusDots isActive={false} panes={[onAutopilot]} />);
    expect(dot("Nightly: Not running").hasAttribute("data-autopilot")).toBe(false);
    unmount();

    // Another machine keeps the project active; this tab's machine does not run it.
    render(
      <PaneStatusDots
        isActive
        panes={[onAutopilot]}
        instances={[{ machineId: "m5", hostname: "zoo-005", primary: true }]}
      />,
    );
    expect(dot("Nightly: Not running on zoo-006").hasAttribute("data-autopilot")).toBe(false);
  });
});
