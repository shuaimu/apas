import { act, fireEvent, render, screen } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore, type SessionInfo } from "@/lib/store";
import { Sidebar } from "./Sidebar";
import { SidebarRail } from "./SidebarRail";

/**
 * A project running on several machines keeps one row in the project list,
 * naming its primary host and how many more machines run it.
 */

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & { href: string; children: ReactNode };

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: LinkProps) => <a href={href} {...props}>{children}</a>,
}));

const initialStore = useStore.getInitialState();
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";

function project(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: "project-a",
    projectId: "project-a",
    workingDir: "/work/chiral-network",
    hostname: "zoo-005",
    gitRemote: "github.com/example/chiral-network",
    status: "active",
    isActive: true,
    instances: [
      { machineId: ZOO6, hostname: "zoo-006", primary: false },
      { machineId: ZOO5, hostname: "zoo-005", primary: true },
    ],
    ...overrides,
  };
}

function seed(sessions: SessionInfo[]) {
  act(() => {
    useStore.setState({
      attachSession: vi.fn(),
      cliClients: [],
      connected: true,
      listSessions: vi.fn(),
      machines: [],
      openSessionPane: vi.fn(),
      refreshCliClients: vi.fn(),
      sessionId: null,
      sessions,
      token: "test-token",
      unreadSessions: new Set(),
      usageLimits: new Map(),
      userId: null,
    });
  });
}

afterEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  act(() => useStore.setState(initialStore, true));
});

describe("project rows across machines", () => {
  it("shows the primary host plus the number of other machines in the sidebar row", () => {
    seed([project()]);
    render(<Sidebar />);
    const host = screen.getByTitle("Running on zoo-005, zoo-006");
    expect(host.textContent).toBe("zoo-005 +1zoo-005 and 1 more machine");
    expect(screen.getByText("zoo-005 +1").getAttribute("aria-hidden")).toBe("true");
    expect(screen.getByText("zoo-005 and 1 more machine").className).toContain("sr-only");
  });

  it("keeps the plain host for a project on one machine", () => {
    seed([project({ instances: [{ machineId: ZOO5, hostname: "zoo-005", primary: true }] })]);
    render(<Sidebar />);
    expect(screen.getByTitle("zoo-005").textContent).toBe("zoo-005");
    expect(screen.queryByText(/\+1/)).toBeNull();
  });

  it("shows the primary host plus the others in the collapsed rail", () => {
    seed([project()]);
    render(<SidebarRail onExpand={vi.fn()} />);
    const icon = screen.getByRole("button", { name: "Open chiral-network" });
    expect(icon.getAttribute("title")?.split("\n")).toContain("zoo-005 +1");
  });

  it("names each idle agent's own machine", () => {
    seed([project({
      panes: [
        { pane_id: 3, label: "Claude 3", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO6, hostname: "zoo-006" },
      ],
    })]);
    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    expect(screen.getByRole("button", { name: "Open Claude 3 in chiral-network" }).textContent).toContain("zoo-006");
  });

  it("lists a stopped machine's agents as not running, not as idle", () => {
    // zoo-006 stopped; zoo-005 keeps the project active. The server reports
    // zoo-006's panes as not working, with an idle time.
    seed([project({
      instances: [{ machineId: ZOO5, hostname: "zoo-005", primary: true }],
      panes: [
        { pane_id: 1, label: "Claude 1", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO5, hostname: "zoo-005" },
        { pane_id: 3, label: "Claude 3", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO6, hostname: "zoo-006", idle_since: "2026-11-12T00:00:00Z" },
        // Written before instances and attributed to zoo-006.
        { pane_id: 4, label: "Claude 4", kind: "terminal", provider: "claude", is_working: false, awaiting_answer: true, hostname: "zoo-006" },
      ],
    })]);
    render(<Sidebar />);
    expect(screen.getByRole("img", { name: "Claude 1: Idle" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "Claude 3: Not running on zoo-006" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "Claude 4: Not running on zoo-006" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    expect(screen.getByRole("button", { name: "Open Claude 1 in chiral-network" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open Claude 3 in chiral-network" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open Claude 4 in chiral-network" })).toBeNull();
  });

  it("still lists idle agents when the server reports no instances", () => {
    // An older server: nothing says which machines run the project, so the
    // session's own state is all there is to go on.
    seed([project({
      instances: undefined,
      panes: [
        { pane_id: 3, label: "Claude 3", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO6, hostname: "zoo-006" },
      ],
    })]);
    render(<Sidebar />);
    expect(screen.getByRole("img", { name: "Claude 3: Idle" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    expect(screen.getByRole("button", { name: "Open Claude 3 in chiral-network" })).toBeTruthy();
  });

  it("keeps a sole older CLI's untagged agents idle: that CLI runs them all", () => {
    // zoo-005's daemon is connected but older, so its instance owns every
    // untagged pane, including ones last recorded on another host.
    seed([project({
      instances: [{ machineId: ZOO5, hostname: "zoo-005", primary: true }],
      panes: [
        { pane_id: 4, label: "Claude 4", kind: "terminal", provider: "claude", is_working: false, hostname: "zoo-006" },
      ],
    })]);
    act(() => useStore.setState({
      machines: [{
        machine: { machineId: ZOO5, hostname: "zoo-005", os: "linux", arch: "x86_64" },
        projects: [{ projectId: "project-a", path: "/work/chiral-network", isRunning: true }],
        clusterAccess: "owner",
        daemonConnected: true,
        projectInstancesAvailable: false,
      }],
    }));
    render(<Sidebar />);
    expect(screen.getByRole("img", { name: "Claude 4: Idle" })).toBeTruthy();
  });
});
