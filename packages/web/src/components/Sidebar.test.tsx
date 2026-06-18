import { act, fireEvent, render, screen } from "@testing-library/react";
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
}: {
  sessions: SessionInfo[];
  cliClients?: CliClient[];
  machines?: MachineWithProjects[];
  attachSession?: StoreState["attachSession"];
}) {
  act(() => {
    useStore.setState({
      attachSession,
      cliClients,
      connected: true,
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
});
