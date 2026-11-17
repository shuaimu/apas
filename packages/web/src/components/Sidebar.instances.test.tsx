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

describe("a project's open state in the project list", () => {
  const twoMachines = [
    { pane_id: 1, label: "Claude 1", kind: "terminal", provider: "claude", is_working: true, machine_id: ZOO5, hostname: "zoo-005" },
    { pane_id: 3, label: "Claude 3", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO6, hostname: "zoo-006" },
  ];

  function stateOf(): string | null | undefined {
    return document.querySelector('[data-project-id="project-a"] [data-testid="project-open-state"]')?.textContent;
  }

  it("reads Open, and marks the tabs of a machine that is not following it", () => {
    // Recorded open; zoo-005 runs it and zoo-006, which owns a tab, does not.
    seed([project({
      openState: "open",
      instances: [{ machineId: ZOO5, hostname: "zoo-005", primary: true }],
      panes: twoMachines,
    })]);
    render(<Sidebar />);
    expect(stateOf()).toBe("Open");
    expect(screen.getByRole("img", { name: "Claude 1: Working" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "Claude 3: Not running on zoo-006" })).toBeTruthy();
  });

  it("reads Closed with every tab not running once a closed project runs nowhere", () => {
    seed([project({
      openState: "closed",
      isActive: false,
      status: "offline",
      instances: [],
      createdAt: "2026-11-10T10:00:00Z",
      panes: twoMachines,
    })]);
    render(<Sidebar />);
    expect(stateOf()).toBe(`Closed · ${new Date("2026-11-10T10:00:00Z").toLocaleDateString()}`);
    expect(screen.getByRole("img", { name: "Claude 1: Not running" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "Claude 3: Not running" })).toBeTruthy();

    // A closed project's agents are not listed as idle.
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    expect(screen.queryByRole("button", { name: "Open Claude 3 in chiral-network" })).toBeNull();
  });

  it("keeps the real status of tabs on a machine that still runs a closed project", () => {
    // The close could not reach zoo-006, which still runs the project.
    seed([project({
      openState: "closed",
      instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true }],
      panes: twoMachines,
    })]);
    render(<Sidebar />);
    expect(stateOf()).toBe("Closed");
    expect(screen.getByRole("img", { name: "Claude 3: Idle" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "Claude 1: Not running on zoo-005" })).toBeTruthy();
  });

  it("reads Open for a recorded-open project that no machine runs, with its tabs not running", () => {
    seed([project({
      openState: "open",
      isActive: false,
      status: "offline",
      instances: [],
      panes: twoMachines,
    })]);
    render(<Sidebar />);
    expect(stateOf()).toBe("Open");
    expect(screen.getByRole("img", { name: "Claude 1: Not running" })).toBeTruthy();
  });

  it("shows a project the server has not recorded as Open while it runs and Closed otherwise", () => {
    seed([project({ panes: twoMachines })]);
    const { unmount } = render(<Sidebar />);
    expect(stateOf()).toBe("Open");
    unmount();

    seed([project({ isActive: false, status: "offline", instances: [], panes: twoMachines })]);
    render(<Sidebar />);
    expect(stateOf()).toBe("Closed");
  });

  it("takes a registration-only project's state from its machine entry", () => {
    // Recorded open while its machine does not run it (it is away, or its
    // start failed): the row says Open, not Stopped.
    seed([]);
    act(() => useStore.setState({
      machines: [{
        machine: { machineId: ZOO5, hostname: "zoo-005", os: "linux", arch: "x86_64" },
        projects: [{ projectId: "project-a", name: "chiral-network", path: "/work/chiral-network", isRunning: false, openState: "open" }],
        clusterAccess: "owner",
        daemonConnected: true,
      }],
    }));
    render(<Sidebar />);
    expect(stateOf()).toBe("Open");
    expect(screen.queryByText("Stopped")).toBeNull();
    expect(screen.getByRole("link", { name: "View chiral-network on Machines" })).toBeTruthy();
  });

  it("says the state in the collapsed rail's tooltip and keeps the dot for what runs", () => {
    seed([project({
      openState: "closed",
      instances: [{ machineId: ZOO6, hostname: "zoo-006", primary: true }],
    })]);
    const { unmount } = render(<SidebarRail onExpand={vi.fn()} />);
    const stillRunning = screen.getByRole("button", { name: "Open chiral-network" });
    expect(stillRunning.getAttribute("title")?.split("\n")).toContain("Closed");
    expect(stillRunning.getAttribute("title")).not.toContain("Active");
    // zoo-006 still runs it, so the dot stays.
    expect(stillRunning.querySelector('[data-testid="rail-active-dot"]')).toBeTruthy();
    unmount();

    seed([project({ openState: "open", isActive: false, status: "offline", instances: [] })]);
    render(<SidebarRail onExpand={vi.fn()} />);
    const openNowhere = screen.getByRole("button", { name: "Open chiral-network" });
    expect(openNowhere.getAttribute("title")?.split("\n")).toContain("Open");
    expect(openNowhere.querySelector('[data-testid="rail-active-dot"]')).toBeNull();
  });
});
