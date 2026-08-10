import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  useStore,
  type CliClient,
  type MachineWithProjects,
  type SessionInfo,
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
}: {
  sessions: SessionInfo[];
  cliClients?: CliClient[];
  machines?: MachineWithProjects[];
  attachSession?: StoreState["attachSession"];
  forgetProject?: StoreState["forgetProject"];
}) {
  act(() => {
    useStore.setState({
      attachSession,
      cliClients,
      connected: true,
      forgetProject,
      listSessions: vi.fn(),
      machines,
      refreshCliClients: vi.fn(),
      sessionId: null,
      sessions,
      token: "test-token",
      unreadSessions: new Set(),
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
});
