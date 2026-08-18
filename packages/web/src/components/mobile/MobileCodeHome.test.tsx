import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "@/lib/store";
import { readSelectedPane } from "@/lib/mobileSelectedPane";
import { MobileCodeHome, type MobileCodeHomeProps } from "./MobileCodeHome";

function session(overrides: Partial<SessionInfo> & Pick<SessionInfo, "id">): SessionInfo {
  return {
    status: "active",
    isActive: true,
    ...overrides,
  };
}

/// The home fetches one bootstrap document; machines ride along in it.
function stubBootstrap(body: Record<string, unknown>) {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
    ok: true,
    json: async () => body,
  }));
}

function renderHomeWith(overrides: Partial<MobileCodeHomeProps> = {}) {
  const props: MobileCodeHomeProps = {
    active: false,
    connected: true,
    legacySessions: [],
    token: "token",
    onAccount: vi.fn(),
    onManageMachines: vi.fn(),
    onOpenSession: vi.fn(),
    onRebootDaemon: vi.fn(),
    onRefreshMachines: vi.fn(),
    ...overrides,
  };
  const view = render(<MobileCodeHome {...props} />);
  return {
    props,
    rerender: (next: MobileCodeHomeProps) => view.rerender(<MobileCodeHome {...next} />),
  };
}

function renderHome(overrides: Partial<MobileCodeHomeProps> = {}) {
  const props: MobileCodeHomeProps = {
    active: false,
    connected: true,
    legacySessions: [
      session({ id: "session-a", workingDir: "/workspace/alpha", hostname: "builder-a" }),
      session({ id: "session-b", workingDir: "/workspace/beta", hostname: "builder-b", status: "completed", isActive: false }),
    ],
    token: "token",
    onAccount: vi.fn(),
    onManageMachines: vi.fn(),
    onOpenSession: vi.fn(),
    onRebootDaemon: vi.fn(),
    ...overrides,
  };
  render(<MobileCodeHome {...props} />);
  return props;
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("MobileCodeHome", () => {
  it("renders only all-project and idle-session categories", () => {
    renderHome();

    expect(screen.getByRole("heading", { name: "Coding sessions" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Account" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /New task/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: "All projects" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Idle sessions" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Active" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Attention" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Completed" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Recent" })).toBeNull();
    expect(screen.getByRole("button", { name: "Open alpha" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open beta" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    // The idle view lists agents, not projects. These legacy sessions carry no
    // pane detail, so it reports nothing rather than listing them wholesale.
    expect(screen.getByText("No idle sessions")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open alpha" })).toBeNull();
  });

  it("uses only working, idle, and offline session badges", () => {
    renderHome({
      legacySessions: [
        session({ id: "working", workingDir: "/workspace/working", isWorking: true }),
        session({ id: "idle", workingDir: "/workspace/idle", isWorking: false }),
        session({ id: "offline", workingDir: "/workspace/offline", isActive: false, isWorking: false }),
      ],
    });

    expect(screen.getByText("Working")).toBeTruthy();
    expect(screen.getByText("Idle")).toBeTruthy();
    expect(screen.getByText("Offline")).toBeTruthy();
    for (const name of ["working", "idle", "offline"]) {
      expect(within(screen.getByRole("button", { name: `Open ${name}` })).queryByText("Active")).toBeNull();
    }
  });

  it("excludes agents in projects that are not running", async () => {
    // Replaces the old project-level rule. A stopped project's panes report
    // "not working" because nothing is running at all, which is not the same as
    // an agent waiting for you.
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        sessions: [
          {
            id: "session-a",
            project_name: "alpha",
            status: "active",
            is_active: true,
            is_working: true,
            panes: [{ pane_id: 1, label: "Working one", kind: "terminal", provider: "codex", is_working: true }],
          },
          {
            id: "session-b",
            project_name: "beta",
            status: "active",
            is_active: true,
            is_working: false,
            panes: [{ pane_id: 2, label: "Waiting one", kind: "terminal", provider: "claude", is_working: false }],
          },
          {
            id: "session-c",
            project_name: "gamma",
            status: "ended",
            is_active: false,
            is_working: false,
            panes: [{ pane_id: 3, label: "Stopped one", kind: "terminal", provider: "claude", is_working: false }],
          },
        ],
      }),
    }));
    renderHome({ active: true, legacySessions: [] });

    fireEvent.click(screen.getByRole("button", { name: "Idle sessions" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Open Waiting one in beta" })).toBeTruthy();
    });
    expect(screen.queryByRole("button", { name: /Working one/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Stopped one/ })).toBeNull();
  });

  it("does not preserve a stale bootstrap working flag once live inventory is present", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        sessions: [{
          id: "session-a",
          project_name: "alpha",
          status: "active",
          is_active: true,
          is_working: true,
        }],
      }),
    }));
    renderHome({
      active: true,
      legacySessions: [session({ id: "session-a", workingDir: "/workspace/alpha", isWorking: undefined })],
    });

    await waitFor(() => {
      expect(within(screen.getByRole("button", { name: "Open alpha" })).getByText("Idle")).toBeTruthy();
    });
  });

  it("shows the most recently messaged session first", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        sessions: [
          {
            id: "session-a",
            project_name: "alpha",
            status: "active",
            is_active: true,
            last_user_input_at: "2026-08-08T12:00:00Z",
          },
          {
            id: "session-b",
            project_name: "beta",
            status: "active",
            is_active: true,
            last_user_input_at: "2026-08-09T12:00:00Z",
          },
        ],
      }),
    }));
    renderHome({ active: true, legacySessions: [] });

    await waitFor(() => {
      const cards = screen.getAllByRole("button")
        .map((button) => button.getAttribute("aria-label"))
        .filter((label): label is string => Boolean(label?.startsWith("Open ")));
      expect(cards).toEqual(["Open beta", "Open alpha"]);
    });
  });

  it("opens cards and maps New task to an honest running-project chooser", () => {
    const props = renderHome();

    fireEvent.click(screen.getByRole("button", { name: /New task/ }));
    const dialog = screen.getByRole("dialog", { name: "Start coding work" });
    expect(dialog).toBeTruthy();
    expect(screen.getByText(/use its \+ control/)).toBeTruthy();
    const chooser = within(dialog);
    expect(chooser.getByRole("button", { name: /alpha/ })).toBeTruthy();
    expect(chooser.queryByRole("button", { name: /beta/ })).toBeNull();

    fireEvent.click(chooser.getByRole("button", { name: /alpha/ }));
    expect(props.onOpenSession).toHaveBeenCalledWith("session-a", "alpha");
  });

  it("lists machines in place of the sessions when Machines is selected", async () => {
    // The bootstrap already carries machines; the list must not cost a
    // second request.
    stubBootstrap({
      sessions: [],
      machines: [
        {
          machine: {
            machine_id: "machine-1",
            hostname: "zoo-005",
            os: "linux",
            arch: "x86_64",
            last_seen: new Date().toISOString(),
          },
          projects: [{ project_id: "p1", is_running: true }, { project_id: "p2" }],
        },
      ],
    });
    renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByText("zoo-005")).toBeTruthy();
    expect(screen.getByText(/linux\/x86_64/)).toBeTruthy();
    expect(screen.getByText(/1 project running/)).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
  });

  it("reports a machine whose daemon has gone quiet as offline", async () => {
    stubBootstrap({
      sessions: [],
      machines: [
        {
          machine: {
            machine_id: "machine-1",
            hostname: "zoo-002",
            last_seen: new Date(Date.now() - 10 * 60_000).toISOString(),
          },
          projects: [],
        },
      ],
    });
    renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByText("Offline")).toBeTruthy();
    expect(screen.getByText(/0 projects running/)).toBeTruthy();
  });

  it("reboots the daemon of the machine whose control was tapped, after confirming", async () => {
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005" }, projects: [] },
        { machine: { machine_id: "machine-b", hostname: "zoo-006" }, projects: [] },
      ],
    });
    const props = renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    fireEvent.click(await screen.findByRole("button", { name: "Reboot the daemon on zoo-006" }));
    expect(props.onRebootDaemon).not.toHaveBeenCalled();

    const dialog = screen.getByRole("dialog", { name: /Reboot the daemon on zoo-006/ });
    // The reassurance is the point: this restarts a process, not the work.
    expect(within(dialog).getByText(/keep running/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reboot" }));

    expect(props.onRebootDaemon).toHaveBeenCalledWith("machine-b", "zoo-006");
  });

  it("lists idle agents one per pane, including inside a working project", async () => {
    // The case the project-level view could not express: this project reads as
    // working, so every idle pane in it used to be invisible.
    stubBootstrap({
      sessions: [
        {
          id: "session-a",
          project_name: "mako",
          hostname: "zoo-005",
          status: "active",
          is_active: true,
          is_working: true,
          panes: [
            { pane_id: 3, label: "Claude terminal 3", kind: "terminal", provider: "claude", is_working: false },
            { pane_id: 4, label: "Codex terminal 2", kind: "terminal", provider: "codex", is_working: true },
          ],
        },
      ],
      machines: [],
    });
    renderHome({ active: true, legacySessions: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Idle sessions" }));
    expect(await screen.findByRole("button", { name: "Open Claude terminal 3 in mako" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Codex terminal 2/ })).toBeNull();
    const row = screen.getByRole("button", { name: "Open Claude terminal 3 in mako" });
    expect(row.textContent).toContain("mako");
    expect(row.textContent).toContain("zoo-005");
  });

  it("remembers the tapped agent so the session opens on it", async () => {
    stubBootstrap({
      sessions: [
        {
          id: "session-a",
          project_name: "mako",
          status: "active",
          is_active: true,
          panes: [
            { pane_id: 7, label: "Claude terminal 3", kind: "terminal", provider: "claude", is_working: false },
          ],
        },
      ],
      machines: [],
    });
    const props = renderHome({ active: true, legacySessions: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Idle sessions" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open Claude terminal 3 in mako" }));

    expect(readSelectedPane("session-a")).toBe(7);
    expect(props.onOpenSession).toHaveBeenCalledWith("session-a", "mako");
  });

  it("treats a session with no pane detail as unknown rather than idle", async () => {
    // An older server omits the field entirely; filling the list with every
    // session would be worse than showing nothing.
    stubBootstrap({
      sessions: [{ id: "session-a", project_name: "mako", status: "active", is_active: true }],
      machines: [],
    });
    renderHome({ active: true, legacySessions: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Idle sessions" }));
    expect(await screen.findByText("No idle sessions")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Open .* in mako/ })).toBeNull();
  });

  it("says so when every agent is working", async () => {
    stubBootstrap({
      sessions: [
        {
          id: "session-a",
          project_name: "mako",
          status: "active",
          is_active: true,
          is_working: true,
          panes: [{ pane_id: 3, label: "Busy", kind: "terminal", provider: "codex", is_working: true }],
        },
      ],
      machines: [],
    });
    renderHome({ active: true, legacySessions: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Idle sessions" }));
    expect(await screen.findByText("No idle sessions")).toBeTruthy();
    expect(screen.getByText(/currently working/)).toBeTruthy();
  });

  it("offers to update the machines that are behind, and only those", async () => {
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005", daemon_version: "26.08.74" }, projects: [] },
        { machine: { machine_id: "machine-b", hostname: "zoo-006", daemon_version: "26.08.70" }, projects: [] },
      ],
    });
    renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    // zoo-006 is behind a machine it can see; zoo-005 is the newest thing there is.
    expect(await screen.findByRole("button", { name: "Reboot and update the daemon on zoo-006" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reboot the daemon on zoo-005" })).toBeTruthy();
    expect(screen.getByText("Reboot to update")).toBeTruthy();
  });

  it("recognises a fleet that is uniformly behind the server", async () => {
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005", daemon_version: "26.08.74" }, projects: [] },
        { machine: { machine_id: "machine-b", hostname: "zoo-006", daemon_version: "26.08.74" }, projects: [] },
      ],
    });
    // Nothing the machines report is newer than each other, so without the
    // server's version every one of them would read as current.
    renderHome({ active: true, serverVersion: "26.09.3" });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByRole("button", { name: "Reboot and update the daemon on zoo-005" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reboot and update the daemon on zoo-006" })).toBeTruthy();
  });

  it("shows each machine's version, and says so when there is none", async () => {
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005", daemon_version: "26.08.74" }, projects: [] },
        { machine: { machine_id: "machine-b", hostname: "zoo-006" }, projects: [] },
      ],
    });
    renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByText(/26\.08\.74/)).toBeTruthy();
    expect(screen.getByText(/version unknown/)).toBeTruthy();
    // An unreadable version is not evidence of being behind.
    expect(screen.getByRole("button", { name: "Reboot the daemon on zoo-006" })).toBeTruthy();
  });

  it("sends the same request from either wording", async () => {
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005", daemon_version: "26.08.74" }, projects: [] },
        { machine: { machine_id: "machine-b", hostname: "zoo-006", daemon_version: "26.08.70" }, projects: [] },
      ],
    });
    const props = renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    fireEvent.click(await screen.findByRole("button", { name: "Reboot and update the daemon on zoo-006" }));
    const dialog = screen.getByRole("dialog", { name: /Reboot and update the daemon on zoo-006/ });
    expect(within(dialog).getByText(/behind/)).toBeTruthy();
    expect(within(dialog).getByText(/keep running/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reboot to update" }));

    expect(props.onRebootDaemon).toHaveBeenCalledWith("machine-b", "zoo-006");
  });

  it("shows the pushed machine list rather than the bootstrap snapshot", async () => {
    // The bug: bootstrap is fetched once, so a daemon restarted onto a new
    // version kept reading as the old one until the page was reloaded by hand.
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005", daemon_version: "26.08.74" }, projects: [] },
      ],
    });
    const { rerender, props } = renderHomeWith({
      active: true,
      liveMachines: [
        {
          machine: { machineId: "machine-a", hostname: "zoo-005", os: "linux", arch: "x64", daemonVersion: "26.08.74" },
          projects: [],
        },
      ],
    });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByText(/26\.08\.74/)).toBeTruthy();

    // The daemon comes back on a new version; the server pushes it.
    rerender({
      ...props,
      liveMachines: [
        {
          machine: { machineId: "machine-a", hostname: "zoo-005", os: "linux", arch: "x64", daemonVersion: "26.08.78" },
          projects: [],
        },
      ],
    });

    expect(await screen.findByText(/26\.08\.78/)).toBeTruthy();
    expect(screen.queryByText(/26\.08\.74/)).toBeNull();
  });

  it("falls back to the bootstrap snapshot until a list has been pushed", async () => {
    // A cold open is a heartbeat away from the first push, and an empty list
    // reads as "no machines" rather than "not yet".
    stubBootstrap({
      sessions: [],
      machines: [
        { machine: { machine_id: "machine-a", hostname: "zoo-005", daemon_version: "26.08.74" }, projects: [] },
      ],
    });
    renderHomeWith({ active: true, liveMachines: [] });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByText("zoo-005")).toBeTruthy();
    expect(screen.queryByText(/No machines yet/)).toBeNull();
  });

  it("asks for a machine list when the machines tab is opened", async () => {
    stubBootstrap({ sessions: [], machines: [] });
    const { props } = renderHomeWith({ active: true });

    expect(props.onRefreshMachines).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(props.onRefreshMachines).toHaveBeenCalled();
  });

  it("sends nothing when a daemon reboot is dismissed", async () => {
    stubBootstrap({
      sessions: [],
      machines: [{ machine: { machine_id: "machine-a", hostname: "zoo-005" }, projects: [] }],
    });
    const props = renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    fireEvent.click(await screen.findByRole("button", { name: "Reboot the daemon on zoo-005" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(props.onRebootDaemon).not.toHaveBeenCalled();
  });

  it("says so plainly when the account can reach no machines", async () => {
    stubBootstrap({ sessions: [], machines: [] });
    renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    expect(await screen.findByText("No machines yet")).toBeTruthy();
  });

  it("keeps Account and machine management reachable without permanent bars", () => {
    const props = renderHome({ legacySessions: [] });

    fireEvent.click(screen.getByRole("button", { name: "Account" }));
    expect(props.onAccount).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Start a task" }));
    expect(screen.getByText(/No running projects/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Manage machines and projects" }));
    expect(props.onManageMachines).toHaveBeenCalledTimes(1);
  });
});
