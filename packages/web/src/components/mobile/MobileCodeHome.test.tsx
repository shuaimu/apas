import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "@/lib/store";
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
    onRebootCli: vi.fn(),
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
  it("renders only all-project and idle-project categories", () => {
    renderHome();

    expect(screen.getByRole("heading", { name: "Coding sessions" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Account" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /New task/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: "All projects" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Idle projects" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Active" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Attention" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Completed" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Recent" })).toBeNull();
    expect(screen.getByRole("button", { name: "Open alpha" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open beta" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Idle projects" }));
    expect(screen.getByRole("button", { name: "Open alpha" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open beta" })).toBeNull();
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

  it("idle projects excludes working and offline sessions", async () => {
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
          },
          {
            id: "session-b",
            project_name: "beta",
            status: "active",
            is_active: true,
            is_working: false,
          },
          {
            id: "session-c",
            project_name: "gamma",
            status: "ended",
            is_active: false,
            is_working: false,
          },
        ],
      }),
    }));
    renderHome({ active: true, legacySessions: [] });

    fireEvent.click(screen.getByRole("button", { name: "Idle projects" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Open beta" })).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Open alpha" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Open gamma" })).toBeNull();
    });
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

  it("reboots the CLI of the session whose icon was tapped, after confirming", () => {
    const props = renderHome();

    // The control names its project, because the list shows several and a
    // mis-tap here restarts someone's work.
    fireEvent.click(screen.getByRole("button", { name: "Reboot CLI for beta" }));
    expect(props.onRebootCli).not.toHaveBeenCalled();

    const dialog = screen.getByRole("dialog", { name: /Reboot this project/ });
    expect(within(dialog).getByText(/Terminal panes keep running/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reboot CLI" }));

    // Routed by the session that was tapped, not by whatever is attached.
    expect(props.onRebootCli).toHaveBeenCalledWith("session-b", "beta");
  });

  it("does nothing when the reboot confirmation is dismissed", () => {
    const props = renderHome();

    fireEvent.click(screen.getByRole("button", { name: "Reboot CLI for alpha" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(props.onRebootCli).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: /Reboot this project/ })).toBeNull();
  });

  it("keeps opening a project distinct from rebooting it", () => {
    const props = renderHome();

    // The two controls sit a thumb-width apart; tapping the card must open,
    // never reboot.
    fireEvent.click(screen.getByRole("button", { name: "Open alpha" }));
    expect(props.onOpenSession).toHaveBeenCalledWith("session-a", "alpha");
    expect(props.onRebootCli).not.toHaveBeenCalled();
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
    fireEvent.click(await screen.findByRole("button", { name: "Reboot daemon on zoo-006" }));
    expect(props.onRebootDaemon).not.toHaveBeenCalled();

    const dialog = screen.getByRole("dialog", { name: /Reboot the daemon on zoo-006/ });
    // The reassurance is the point: this restarts a process, not the work.
    expect(within(dialog).getByText(/keep running/)).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Reboot daemon" }));

    expect(props.onRebootDaemon).toHaveBeenCalledWith("machine-b", "zoo-006");
  });

  it("sends nothing when a daemon reboot is dismissed", async () => {
    stubBootstrap({
      sessions: [],
      machines: [{ machine: { machine_id: "machine-a", hostname: "zoo-005" }, projects: [] }],
    });
    const props = renderHome({ active: true });

    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    fireEvent.click(await screen.findByRole("button", { name: "Reboot daemon on zoo-005" }));
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
