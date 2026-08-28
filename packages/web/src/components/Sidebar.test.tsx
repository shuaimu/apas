import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  useStore,
  type CliClient,
  type MachineWithProjects,
  type SessionInfo,
  type UsageLimitsByProvider,
} from "@/lib/store";
import { Sidebar } from "./Sidebar";

type StoreState = ReturnType<typeof useStore.getState>;
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

function makeCliClient(overrides: Partial<CliClient> & Pick<CliClient, "id">): CliClient {
  return {
    status: "online",
    ...overrides,
  };
}

function makeMachine(projects: MachineWithProjects["projects"]): MachineWithProjects {
  return {
    machine: {
      machineId: "machine-1",
      hostname: "daemon-host",
      os: "linux",
      arch: "x64",
    },
    projects,
  };
}

function seedSidebarState({
  sessions,
  cliClients = [],
  machines = [],
  attachSession = vi.fn(),
  forgetProject = vi.fn(),
  openSessionPane = vi.fn(),
  usageLimits = new Map(),
}: {
  sessions: SessionInfo[];
  cliClients?: CliClient[];
  machines?: MachineWithProjects[];
  attachSession?: StoreState["attachSession"];
  forgetProject?: StoreState["forgetProject"];
  openSessionPane?: StoreState["openSessionPane"];
  usageLimits?: Map<string, UsageLimitsByProvider>;
}) {
  act(() => {
    useStore.setState({
      attachSession,
      cliClients,
      connected: true,
      forgetProject,
      listSessions: vi.fn(),
      machines,
      openSessionPane,
      refreshCliClients: vi.fn(),
      sessionId: null,
      sessions,
      token: "test-token",
      unreadSessions: new Set(),
      usageLimits,
      userId: null,
    });
  });

  return attachSession;
}

function projectRow(workingDir: string): HTMLElement {
  const label = screen.getByText(workingDir);
  const row = label.closest(".cursor-pointer");
  if (!row) {
    throw new Error(`No project row found for ${workingDir}`);
  }
  return row as HTMLElement;
}

describe("Sidebar project list", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.unstubAllGlobals();
    // Collapse state persists to localStorage; clear it so a group collapsed in
    // one test doesn't start collapsed in the next.
    window.localStorage.clear();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("links to the account's own cluster and never to system administration", () => {
    seedSidebarState({ sessions: [] });
    render(<Sidebar />);

    const cluster = screen.getByRole("link", { name: /My Cluster/ });
    expect(cluster.getAttribute("href")).toBe("/machines");
    // System administration has its own URL and its own credential. Nothing in
    // the ordinary interface advertises it, for any account.
    expect(screen.queryByRole("link", { name: /Administration/i })).toBeNull();
    expect(
      screen.queryAllByRole("link").some((link) => link.getAttribute("href") === "/admin"),
    ).toBe(false);
  });

  it("offers GitHub project creation even when there is no existing repository group", async () => {
    seedSidebarState({ sessions: [], machines: [makeMachine([])] });
    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: "Create project from GitHub" }));
    expect(await screen.findByRole("heading", { name: "New project" })).toBeTruthy();
    expect(screen.getByLabelText("Clone URL")).toBeTruthy();
  });

  it("deduplicates by project id and sorts active CLI and daemon projects before inactive history", () => {
    seedSidebarState({
      cliClients: [
        makeCliClient({
          id: "cli-active",
          activeSession: "session-cli-current",
        }),
      ],
      machines: [
        makeMachine([
          {
            projectId: "project-daemon",
            path: "/repo/daemon-only",
            isRunning: true,
          },
        ]),
      ],
      sessions: [
        makeSession({
          id: "session-cli-current",
          projectId: "project-cli",
          workingDir: "/repo/cli-active",
          createdAt: "2026-06-17T10:00:00Z",
        }),
        makeSession({
          id: "session-history",
          projectId: "project-history",
          workingDir: "/repo/history",
          createdAt: "2026-06-17T11:00:00Z",
        }),
        makeSession({
          id: "session-duplicate-new",
          projectId: "project-duplicate",
          workingDir: "/repo/duplicate-new",
          createdAt: "2026-06-16T10:00:00Z",
        }),
        makeSession({
          id: "session-duplicate-old",
          projectId: "project-duplicate",
          workingDir: "/repo/duplicate-old",
          createdAt: "2026-06-15T10:00:00Z",
        }),
        makeSession({
          id: "session-daemon",
          projectId: "project-daemon",
          workingDir: "/repo/daemon-only",
          createdAt: "2026-06-14T10:00:00Z",
        }),
      ],
    });

    const { container } = render(<Sidebar />);

    expect(screen.getByText("/repo/duplicate-new")).toBeTruthy();
    expect(screen.queryByText("/repo/duplicate-old")).toBeNull();
    expect(projectRow("/repo/cli-active").textContent).toContain("Active");
    expect(projectRow("/repo/daemon-only").textContent).toContain("Active");

    const rowTexts = Array.from(container.querySelectorAll(".cursor-pointer")).map(
      (row) => row.textContent || "",
    );
    const cliActiveIndex = rowTexts.findIndex((text) => text.includes("/repo/cli-active"));
    const daemonActiveIndex = rowTexts.findIndex((text) => text.includes("/repo/daemon-only"));
    const inactiveHistoryIndex = rowTexts.findIndex((text) => text.includes("/repo/history"));
    const duplicateIndex = rowTexts.findIndex((text) => text.includes("/repo/duplicate-new"));

    expect(cliActiveIndex).toBeGreaterThanOrEqual(0);
    expect(daemonActiveIndex).toBeGreaterThanOrEqual(0);
    expect(inactiveHistoryIndex).toBeGreaterThanOrEqual(0);
    expect(duplicateIndex).toBeGreaterThanOrEqual(0);
    expect(cliActiveIndex).toBeLessThan(inactiveHistoryIndex);
    expect(daemonActiveIndex).toBeLessThan(inactiveHistoryIndex);
    expect(inactiveHistoryIndex).toBeLessThan(duplicateIndex);
  });

  it("attaches the selected project row and closes the sidebar", () => {
    const attachSession = vi.fn();
    const onClose = vi.fn();

    seedSidebarState({
      attachSession: attachSession as StoreState["attachSession"],
      sessions: [
        makeSession({
          id: "session-click-target",
          projectId: "project-click-target",
          workingDir: "/repo/click-target",
          createdAt: "2026-06-17T12:00:00Z",
        }),
      ],
    });

    render(<Sidebar onClose={onClose} />);

    fireEvent.click(projectRow("/repo/click-target"));

    expect(attachSession).toHaveBeenCalledWith("session-click-target");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("renders a repo header per group, including the no-remote bucket", () => {
    seedSidebarState({
      sessions: [
        makeSession({
          id: "s-apas",
          projectId: "p-apas",
          workingDir: "/repo/apas",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-17T10:00:00Z",
        }),
        makeSession({
          id: "s-loose",
          projectId: "p-loose",
          workingDir: "/repo/loose",
          createdAt: "2026-06-16T10:00:00Z",
        }),
      ],
    });

    render(<Sidebar />);

    // GitHub remote is shown as owner/repo; remote-less projects group under
    // the literal "(no remote)" header.
    expect(screen.getByText("shuaimu/apas")).toBeTruthy();
    expect(screen.getByText("(no remote)")).toBeTruthy();
    expect(projectRow("/repo/apas").textContent).toContain("/repo/apas");
    expect(projectRow("/repo/loose").textContent).toContain("/repo/loose");
  });

  it("groups two projects that share the same git remote under one header", () => {
    seedSidebarState({
      sessions: [
        makeSession({
          id: "s-a",
          projectId: "p-a",
          workingDir: "/home/a/apas",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-17T10:00:00Z",
        }),
        makeSession({
          id: "s-b",
          projectId: "p-b",
          workingDir: "/home/b/apas-fork",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-16T10:00:00Z",
        }),
      ],
    });

    render(<Sidebar />);

    // A single shared header covers both project rows.
    expect(screen.getAllByText("shuaimu/apas")).toHaveLength(1);
    expect(screen.getByText("/home/a/apas")).toBeTruthy();
    expect(screen.getByText("/home/b/apas-fork")).toBeTruthy();
  });

  it("toggling a repo header hides and reshows its project rows", () => {
    seedSidebarState({
      sessions: [
        makeSession({
          id: "s-apas",
          projectId: "p-apas",
          workingDir: "/repo/apas",
          gitRemote: "github.com/shuaimu/apas",
          createdAt: "2026-06-17T10:00:00Z",
        }),
      ],
    });

    render(<Sidebar />);

    const header = screen.getByText("shuaimu/apas");
    expect(screen.getByText("/repo/apas")).toBeTruthy();

    fireEvent.click(header);
    expect(screen.queryByText("/repo/apas")).toBeNull();

    fireEvent.click(header);
    expect(screen.getByText("/repo/apas")).toBeTruthy();
  });

  it("shows leave-only actions to an ordinary project user", async () => {
    seedSidebarState({
      sessions: [makeSession({
        id: "representative-session",
        projectId: "canonical-project",
        workingDir: "/repo/shared",
        isShared: true,
        shareRole: "user",
      })],
    });
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    render(<Sidebar />);
    fireEvent.click(screen.getByTitle("Project actions"));

    expect(await screen.findByRole("button", { name: "Leave project" })).toBeTruthy();
    expect(screen.queryByText("Invite")).toBeNull();
    expect(screen.queryByTitle("Transfer ownership")).toBeNull();
    expect(screen.queryByRole("button", { name: "Permanently delete project" })).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("uses the canonical project id for transfer and exact deletion confirmation", async () => {
    seedSidebarState({
      sessions: [makeSession({
        id: "representative-session",
        projectId: "canonical-project",
        workingDir: "/repo/owned",
        isShared: false,
        shareRole: "owner",
      })],
    });
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ code: "CODE", share_url: "http://share" }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          owner: { user_id: "owner", user_email: "owner@test" },
          shares: [{ user_id: "member", user_email: "member@test", role: "user" }],
          viewer_role: "owner",
          can_manage: true,
        }),
      })
      .mockResolvedValueOnce({ ok: true, json: async () => ({ success: true }) });
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<Sidebar />);
    fireEvent.click(screen.getByTitle("Manage project access"));
    fireEvent.click(await screen.findByText("Manage Access"));
    fireEvent.click(await screen.findByTitle("Transfer ownership"));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    expect(fetchMock.mock.calls[2][0]).toBe(
      "https://apas.mpaxos.com/projects/canonical-project/owner",
    );
    expect(fetchMock.mock.calls[2][1]).toMatchObject({
      method: "PATCH",
      body: JSON.stringify({ user_id: "member" }),
    });

    // Reopen to inspect the owner danger section after transfer closed it.
    fetchMock
      .mockResolvedValueOnce({ ok: true, json: async () => ({ code: "CODE2", share_url: "http://share2" }) })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ owner: null, shares: [], viewer_role: "owner", can_manage: true }),
      });
    fireEvent.click(screen.getByTitle("Manage project access"));
    fireEvent.click(await screen.findByText("Manage Access"));
    const deleteButton = await screen.findByRole("button", { name: "Permanently delete project" }) as HTMLButtonElement;
    expect(deleteButton.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Project deletion confirmation"), {
      target: { value: "wrong" },
    });
    expect(deleteButton.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Project deletion confirmation"), {
      target: { value: "canonical-project" },
    });
    expect(deleteButton.disabled).toBe(false);
    expect(screen.queryByRole("button", { name: "Leave project" })).toBeNull();
  });

  it("keeps project state and shows the server error when leave fails", async () => {
    const forgetProject = vi.fn();
    seedSidebarState({
      forgetProject,
      sessions: [makeSession({
        id: "shared-session",
        projectId: "shared-project",
        workingDir: "/repo/shared-error",
        isShared: true,
        shareRole: "user",
      })],
    });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: false,
      status: 409,
      json: async () => ({ error: "Membership changed concurrently" }),
    }));
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<Sidebar />);
    fireEvent.click(screen.getByTitle("Project actions"));
    fireEvent.click(await screen.findByRole("button", { name: "Leave project" }));

    expect(await screen.findByText("Membership changed concurrently")).toBeTruthy();
    expect(forgetProject).not.toHaveBeenCalled();
  });

  it("lists idle agents one per pane, including inside a working project", () => {
    // The case the project list cannot express: this project is working, so it
    // reads as busy while two of its agents sit waiting.
    seedSidebarState({
      sessions: [makeSession({
        id: "session-a",
        workingDir: "/repo/mako",
        hostname: "zoo-005",
        isActive: true,
        isWorking: true,
        panes: [
          { pane_id: 3, label: "Claude terminal 3", kind: "terminal", provider: "claude", is_working: false },
          { pane_id: 4, label: "Codex terminal 2", kind: "terminal", provider: "codex", is_working: true },
        ],
      })],
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));

    const row = screen.getByRole("button", { name: "Open Claude terminal 3 in mako" });
    const text = row.textContent ?? "";
    expect(text).toContain("zoo-005");
    // Project before agent, both on the prominent line.
    expect(text.indexOf("mako")).toBeGreaterThanOrEqual(0);
    expect(text.indexOf("mako")).toBeLessThan(text.indexOf("Claude terminal 3"));
    expect(screen.queryByRole("button", { name: /Codex terminal 2/ })).toBeNull();
  });

  it("ranks the most recently idle agent first and keeps legacy panes afterward", () => {
    seedSidebarState({
      sessions: [
        makeSession({
          id: "session-old",
          workingDir: "/repo/older",
          isActive: true,
          panes: [{
            pane_id: 1,
            label: "Older agent",
            kind: "terminal",
            provider: "claude",
            is_working: false,
            idle_since: "2026-08-20T12:00:00Z",
          }],
        }),
        makeSession({
          id: "session-legacy",
          workingDir: "/repo/legacy",
          isActive: true,
          panes: [{
            pane_id: 2,
            label: "Legacy agent",
            kind: "terminal",
            provider: "claude",
            is_working: false,
          }],
        }),
        makeSession({
          id: "session-new",
          workingDir: "/repo/newer",
          isActive: true,
          panes: [{
            pane_id: 3,
            label: "Newest agent",
            kind: "terminal",
            provider: "codex",
            is_working: false,
            idle_since: "2026-08-20T13:00:00Z",
          }],
        }),
      ],
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));

    const rows = screen.getAllByRole("button")
      .map((button) => button.getAttribute("aria-label"))
      .filter((label): label is string => Boolean(label?.startsWith("Open ")));
    expect(rows).toEqual([
      "Open Newest agent in newer",
      "Open Older agent in older",
      "Open Legacy agent in legacy",
    ]);
  });

  it("puts provider-blocked agents below idle agents in the merged view", () => {
    seedSidebarState({
      sessions: [makeSession({
        id: "session-a",
        cliClientId: "cli-a",
        workingDir: "/repo/mako",
        hostname: "zoo-005",
        isActive: true,
        panes: [
          { pane_id: 197, label: "Claude 4", kind: "terminal", provider: "claude", is_working: false },
          { pane_id: 198, label: "Codex 4", kind: "terminal", provider: "codex", is_working: false },
        ],
      })],
      usageLimits: new Map([
        ["cli-a", {
          claude: {
            sevenDay: { utilization: 1 },
            usageLimited: { window: "weekly", resetsAt: "2099-08-23T13:00:00Z" },
          },
        }],
      ]),
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    expect(screen.queryByRole("button", { name: "Usage limited" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Usage limited" })).toBeTruthy();
    const idleRow = screen.getByRole("button", { name: "Open Codex 4 in mako" });
    const limitedRow = screen.getByRole("button", { name: "Open Claude 4 in mako" });
    expect(limitedRow.textContent).toContain("Weekly usage limited");
    expect(limitedRow.textContent).toContain("Resets in");
    expect(idleRow.compareDocumentPosition(limitedRow) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("shows pending answers ahead of idle and usage-limited agents", () => {
    seedSidebarState({
      sessions: [makeSession({
        id: "session-a",
        cliClientId: "cli-a",
        workingDir: "/repo/mako-cloud",
        hostname: "zoo-005",
        isActive: true,
        isWorking: false,
        panes: [
          { pane_id: 305, label: "Claude 3", kind: "terminal", provider: "claude", is_working: false, awaiting_answer: true },
          { pane_id: 306, label: "Idle helper", kind: "terminal", provider: "codex", is_working: false },
          { pane_id: 307, label: "Limited helper", kind: "terminal", provider: "claude", is_working: false },
        ],
      })],
      usageLimits: new Map([["cli-a", {
        claude: {
          sevenDay: { utilization: 1 },
          usageLimited: { window: "weekly", resetsAt: "2099-08-23T13:00:00Z" },
        },
      }]]),
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));

    const pending = screen.getByRole("button", { name: "Open Claude 3 in mako-cloud" });
    const idle = screen.getByRole("button", { name: "Open Idle helper in mako-cloud" });
    const limited = screen.getByRole("button", { name: "Open Limited helper in mako-cloud" });
    expect(pending.textContent).toContain("Pending answer");
    expect(pending.compareDocumentPosition(idle) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(idle.compareDocumentPosition(limited) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("opens the agent that was named, not just its project", () => {
    const openSessionPane = vi.fn();
    seedSidebarState({
      openSessionPane,
      sessions: [makeSession({
        id: "session-a",
        workingDir: "/repo/mako",
        isActive: true,
        panes: [{ pane_id: 7, label: "Claude terminal 3", kind: "terminal", provider: "claude", is_working: false }],
      })],
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Claude terminal 3 in mako" }));

    expect(openSessionPane).toHaveBeenCalledWith("session-a", 7);
  });

  it("excludes agents in projects that are not running", () => {
    seedSidebarState({
      sessions: [makeSession({
        id: "session-a",
        workingDir: "/repo/stopped",
        isActive: false,
        panes: [{ pane_id: 3, label: "Stopped one", kind: "terminal", provider: "claude", is_working: false }],
      })],
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));

    expect(screen.getByText("No idle sessions")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Stopped one/ })).toBeNull();
  });

  it("treats a session with no pane detail as unknown rather than idle", () => {
    // An older server omits the field; filling the list with every project
    // would be worse than showing nothing.
    seedSidebarState({
      sessions: [makeSession({ id: "session-a", workingDir: "/repo/mako", isActive: true })],
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));

    expect(screen.getByText("No idle sessions")).toBeTruthy();
  });

  it("keeps the project list one click away", () => {
    seedSidebarState({
      sessions: [makeSession({ id: "session-a", workingDir: "/repo/mako", isActive: true })],
    });

    render(<Sidebar />);
    expect(screen.getByRole("button", { name: "All projects" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    expect(screen.queryByText("/repo/mako")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "All projects" }));
    expect(screen.getByText("/repo/mako")).toBeTruthy();
  });
});
