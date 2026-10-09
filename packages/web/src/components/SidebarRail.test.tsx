import { act, fireEvent, render, screen } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore, type MachineWithProjects, type SessionInfo } from "@/lib/store";
import { SidebarRail } from "./SidebarRail";

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  children: ReactNode;
};

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: LinkProps) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

const initialStore = useStore.getInitialState();

function makeSession(overrides: Partial<SessionInfo> & Pick<SessionInfo, "id" | "workingDir">): SessionInfo {
  return {
    projectId: overrides.id,
    status: "inactive",
    isActive: false,
    ...overrides,
  };
}

function makeMachine(projects: MachineWithProjects["projects"]): MachineWithProjects {
  return {
    machine: { machineId: "machine-1", hostname: "daemon-host", os: "linux", arch: "x64" },
    projects,
  };
}

function seedRail({
  sessions,
  machines = [],
  sessionId = null,
  unreadSessions = new Set<string>(),
}: {
  sessions: SessionInfo[];
  machines?: MachineWithProjects[];
  sessionId?: string | null;
  unreadSessions?: Set<string>;
}) {
  const attachSession = vi.fn();
  act(() => {
    useStore.setState({
      attachSession,
      cliClients: [],
      connected: true,
      listSessions: vi.fn(),
      machines,
      refreshCliClients: vi.fn(),
      sessionId,
      sessions,
      token: "test-token",
      unreadSessions,
      userId: null,
    });
  });
  return attachSession;
}

describe("SidebarRail", () => {
  afterEach(() => {
    vi.clearAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("shows one icon per project, grouped by repo with the no-remote bucket last", () => {
    seedRail({
      sessions: [
        makeSession({
          id: "s-loose",
          projectId: "p-loose",
          workingDir: "/work/loose-dir",
          createdAt: "2026-06-17T12:00:00Z",
        }),
        makeSession({
          id: "s-apas",
          projectId: "p-apas",
          workingDir: "/work/apas",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-17T11:00:00Z",
        }),
        makeSession({
          id: "s-apas-old",
          projectId: "p-apas",
          workingDir: "/work/apas-old",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-16T11:00:00Z",
        }),
        makeSession({
          id: "s-web",
          projectId: "p-web",
          workingDir: "/work/my-web",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-17T10:00:00Z",
        }),
      ],
    });

    render(<SidebarRail onExpand={vi.fn()} />);

    const icons = screen.getAllByRole("button", { name: /^Open / });
    // Deduplicated by project id, repo group first, no-remote bucket last.
    expect(icons.map((icon) => icon.getAttribute("aria-label"))).toEqual([
      "Open apas",
      "Open my-web",
      "Open loose-dir",
    ]);
    expect(icons.map((icon) => icon.textContent)).toEqual(["AP", "MW", "LD"]);
    // The path is on the tooltip, since the icon itself cannot show it.
    expect(icons[0].getAttribute("title")).toContain("/work/apas");
    // One divider between the two groups.
    expect(screen.getAllByRole("separator")).toHaveLength(1);
  });

  it("attaches the project's session when its icon is clicked", () => {
    const attachSession = seedRail({
      sessions: [makeSession({ id: "s-apas", projectId: "p-apas", workingDir: "/work/apas" })],
    });

    render(<SidebarRail onExpand={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Open apas" }));

    expect(attachSession).toHaveBeenCalledWith("s-apas");
  });

  it("offers one keyboard-focusable Machines link for duplicate owned registrations without attaching or starting", () => {
    const owned = {
      ...makeMachine([{ projectId: "p-q-index", name: "q-index", path: "/work/q-index", isRunning: false, preferredMachineId: "machine-005" }]),
      machine: { machineId: "machine-002", hostname: "zoo-002", os: "linux", arch: "x64" },
      clusterOwnerUserId: "owner-1",
      clusterAccess: "owner" as const,
    };
    const attachSession = seedRail({
      sessions: [],
      machines: [
        owned,
        { ...owned, machine: { ...owned.machine, machineId: "machine-005", hostname: "zoo-005" } },
        { ...makeMachine([{ projectId: "p-member", path: "/work/member-only", isRunning: false }]), clusterAccess: "member" },
      ],
      unreadSessions: new Set(["p-q-index"]),
    });
    const start = vi.fn();
    act(() => useStore.setState({ startMachineProjectCli: start }));
    render(<SidebarRail onExpand={vi.fn()} />);

    const link = screen.getByRole("link", { name: "View q-index on Machines — Stopped" });
    const destination = new URL(link.getAttribute("href")!, "https://apas.test");
    expect(destination.pathname).toBe("/machines");
    expect(destination.searchParams.get("cluster_owner")).toBe("owner-1");
    expect(destination.searchParams.get("machine")).toBe("machine-005");
    expect(destination.searchParams.get("project")).toBe("p-q-index");
    expect(screen.queryByRole("link", { name: /member-only/ })).toBeNull();
    expect(link.getAttribute("aria-current")).toBeNull();
    expect(link.querySelector('[data-testid="rail-unread-dot"]')).toBeNull();
    expect(link.querySelector('[data-testid="rail-active-dot"]')).toBeNull();
    link.focus();
    expect(document.activeElement).toBe(link);
    link.addEventListener("click", (event) => event.preventDefault(), { once: true });
    fireEvent.click(link);
    expect(attachSession).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
  });

  it("keeps identity and switches to the arriving real session with normal current selection", () => {
    const attachSession = seedRail({
      sessions: [],
      machines: [makeMachine([{ projectId: "p-q-index", path: "/work/q-index", isRunning: true }])],
    });
    render(<SidebarRail onExpand={vi.fn()} />);
    const registered = screen.getByRole("link", { name: "View q-index on Machines — Running" });
    const colour = registered.style.backgroundColor;
    expect(registered.querySelector('[data-testid="rail-active-dot"]')).not.toBeNull();
    act(() => useStore.setState({
      sessions: [makeSession({ id: "s-q-index", projectId: "p-q-index", workingDir: "/work/q-index" })],
      sessionId: "s-q-index",
    }));
    expect(screen.queryByRole("link", { name: /View q-index on Machines/ })).toBeNull();
    const current = screen.getByRole("button", { name: "Open q-index" });
    expect(current.style.backgroundColor).toBe(colour);
    expect(current.getAttribute("aria-current")).toBe("page");
    fireEvent.click(current);
    expect(attachSession).toHaveBeenCalledWith("s-q-index");
  });

  it("marks the current project, running projects, and unread activity", () => {
    seedRail({
      sessions: [
        makeSession({ id: "s-current", projectId: "p-current", workingDir: "/work/current" }),
        makeSession({ id: "s-running", projectId: "p-running", workingDir: "/work/running" }),
        makeSession({ id: "s-quiet", projectId: "p-quiet", workingDir: "/work/quiet" }),
      ],
      machines: [makeMachine([{ projectId: "p-running", path: "/work/running", isRunning: true }])],
      sessionId: "s-current",
      // Unread on the current project must not show: you are looking at it.
      unreadSessions: new Set(["s-current", "s-running"]),
    });

    render(<SidebarRail onExpand={vi.fn()} />);

    const current = screen.getByRole("button", { name: "Open current" });
    const running = screen.getByRole("button", { name: "Open running" });
    const quiet = screen.getByRole("button", { name: "Open quiet" });

    expect(current.getAttribute("aria-current")).toBe("page");
    expect(running.getAttribute("aria-current")).toBeNull();

    expect(running.querySelector('[data-testid="rail-active-dot"]')).not.toBeNull();
    expect(current.querySelector('[data-testid="rail-active-dot"]')).toBeNull();
    expect(quiet.querySelector('[data-testid="rail-active-dot"]')).toBeNull();

    expect(running.querySelector('[data-testid="rail-unread-dot"]')).not.toBeNull();
    expect(current.querySelector('[data-testid="rail-unread-dot"]')).toBeNull();
    expect(quiet.querySelector('[data-testid="rail-unread-dot"]')).toBeNull();
  });

  it("keeps the same colour for a project regardless of which session represents it", () => {
    seedRail({
      sessions: [makeSession({ id: "s-1", projectId: "p-stable", workingDir: "/work/stable" })],
    });
    const first = render(<SidebarRail onExpand={vi.fn()} />);
    const colourA = screen.getByRole("button", { name: "Open stable" }).style.backgroundColor;
    first.unmount();

    seedRail({
      sessions: [makeSession({ id: "s-2", projectId: "p-stable", workingDir: "/work/stable" })],
    });
    render(<SidebarRail onExpand={vi.fn()} />);
    const colourB = screen.getByRole("button", { name: "Open stable" }).style.backgroundColor;

    expect(colourA).not.toBe("");
    expect(colourB).toBe(colourA);
  });

  it("expands the sidebar and links to the cluster page", () => {
    const onExpand = vi.fn();
    seedRail({ sessions: [], machines: [makeMachine([])] });

    render(<SidebarRail onExpand={onExpand} />);

    fireEvent.click(screen.getByRole("button", { name: "Expand sidebar" }));
    expect(onExpand).toHaveBeenCalledTimes(1);

    expect(screen.getByRole("link", { name: "My Cluster" }).getAttribute("href")).toBe("/machines");

  });
});
