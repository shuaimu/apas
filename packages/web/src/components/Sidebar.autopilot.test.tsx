import { act, fireEvent, render, screen, within } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore, type SessionInfo, type SessionPaneSummary } from "@/lib/store";
import { Sidebar } from "./Sidebar";

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & { href: string; children: ReactNode };

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: LinkProps) => <a href={href} {...props}>{children}</a>,
}));

const initialStore = useStore.getInitialState();

function omp(pane_id: number, label: string, overrides: Partial<SessionPaneSummary> = {}): SessionPaneSummary {
  return { pane_id, label, kind: "terminal", provider: "omp", is_working: false, ...overrides };
}

function seed(panes: SessionPaneSummary[]) {
  const session: SessionInfo = {
    id: "session-a",
    projectId: "session-a",
    cliClientId: "cli-a",
    workingDir: "/repo/mako",
    hostname: "zoo-006",
    status: "active",
    isActive: true,
    panes,
  };
  act(() => {
    useStore.setState({
      attachSession: vi.fn(),
      cliClients: [],
      connected: true,
      forgetProject: vi.fn(),
      listSessions: vi.fn(),
      machines: [],
      openSessionPane: vi.fn(),
      refreshCliClients: vi.fn(),
      sessionId: null,
      sessions: [session],
      token: "test-token",
      unreadSessions: new Set(),
      usageLimits: new Map(),
      userId: null,
    });
  });
}

function row(label: string): HTMLElement {
  return screen.getByRole("button", { name: `Open ${label} in mako` });
}

function openIdleList() {
  render(<Sidebar />);
  fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
}

afterEach(() => {
  act(() => {
    useStore.setState(initialStore, true);
  });
  localStorage.clear();
  vi.clearAllMocks();
});

describe("Sidebar idle sessions and autopilot", () => {
  it("lists a tab waiting for its next iteration, or holding, under On autopilot", () => {
    seed([
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

    const heading = screen.getByRole("heading", { name: "On autopilot" });
    const section = heading.closest("section") as HTMLElement;
    expect(section.contains(row("Nightly"))).toBe(true);
    expect(section.contains(row("Held"))).toBe(true);
    // Not among the tabs that wait for their user.
    expect(section.contains(row("Plain"))).toBe(false);
    expect(row("Plain").compareDocumentPosition(row("Nightly")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    expect(row("Nightly").textContent).toContain("On autopilot");
    expect(row("Nightly").textContent).toMatch(/Waiting for the next iteration at /);
    expect(row("Nightly").textContent).not.toContain("Idle");
    expect(row("Held").textContent).toContain("Holding for the server");
    expect(row("Plain").textContent).toContain("Idle");
  });

  it("keeps a paused and an ended autopilot among the idle tabs, with their reasons", () => {
    seed([
      omp(1, "Failing", { autopilot: { support: "supported", condition: "paused", detail: "failures" } }),
      omp(2, "Finished", { autopilot: { support: "supported", condition: "ended", detail: "iteration_limit" } }),
      omp(3, "Stale", { autopilot: { support: "needs_agent_reboot" } }),
    ]);
    openIdleList();

    expect(screen.queryByRole("heading", { name: "On autopilot" })).toBeNull();
    expect(row("Failing").textContent).toContain("Idle");
    expect(row("Failing").textContent).toContain("Paused after three failed iterations");
    expect(row("Finished").textContent).toContain("Idle");
    expect(row("Finished").textContent).toContain("Ended: iteration limit reached");
    // Each reason is a line of its own that wraps: sharing the host's one
    // truncated line, the sidebar's width would cut it short.
    for (const [label, reason] of [
      ["Failing", "Paused after three failed iterations"],
      ["Finished", "Ended: iteration limit reached"],
    ]) {
      const line = within(row(label)).getByText(reason);
      expect(line.className).toContain("break-words");
      expect(line.className).not.toContain("truncate");
    }
    expect(within(row("Failing")).getByText("zoo-006").className).toContain("truncate");
    // Availability alone is not something a list shows.
    expect(row("Stale").textContent).not.toMatch(/autopilot/i);
  });

  it("keeps Pending answer first for a tab on autopilot that asked a question", () => {
    seed([
      omp(1, "Asker", {
        awaiting_answer: true,
        autopilot: { support: "supported", condition: "running" },
      }),
    ]);
    openIdleList();
    const pending = screen.getByRole("heading", { name: "Pending answer" }).closest("section") as HTMLElement;
    expect(pending.contains(row("Asker"))).toBe(true);
    expect(screen.queryByRole("heading", { name: "On autopilot" })).toBeNull();
    expect(row("Asker").textContent).toContain("Pending answer");
  });

  it("shows the group by itself when every idle tab is on autopilot", () => {
    seed([omp(2, "Nightly", { autopilot: { support: "supported", condition: "waiting", detail: "background_work" } })]);
    openIdleList();
    expect(screen.getByRole("heading", { name: "On autopilot" })).toBeTruthy();
    expect(screen.queryByText("No idle sessions")).toBeNull();
    expect(row("Nightly").textContent).toContain("Waiting for background work");
  });

  it("does not list a tab on autopilot that is working", () => {
    seed([omp(2, "Busy", { is_working: true, autopilot: { support: "supported", condition: "running" } })]);
    openIdleList();
    expect(screen.queryByRole("button", { name: "Open Busy in mako" })).toBeNull();
    expect(screen.getByText("No idle sessions")).toBeTruthy();
  });
});
