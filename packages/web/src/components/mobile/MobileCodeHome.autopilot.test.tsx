import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo, SessionPaneSummary } from "@/lib/store";
import { MobileCodeHome, type MobileCodeHomeProps } from "./MobileCodeHome";

/**
 * Autopilot in the mobile-browser home: the project cards' pane dots and the
 * Idle sessions list, read from the pane summary's autopilot block as the
 * desktop sidebar reads it. The block never carries a prompt.
 */

function omp(pane_id: number, label: string, overrides: Partial<SessionPaneSummary> = {}): SessionPaneSummary {
  return { pane_id, label, kind: "terminal", provider: "omp", is_working: false, ...overrides };
}

function renderHome(panes: SessionPaneSummary[], overrides: Partial<SessionInfo> = {}) {
  const session: SessionInfo = {
    id: "session-a",
    projectId: "session-a",
    cliClientId: "cli-a",
    workingDir: "/repo/mako",
    hostname: "zoo-006",
    status: "active",
    isActive: true,
    panes,
    ...overrides,
  };
  const props: MobileCodeHomeProps = {
    // Not active: the list comes from the live session inventory alone.
    active: false,
    connected: true,
    machineListReceived: false,
    legacySessions: [session],
    token: "token",
    onAccount: vi.fn(),
    onManageMachines: vi.fn(),
    onOpenSession: vi.fn(),
    onRebootDaemon: vi.fn(),
  };
  render(<MobileCodeHome {...props} />);
  return props;
}

function row(label: string): HTMLElement {
  return screen.getByRole("button", { name: `Open ${label} in mako` });
}

function openIdleList() {
  fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("mobile All projects cards and autopilot", () => {
  it("labels the dot of a tab on autopilot and leaves the others as they were", () => {
    renderHome([
      omp(1, "Plain"),
      omp(2, "Nightly", {
        autopilot: { support: "supported", condition: "waiting", detail: "next_iteration", iterations: 2 },
      }),
      omp(3, "Busy", { is_working: true, autopilot: { support: "supported", condition: "running" } }),
      omp(4, "Failing", { autopilot: { support: "supported", condition: "paused", detail: "failures" } }),
      omp(5, "Finished", { autopilot: { support: "supported", condition: "ended", detail: "iteration_limit" } }),
    ]);
    const card = screen.getByRole("button", { name: "Open mako" });
    // Autopilot is a second attribute: each dot keeps its status word.
    expect(within(card).getByRole("img", { name: "Nightly: Idle, on autopilot" }).getAttribute("data-autopilot")).toBe("on");
    expect(within(card).getByRole("img", { name: "Busy: Working, on autopilot" }).getAttribute("data-autopilot")).toBe("on");
    for (const name of ["Plain: Idle", "Failing: Idle", "Finished: Idle"]) {
      expect(within(card).getByRole("img", { name }).hasAttribute("data-autopilot")).toBe(false);
    }
    // The card never shows a prompt, and autopilot is not a worker.
    expect(card.textContent).not.toMatch(/worker|bot/i);
  });

  it("keeps Pending answer first on the dot of a tab on autopilot that asked a question", () => {
    renderHome([
      omp(1, "Asker", { awaiting_answer: true, autopilot: { support: "supported", condition: "running" } }),
    ]);
    const card = screen.getByRole("button", { name: "Open mako" });
    expect(within(card).getByRole("img", { name: "Asker: Pending answer, on autopilot" })).toBeTruthy();
    expect(within(card).getByText("Pending answer")).toBeTruthy();
  });

  it("does not mark the dots of a project that is not running", () => {
    renderHome([
      omp(2, "Nightly", { autopilot: { support: "supported", condition: "waiting", detail: "next_iteration" } }),
    ], { isActive: false, status: "offline" });
    const card = screen.getByRole("button", { name: "Open mako" });
    expect(within(card).getByRole("img", { name: "Nightly: Not running" }).hasAttribute("data-autopilot")).toBe(false);
  });
});

describe("mobile Idle sessions and autopilot", () => {
  it("lists a tab waiting for its next iteration, or holding, under On autopilot", () => {
    renderHome([
      omp(1, "Plain"),
      omp(2, "Nightly", {
        autopilot: {
          support: "supported",
          condition: "waiting",
          detail: "next_iteration",
          next_iteration_at: "2026-11-13T14:30:00Z",
          iterations: 2,
        },
      }),
      omp(3, "Held", { autopilot: { support: "supported", condition: "holding", detail: "server" } }),
    ]);
    openIdleList();

    const section = screen.getByRole("heading", { name: "On autopilot" }).closest("section") as HTMLElement;
    expect(section.contains(row("Nightly"))).toBe(true);
    expect(section.contains(row("Held"))).toBe(true);
    // Not among the tabs that wait for their user, which come first.
    expect(section.contains(row("Plain"))).toBe(false);
    expect(row("Plain").compareDocumentPosition(row("Nightly")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    expect(row("Nightly").textContent).toContain("On autopilot");
    expect(row("Nightly").textContent).toMatch(/Waiting for the next iteration at /);
    expect(row("Nightly").textContent).not.toContain("Idle");
    expect(row("Held").textContent).toContain("Holding for the server");
    expect(row("Plain").textContent).toContain("Idle");
    expect(row("Plain").textContent).not.toMatch(/autopilot/i);
  });

  it("keeps a paused and an ended autopilot among the idle tabs, each with its reason in full", () => {
    renderHome([
      omp(1, "Failing", { autopilot: { support: "supported", condition: "paused", detail: "failures" } }),
      omp(2, "Finished", { autopilot: { support: "supported", condition: "ended", detail: "iteration_limit" } }),
      omp(3, "Cut off", { autopilot: { support: "supported", condition: "paused", detail: "no_control" } }),
      omp(4, "Stale", { autopilot: { support: "needs_agent_reboot" } }),
    ]);
    openIdleList();

    expect(screen.queryByRole("heading", { name: "On autopilot" })).toBeNull();
    expect(row("Failing").textContent).toContain("Idle");
    expect(row("Failing").textContent).toContain("Paused after three failed iterations");
    expect(row("Finished").textContent).toContain("Idle");
    expect(row("Finished").textContent).toContain("Ended: iteration limit reached");
    // The reason is on a line that wraps, not on the truncated host line.
    const reason = within(row("Cut off")).getByText("Paused: APAS could not control this tab for more than four hours");
    expect(reason.className.split(" ")).toContain("break-words");
    expect(reason.className.split(" ")).not.toContain("truncate");
    // Availability alone is not something a list shows.
    expect(row("Stale").textContent).not.toMatch(/autopilot/i);
  });

  it("keeps Pending answer first for a tab on autopilot that asked a question", () => {
    renderHome([
      omp(1, "Asker", { awaiting_answer: true, autopilot: { support: "supported", condition: "running" } }),
    ]);
    openIdleList();
    const pending = screen.getByRole("heading", { name: "Pending answer" }).closest("section") as HTMLElement;
    expect(pending.contains(row("Asker"))).toBe(true);
    expect(screen.queryByRole("heading", { name: "On autopilot" })).toBeNull();
    expect(row("Asker").textContent).toContain("Pending answer");
    expect(row("Asker").textContent).not.toContain("Running an iteration");
  });

  it("shows the group by itself when every idle tab is on autopilot", () => {
    renderHome([
      omp(2, "Nightly", { autopilot: { support: "supported", condition: "waiting", detail: "background_work" } }),
    ]);
    openIdleList();
    expect(screen.getByRole("heading", { name: "On autopilot" })).toBeTruthy();
    expect(screen.queryByText("No idle sessions")).toBeNull();
    expect(row("Nightly").textContent).toContain("Waiting for background work");
  });

  it("does not list a tab on autopilot that is working", () => {
    renderHome([
      omp(2, "Busy", { is_working: true, autopilot: { support: "supported", condition: "running" } }),
    ]);
    openIdleList();
    expect(screen.queryByRole("heading", { name: "On autopilot" })).toBeNull();
    expect(screen.getByText("No idle sessions")).toBeTruthy();
  });

  it("lists a tab on autopilot there even when its provider is usage limited", () => {
    const limited = { window: "weekly", resets_at: "2999-01-01T00:00:00Z" };
    renderHome([
      omp(1, "Blocked", { usage_limited: limited }),
      omp(2, "Nightly", {
        autopilot: { support: "supported", condition: "waiting", detail: "next_iteration" },
        usage_limited: limited,
      }),
    ]);
    openIdleList();
    const onAutopilot = screen.getByRole("heading", { name: "On autopilot" }).closest("section") as HTMLElement;
    const usageLimited = screen.getByRole("heading", { name: "Usage limited" }).closest("section") as HTMLElement;
    expect(onAutopilot.contains(row("Nightly"))).toBe(true);
    expect(usageLimited.contains(row("Nightly"))).toBe(false);
    expect(usageLimited.contains(row("Blocked"))).toBe(true);
    expect(row("Nightly").textContent).toContain("On autopilot");
    expect(row("Blocked").textContent).toContain("Weekly usage limited");
  });

  it("opens the session on the tab the row names", () => {
    const props = renderHome([
      omp(2, "Nightly", { autopilot: { support: "supported", condition: "holding", detail: "controller" } }),
    ]);
    openIdleList();
    fireEvent.click(row("Nightly"));
    expect(props.onOpenSession).toHaveBeenCalledWith("session-a", "mako");
  });
});
