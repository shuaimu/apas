import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import MachinesPage from "./page";
import { validateClientMessage } from "@apas/protocol";
import { handleServerMessage, useStore, type MachineInfo, type MachineWithProjects } from "@/lib/store";
import {
  CLOSE_DELETES_NOTHING,
  NOTHING_SENT_TEXT,
  PROJECT_OPEN_STATE_CAPABILITY,
  STOP_RUNTIME_CLOSES_PROJECT,
  SUSPEND_CLOSES_PROJECT,
} from "@/lib/projectOpenState";

const routerPush = vi.hoisted(() => vi.fn());

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: routerPush }),
}));

const initialStore = useStore.getState();
const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

const policy = {
  team_available: false,
  allowed_launch_profiles: ["terminal:codex:official:default"],
  version: 3,
  project_suspended: false,
};

function apiResponse(body: unknown, ok = true, status = ok ? 200 : 403) {
  return { ok, status, json: vi.fn().mockResolvedValue(body) };
}

/**
 * The server scopes every /cluster route to the caller's own cluster, so the
 * fixture returns only what this account hosts and rejects anything else.
 */
function installClusterApi({ projectDenied = false } = {}) {
  fetchMock.mockImplementation(async (input: string | URL) => {
    const url = String(input);
    if (url.includes("/cluster/projects?")) {
      return apiResponse({
        items: [{
          id: "project-a",
          project_name: "mako-soumojit",
          hostname: "build-host",
          owner_user_id: "owner-1",
          owner_email: "owner@example.com",
          lifecycle_status: "active",
          member_count: 1,
          active_session_count: 1,
          hosting_emails: ["me@example.com"],
          connected: true,
          effective_policy: policy,
        }],
        limit: 200,
        offset: 0,
      });
    }
    if (url.match(/\/cluster\/projects\/project-a$/)) {
      if (projectDenied) {
        return apiResponse({ message: "That project is not in your cluster" }, false, 403);
      }
      return apiResponse({
        project: {
          id: "project-a",
          project_name: "mako-soumojit",
          hostname: "build-host",
          owner_user_id: "owner-1",
          owner_email: "owner@example.com",
          lifecycle_status: "active",
          member_count: 1,
          active_session_count: 1,
          hosting_emails: ["me@example.com"],
        },
        members: [{ user_id: "user-1", email: "member@example.com" }],
        policy,
      });
    }
    if (url.endsWith("/cluster/policy/default")) {
      return apiResponse({
        cluster: null,
        deployment: {
          team_available: true,
          allowed_launch_profiles: ["terminal:codex:official:default"],
          version: 3,
          project_suspended: false,
        },
      });
    }
    if (url.endsWith("/cluster/launch-profiles")) {
      return apiResponse([
        { key: "terminal:codex:official:default", label: "Codex Terminal" },
        { key: "agent:claude:glm:glm-5.1", label: "Legacy GLM" },
      ]);
    }
    if (url.includes("/cluster/audit")) {
      return apiResponse({
        items: [{
          id: 4,
          actor_kind: "user",
          actor_user_id: "me",
          action: "project.runtime_stopped",
          target_type: "project",
          target_id: "project-a",
          created_at: "2026-08-14T09:00:00Z",
        }],
        limit: 25,
        offset: 0,
      });
    }
    return apiResponse({ success: true });
  });
}

function machineEntry(): MachineWithProjects {
  return {
    machine: {
      machineId: "machine-1",
      hostname: "build-host",
      os: "linux",
      arch: "x64",
      lastSeen: "2026-06-17T05:00:00Z",
      // A key the store should never hold, as if one slipped through: the
      // page must still not show it anywhere.
      deepseekBackend: {
        apiBaseUrl: "https://api.deepseek.com/anthropic",
        apiKey: "sk-deepseek",
        apiKeyConfigured: true,
      } as NonNullable<MachineInfo["deepseekBackend"]>,
    },
    projects: [
      {
        projectId: "project-running",
        name: "Active API",
        path: "/srv/active-api",
        isRunning: true,
        pid: 4242,
        memoryKb: 2 * 1024 * 1024,
      },
      {
        projectId: "project-stopped",
        name: "Stopped API",
        path: "/srv/stopped-api",
        isRunning: false,
      },
    ],
  };
}

function machineAt(machineId: string, hostname: string, daemonVersion?: string): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x64", daemonVersion },
    projects: [],
  };
}

function seedMachines(machines: MachineWithProjects[] = [machineEntry()]) {
  const actions = {
    connect: vi.fn(),
    listMachines: vi.fn(),
    // Each reports that it sent, as the store does while the socket is open.
    openProject: vi.fn().mockReturnValue(true),
    closeProject: vi.fn().mockReturnValue(true),
    setMachineDeepseekConfig: vi.fn().mockReturnValue(true),
    rebootDaemon: vi.fn(),
    rebootDaemons: vi.fn(),
  };

  window.localStorage.setItem("apas_token", "test-token");
  act(() => {
    useStore.setState({
      token: "test-token",
      connected: true,
      machines,
      usageLimits: new Map(),
      ...actions,
    });
  });

  return actions;
}

function openView(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
}

function confirmInDialog(name: string) {
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name }));
}

beforeEach(() => {
  // Each test starts on the page's own address, also when run alone.
  window.history.replaceState({}, "", "/machines");
  fetchMock.mockReset();
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  installClusterApi();
});

afterEach(() => {
  routerPush.mockReset();
  window.localStorage.clear();
  window.history.replaceState({}, "", "/machines");
  globalThis.fetch = originalFetch;
  act(() => {
    useStore.setState(initialStore, true);
  });
});

describe("MachinesPage daemon restart", () => {
  it("offers the restart control on every machine, which the page never had", () => {
    seedMachines([
      machineAt("machine-a", "zoo-005", "26.08.74"),
      machineAt("machine-b", "zoo-006", "26.08.74"),
    ]);

    render(<MachinesPage />);

    expect(screen.getByRole("button", { name: "Reboot the daemon on zoo-005" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reboot the daemon on zoo-006" })).toBeTruthy();
  });

  it("shows each machine's version, and says so when it reports none", () => {
    seedMachines([machineAt("machine-a", "zoo-005", "26.08.74"), machineAt("machine-b", "zoo-006")]);

    render(<MachinesPage />);

    expect(screen.getByText(/26\.08\.74/)).toBeTruthy();
    expect(screen.getByText(/version unknown/)).toBeTruthy();
    // Unknown is not evidence of being behind.
    expect(screen.getByRole("button", { name: "Reboot the daemon on zoo-006" })).toBeTruthy();
  });

  it("says a restart will update the machines that are behind", () => {
    seedMachines([
      machineAt("machine-a", "zoo-005", "26.08.74"),
      machineAt("machine-b", "zoo-006", "26.08.70"),
    ]);

    render(<MachinesPage />);

    expect(screen.getByRole("button", { name: "Reboot and update the daemon on zoo-006" })).toBeTruthy();
    expect(screen.getByText("Reboot to update")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reboot the daemon on zoo-005" })).toBeTruthy();
  });

  it("recognises a fleet uniformly behind the server", () => {
    seedMachines([machineAt("machine-a", "zoo-005", "26.08.74")]);
    act(() => {
      useStore.setState({ serverVersion: "26.09.3" });
    });

    render(<MachinesPage />);

    expect(screen.getByRole("button", { name: "Reboot and update the daemon on zoo-005" })).toBeTruthy();
  });

  it("confirms before sending, and sends for the machine whose control was used", () => {
    const actions = seedMachines([
      machineAt("machine-a", "zoo-005", "26.08.74"),
      machineAt("machine-b", "zoo-006", "26.08.70"),
    ]);

    render(<MachinesPage />);
    fireEvent.click(screen.getByRole("button", { name: "Reboot and update the daemon on zoo-006" }));
    expect(actions.rebootDaemon).not.toHaveBeenCalled();

    const dialog = screen.getByRole("dialog", { name: "Reboot and update the daemon on zoo-006" });
    // The visible title names the machine too.
    expect(within(dialog).getByRole("heading", { name: "Reboot and update the daemon on zoo-006?" })).toBeTruthy();
    // Projects run inside the daemon: the dialog must not promise they keep running.
    expect(within(dialog).getByText(/Every project running on zoo-006 restarts with it\. Terminal agents normally keep running/)).toBeTruthy();
    expect(within(dialog).queryByText(/does not own/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Reboot to update" }));

    expect(actions.rebootDaemon).toHaveBeenCalledWith("machine-b");
  });

  it("sends nothing when the confirmation is dismissed", () => {
    const actions = seedMachines([machineAt("machine-a", "zoo-005", "26.08.74")]);

    render(<MachinesPage />);
    fireEvent.click(screen.getByRole("button", { name: "Reboot the daemon on zoo-005" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(actions.rebootDaemon).not.toHaveBeenCalled();
  });

  it("confirms once and requests every daemon in My Cluster", () => {
    const actions = seedMachines([
      machineAt("machine-a", "zoo-005", "26.08.74"),
      machineAt("machine-b", "zoo-006", "26.08.70"),
    ]);

    render(<MachinesPage />);
    fireEvent.click(screen.getByRole("button", { name: "Reboot all daemons" }));
    expect(actions.rebootDaemons).not.toHaveBeenCalled();

    const dialog = screen.getByRole("dialog", { name: "Reboot all daemons" });
    expect(dialog.textContent).toContain("all 2 machines");
    expect(dialog.textContent).toContain("1 daemon is behind");
    expect(dialog.textContent).toContain("zoo-005, zoo-006");
    fireEvent.click(screen.getAllByRole("button", { name: "Reboot all daemons" })[1]);

    expect(actions.rebootDaemons).toHaveBeenCalledWith(["machine-a", "machine-b"]);
  });
});

describe("MachinesPage provider availability", () => {
  it("lists each machine's installed providers quietly, with versions in the title", () => {
    const mac = machineAt("machine-mac", "Shuais-MacBook-Air-2.local", "26.11.54");
    mac.machine.providers = [
      { provider: "omp", installed: false, canInstall: true },
      { provider: "codex", installed: true, version: "0.160.0", canInstall: false },
      { provider: "claude", installed: true, version: "2.1.233", canInstall: false },
    ];
    const empty = machineAt("machine-empty", "zoo-007", "26.11.54");
    empty.machine.providers = [];
    seedMachines([mac, empty, machineAt("machine-old", "zoo-005", "26.10.9")]);

    render(<MachinesPage />);

    const line = screen.getByText("Providers: Claude, Codex");
    expect(line.getAttribute("title")).toBe("Claude 2.1.233, Codex 0.160.0");
    expect(screen.getByText("No providers installed")).toBeTruthy();
    // An older daemon reports nothing, which is not "none installed": its
    // row is as it was.
    expect(screen.getAllByText(/^Providers:|^No providers installed$/)).toHaveLength(2);
  });
});

describe("MachinesPage", () => {
  it("renders machine identity, supported backend state, projects, and the empty state", () => {
    const actions = seedMachines();

    const { unmount } = render(<MachinesPage />);

    expect(actions.listMachines).toHaveBeenCalledTimes(1);
    expect(screen.getAllByText("build-host").length).toBeGreaterThan(0);
    expect(screen.getByText(/linux\/x64/)).toBeTruthy();
    expect(screen.getByRole("heading", { name: "DeepSeek keys" })).toBeTruthy();
    expect(screen.getByText("API key configured")).toBeTruthy();
    expect(screen.queryByText(/MiniMax/i)).toBeNull();
    expect(screen.queryByText(/GLM/i)).toBeNull();
    expect(screen.getByText("Active API")).toBeTruthy();
    expect(screen.getByText("Stopped API")).toBeTruthy();
    expect(screen.getByText("Running · 2.0 GiB").getAttribute("title")).toBe("pid 4242");
    // The project's state, not "Running" / "Stopped": rows that run here
    // come first.
    expect(screen.getAllByTestId("project-open-state").map((chip) => chip.textContent)).toEqual(["Open", "Closed"]);
    expect(screen.queryByText("Stopped")).toBeNull();
    expect(screen.getByText("My Cluster")).toBeTruthy();
    expect(screen.queryByText(/No machines reported yet/)).toBeNull();
    // The key is never shown, prefilled or put in an input.
    expect(screen.queryByDisplayValue("sk-deepseek")).toBeNull();
    expect(document.body.textContent).not.toContain("sk-deepseek");

    unmount();
    seedMachines([]);
    render(<MachinesPage />);

    expect(screen.getByText(/No machines reported yet/)).toBeTruthy();
  });

  it("wires DeepSeek Save and Clear controls to the retained store action", () => {
    const actions = seedMachines();

    render(<MachinesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Replace DeepSeek API key for build-host" }));
    const input = screen.getByLabelText("DeepSeek API key for build-host") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.value).toBe("");
    const save = screen.getByRole("button", { name: "Save DeepSeek API key for build-host" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "  sk-deepseek-new  " } });
    fireEvent.click(save);
    expect(actions.setMachineDeepseekConfig).toHaveBeenCalledWith(
      "machine-1",
      "sk-deepseek-new",
      false,
    );
    expect(screen.queryByDisplayValue(/sk-deepseek-new/)).toBeNull();
    expect(screen.getByText("Key sent to build-host.")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Clear DeepSeek API key for build-host" }));
    expect(actions.setMachineDeepseekConfig).toHaveBeenCalledTimes(1);
    confirmInDialog("Clear key");
    expect(actions.setMachineDeepseekConfig).toHaveBeenLastCalledWith("machine-1", undefined, true);
  });

  it("sends nothing when the Clear key confirmation is dismissed with Escape", () => {
    const actions = seedMachines();

    render(<MachinesPage />);
    fireEvent.click(screen.getByRole("button", { name: "Clear DeepSeek API key for build-host" }));
    fireEvent.keyDown(screen.getByRole("dialog", { name: "Clear the DeepSeek API key on build-host?" }), { key: "Escape" });

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(actions.setMachineDeepseekConfig).not.toHaveBeenCalled();
  });

  it("wires Refresh and each project's Open and Close to the store, after a confirmation", () => {
    const actions = seedMachines();

    render(<MachinesPage />);
    actions.listMachines.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "Refresh machines" }));
    expect(actions.listMachines).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" }));
    // Nothing is sent before the confirmation.
    expect(actions.openProject).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Open Stopped API?" })).toBeTruthy();
    confirmInDialog("Open");
    expect(actions.openProject).toHaveBeenCalledTimes(1);
    expect(actions.openProject).toHaveBeenCalledWith("machine-1", "project-stopped");
    const opening = screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" }) as HTMLButtonElement;
    expect(opening.textContent).toContain("Opening…");
    expect(opening.disabled).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Close Active API (listed on build-host)" }));
    expect(actions.closeProject).not.toHaveBeenCalled();
    const dialog = screen.getByRole("dialog", { name: "Close Active API?" });
    expect(dialog.textContent).toContain(CLOSE_DELETES_NOTHING);
    confirmInDialog("Close");
    expect(actions.closeProject).toHaveBeenCalledTimes(1);
    expect(actions.closeProject).toHaveBeenCalledWith("machine-1", "project-running");

    // One machine alone is never started or stopped from this page.
    expect(screen.queryByRole("button", { name: /^(Start|Stop) / })).toBeNull();
  });

  it("sends nothing when an Open or Close confirmation is dismissed", () => {
    const actions = seedMachines();

    render(<MachinesPage />);
    fireEvent.click(screen.getByRole("button", { name: "Close Active API (listed on build-host)" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" }));
    fireEvent.keyDown(screen.getByRole("dialog", { name: "Open Stopped API?" }), { key: "Escape" });

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(actions.openProject).not.toHaveBeenCalled();
    expect(actions.closeProject).not.toHaveBeenCalled();
  });
});

describe("MachinesPage open state", () => {
  const PROJECT = "30000000-0000-4000-8000-000000000001";
  const ZOO5 = "50000000-0000-4000-8000-000000000001";
  const ZOO6 = "50000000-0000-4000-8000-000000000002";

  function listing(
    machineId: string,
    hostname: string,
    project: Partial<MachineWithProjects["projects"][number]>,
  ): MachineWithProjects {
    return {
      machine: { machineId, hostname, os: "linux", arch: "x64" },
      projects: [{ projectId: PROJECT, name: "mako", path: "/work/mako", isRunning: false, ...project }],
      clusterAccess: "owner",
      daemonConnected: true,
      projectInstancesAvailable: true,
    };
  }

  /** The store's own `openProject` / `closeProject`, over a socket that
   *  records what is sent. */
  function seedOverSocket(machines: MachineWithProjects[], { capable = true } = {}) {
    const send = vi.fn();
    window.localStorage.setItem("apas_token", "test-token");
    act(() => {
      useStore.setState({
        token: "test-token",
        connected: true,
        machines,
        usageLimits: new Map(),
        negotiatedCapabilities: new Set(capable ? [PROJECT_OPEN_STATE_CAPABILITY] : []),
        ws: { readyState: WebSocket.OPEN, send } as unknown as WebSocket,
        connect: vi.fn(),
        listMachines: vi.fn(),
      });
    });
    return () => send.mock.calls.map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>);
  }

  const dialogLines = () => Array.from(screen.getByRole("dialog").querySelectorAll("p")).map((line) => line.textContent);

  it("opens the whole project from one machine's entry, naming the other machine first", () => {
    const sent = seedOverSocket([
      listing(ZOO5, "zoo-005", { openState: "closed", part: "not_started", otherHosts: [{ hostname: "zoo-006", connected: true, running: false, ownsTabs: true }] }),
      listing(ZOO6, "zoo-006", { openState: "closed", part: "not_started", otherHosts: [{ hostname: "zoo-005", connected: true, running: false, ownsTabs: true }] }),
    ]);
    render(<MachinesPage />);

    expect(screen.getAllByTestId("project-open-state").map((chip) => chip.textContent)).toEqual(["Closed", "Closed"]);
    fireEvent.click(screen.getByRole("button", { name: "Open mako (listed on zoo-005)" }));
    expect(dialogLines()).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    expect(sent()).toEqual([]);
    confirmInDialog("Open");

    const frames = sent();
    expect(frames).toEqual([{
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: PROJECT,
      all_instances: true,
    }]);
    expect(validateClientMessage(frames[0]).valid).toBe(true);
  });

  it("closes the whole project from one machine's entry, naming both machines", () => {
    const sent = seedOverSocket([
      listing(ZOO5, "zoo-005", { isRunning: true, openState: "open", part: "running", otherHosts: [{ hostname: "zoo-006", connected: true, running: true, ownsTabs: true }] }),
      listing(ZOO6, "zoo-006", { isRunning: true, openState: "open", part: "running", otherHosts: [{ hostname: "zoo-005", connected: true, running: true, ownsTabs: true }] }),
    ]);
    render(<MachinesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Close mako (listed on zoo-006)" }));
    expect(dialogLines()).toEqual(["This stops its agents on zoo-005 and zoo-006.", CLOSE_DELETES_NOTHING]);
    confirmInDialog("Close");

    const frames = sent();
    expect(frames).toEqual([{
      type: "stop_machine_project_cli",
      machine_id: ZOO6,
      project_id: PROJECT,
      all_instances: true,
    }]);
    expect(validateClientMessage(frames[0]).valid).toBe(true);
  });

  it("offers to keep open and to close a project nobody has recorded, and records it with one request", () => {
    const sent = seedOverSocket([listing(ZOO5, "zoo-005", { isRunning: true, part: "running" })]);
    render(<MachinesPage />);

    expect(screen.getByRole("button", { name: "Close mako (listed on zoo-005)" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Keep mako open (listed on zoo-005)" }));
    expect(dialogLines()).toEqual([
      "This records the project as open. Nothing starts now.",
      "Its tabs start again on zoo-005 after those machines have been away.",
    ]);
    confirmInDialog("Keep open");
    expect(sent()).toEqual([{
      type: "start_machine_project_cli",
      machine_id: ZOO5,
      project_id: PROJECT,
      all_instances: true,
    }]);
  });

  it("shows the server's new state after a request, and sends nothing again", () => {
    const sent = seedOverSocket([listing(ZOO5, "zoo-005", { openState: "closed", part: "not_started" })]);
    render(<MachinesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Open mako (listed on zoo-005)" }));
    confirmInDialog("Open");
    expect(screen.getByRole("button", { name: "Open mako (listed on zoo-005)" }).textContent).toContain("Opening…");

    act(() => {
      useStore.setState({ machines: [listing(ZOO5, "zoo-005", { isRunning: true, openState: "open", part: "running" })] });
    });
    expect(screen.getByTestId("project-open-state").textContent).toBe("Open");
    expect(screen.getByTestId("machine-part").textContent).toBe("Running");
    expect(screen.queryByRole("button", { name: /^Open/ })).toBeNull();
    expect(screen.getByText("1 of 1 projects running")).toBeTruthy();
    expect(sent()).toHaveLength(1);
  });

  it("never offers Keep open on a server that records no open state, and still asks for the whole project", () => {
    const sent = seedOverSocket([
      listing(ZOO5, "zoo-005", { isRunning: true }),
      { ...listing(ZOO6, "zoo-006", {}), projects: [{ projectId: "other", name: "idle", path: "/work/idle", isRunning: false }] },
    ], { capable: false });
    render(<MachinesPage />);

    expect(screen.queryByRole("button", { name: /^Keep/ })).toBeNull();
    expect(screen.getAllByTestId("project-open-state").map((chip) => chip.textContent)).toEqual(["Open", "Closed"]);
    fireEvent.click(screen.getByRole("button", { name: "Open idle (listed on zoo-006)" }));
    confirmInDialog("Open");
    fireEvent.click(screen.getByRole("button", { name: "Close mako (listed on zoo-005)" }));
    confirmInDialog("Close");
    expect(sent()).toEqual([
      { type: "start_machine_project_cli", machine_id: ZOO6, project_id: "other", all_instances: true },
      { type: "stop_machine_project_cli", machine_id: ZOO5, project_id: PROJECT, all_instances: true },
    ]);
  });
});

describe("MachinesPage cluster administration", () => {
  it("lists the projects hosted in this cluster, including one another account owns", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Projects");

    expect(await screen.findByText("Projects in this cluster")).toBeTruthy();
    expect(await screen.findByText("mako-soumojit")).toBeTruthy();
    expect(screen.getByText(/Owner owner@example.com/)).toBeTruthy();
  });

  it("suspends a hosted project and stops its runtime", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Suspend project" }));
    expect(fetchMock).not.toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/lifecycle",
      expect.anything(),
    );
    // This server did not say it records open states, so nothing is
    // claimed about the project being closed.
    expect(screen.getByRole("dialog").textContent).not.toContain("closes the project");
    confirmInDialog("Suspend project");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/lifecycle",
      expect.objectContaining({ method: "PATCH", body: JSON.stringify({ status: "suspended" }) }),
    ));

    fireEvent.click(await screen.findByRole("button", { name: "Stop runtime" }));
    confirmInDialog("Stop runtime");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/stop-runtime",
      expect.objectContaining({ method: "POST" }),
    ));
  });

  it("manages members and ownership without joining the project", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    expect(await screen.findByText("member@example.com")).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Member user ID"), { target: { value: "user-2" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/members",
      expect.objectContaining({ method: "POST", body: JSON.stringify({ user_id: "user-2" }) }),
    ));

    fireEvent.click(await screen.findByRole("button", { name: "Remove member@example.com" }));
    confirmInDialog("Remove");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/members/user-1",
      expect.objectContaining({ method: "DELETE" }),
    ));
  });

  it("adds a cluster member directly with selected machine and agent", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Sharing");

    expect(await screen.findByText("Share machine access")).toBeTruthy();
    expect(screen.getByText(/choose the machines it can run on/i)).toBeTruthy();
    expect(screen.getByText(/default AI agent for projects it creates here/i)).toBeTruthy();
    expect(screen.getByText(/does not share any existing or unrelated project/i)).toBeTruthy();
    expect(screen.getByText(/selected hosting machine/i)).toBeTruthy();
    fireEvent.change(await screen.findByLabelText("Member account email"), {
      target: { value: "new-member@example.com" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: "build-host" }));
    await waitFor(() => expect(
      (screen.getByLabelText("Default AI agent for new member projects") as HTMLSelectElement).value,
    ).toBe("terminal:codex:official:default"));
    fireEvent.click(screen.getByRole("button", { name: "Add member" }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/members",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          email: "new-member@example.com",
          allowed_machine_ids: ["machine-1"],
          default_launch_profile: "terminal:codex:official:default",
        }),
      }),
    ));
  });

  it("saves a cluster default policy and hides retired profiles", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Policies");

    expect(await screen.findByText("Cluster default policy")).toBeTruthy();
    expect(screen.getAllByText("Codex Terminal").length).toBeGreaterThan(0);
    expect(screen.queryByText("Legacy GLM")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Save cluster policy" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/policy/default",
      expect.objectContaining({ method: "PATCH" }),
    ));
  });

  it("shows this cluster's activity", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Activity");

    expect(await screen.findByText("Cluster activity")).toBeTruthy();
    // In words, with the server's code kept in the title.
    expect((await screen.findByText("Runtime stopped")).getAttribute("title")).toBe("project.runtime_stopped");
    expect(screen.getByText("The latest event")).toBeTruthy();
  });

  it("refreshes owner membership state after revocation", async () => {
    let memberActive = true;
    fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
      const url = String(input);
      if (init?.method === "DELETE" && url.endsWith("/cluster/members/member-1")) {
        memberActive = false;
        return apiResponse({ success: true });
      }
      if (url.endsWith("/cluster/contexts")) return apiResponse([
        { owner_user_id: "me", owner_email: "me@example.com", access: "owner" },
      ]);
      if (url.includes("/cluster/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      if (url.endsWith("/cluster/launch-profiles")) return apiResponse([]);
      if (url.includes("/cluster/audit")) return apiResponse({ items: [], limit: 25, offset: 0 });
      if (url.endsWith("/cluster/members")) return apiResponse(memberActive ? [{
        user_id: "member-1",
        user_email: "member@example.com",
        status: "active",
        allowed_machine_ids: null,
        default_launch_profile: "terminal:codex:official:default",
      }] : []);
      if (url.includes("/cluster/usage")) return apiResponse({ success: true });
      return apiResponse({ success: true });
    });
    seedMachines();
    render(<MachinesPage />);
    openView("Sharing");

    expect(await screen.findByText("member@example.com")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Revoke access for member@example.com" }));
    confirmInDialog("Revoke access");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/members/member-1",
      expect.objectContaining({ method: "DELETE" }),
    ));
    expect(await screen.findByText("No members have access to this cluster.")).toBeTruthy();
    expect(screen.queryByText("member@example.com")).toBeNull();
  });

  it("renders usage windows and distinguishes unavailable cost from zero", async () => {
    const counters = {
      prompts: 2,
      responses: 2,
      input_tokens: 10,
      output_tokens: 5,
      cost_usd: 0,
      cost_usd_reported: false,
    };
    fetchMock.mockImplementation(async (input: string | URL) => {
      const url = String(input);
      if (url.endsWith("/cluster/contexts")) return apiResponse([
        { owner_user_id: "me", owner_email: "me@example.com", access: "owner" },
      ]);
      if (url.includes("/cluster/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      if (url.endsWith("/cluster/launch-profiles")) return apiResponse([]);
      if (url.includes("/cluster/audit")) return apiResponse({ items: [], limit: 25, offset: 0 });
      if (url.endsWith("/cluster/invitations") || url.endsWith("/cluster/members")) return apiResponse([]);
      if (url.includes("/cluster/usage")) return apiResponse({
        lifetime: counters,
        last_7d: counters,
        today: counters,
        projects: [{
          project_id: "member-project",
          project_name: "Member project",
          owner_email: "member@example.com",
          usage: { lifetime: counters, last_7d: counters, today: counters },
        }],
      });
      return apiResponse({ success: true });
    });
    seedMachines();
    render(<MachinesPage />);
    openView("Usage");

    expect(await screen.findByRole("heading", { name: "Usage" })).toBeTruthy();
    expect(await screen.findByText("Member project")).toBeTruthy();
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
    const today = within(screen.getByRole("group", { name: "Usage period" })).getByRole("button", { name: "Today" });
    fireEvent.click(today);
    expect(today.getAttribute("aria-pressed")).toBe("true");
  });

  it("surfaces the server's refusal for a project outside this cluster", async () => {
    seedMachines();
    installClusterApi({ projectDenied: true });
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    expect(await screen.findByText("That project is not in your cluster")).toBeTruthy();
    expect(screen.queryByText("Members")).toBeNull();
  });

  it("renders a shared cluster without owner-only controls or secrets", async () => {
    fetchMock.mockImplementation(async (input: string | URL) => {
      const url = String(input);
      if (url.endsWith("/cluster/contexts")) return apiResponse([
        { owner_user_id: "me", owner_email: "me@example.com", access: "owner" },
        { owner_user_id: "host", owner_email: "host@example.com", access: "member" },
      ]);
      if (url.includes("/cluster/contexts/host/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/contexts/host/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      if (url.endsWith("/cluster/launch-profiles")) return apiResponse([]);
      if (url.includes("/cluster/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      if (url.includes("/cluster/audit")) return apiResponse({ items: [], limit: 25, offset: 0 });
      return apiResponse({ success: true });
    });
    const shared = machineAt("shared-machine", "host-box", "26.08.74");
    shared.clusterOwnerUserId = "host";
    shared.clusterAccess = "member";
    shared.sharedProvisioningAvailable = true;
    seedMachines([shared]);
    render(<MachinesPage />);

    const selector = await screen.findByLabelText("Selected cluster");
    fireEvent.change(selector, { target: { value: "host" } });
    expect(await screen.findByText("Trusted compute boundary")).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "DeepSeek keys" })).toBeNull();
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(2);
    expect(tabs[0]).toBe(screen.getByRole("tab", { name: "Machines" }));
    expect(tabs[1]).toBe(screen.getByRole("tab", { name: "Projects" }));
    expect(screen.queryByText("Cluster default policy")).toBeNull();
    expect(screen.queryByText("Cluster activity")).toBeNull();
    expect(screen.queryByRole("button", { name: /Restart daemon on host-box/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reboot all daemons" })).toBeNull();
  });
});

describe("registration completion navigation", () => {
  it("overrides a remembered shared cluster and reveals the closed target with an explicit Open", async () => {
    const own = machineEntry();
    own.clusterOwnerUserId = "owner";
    own.clusterAccess = "owner";
    const shared = { ...machineAt("shared", "shared-host"), clusterOwnerUserId: "guest-host", clusterAccess: "member" as const };
    const actions = seedMachines([own, shared]);
    localStorage.setItem("apas_mobile_cluster_owner", "guest-host");
    window.history.replaceState({}, "", "/machines?cluster_owner=owner&machine=machine-1&project=project-stopped#project-machine-1-project-stopped");
    const fallback = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation((input: string | URL) => String(input).endsWith("/cluster/contexts")
      ? Promise.resolve(apiResponse([
        { owner_user_id: "owner", owner_email: "owner@example.com", access: "owner" },
        { owner_user_id: "guest-host", owner_email: "shared@example.com", access: "member" },
      ]))
      : fallback(input));
    render(<MachinesPage />);
    await waitFor(() => expect((screen.getByLabelText("Selected cluster") as HTMLSelectElement).value).toBe("owner"));
    expect(localStorage.getItem("apas_mobile_cluster_owner")).toBe("owner");
    expect(screen.queryByText("shared-host")).toBeNull();
    const row = document.getElementById("project-machine-1-project-stopped");
    expect(row).toBeTruthy();
    expect(document.activeElement).toBe(row);
    expect(screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" })).toBeTruthy();
    expect(actions.openProject).not.toHaveBeenCalled();
  });

  it("distinguishes clone and local pending operations without treating paths as remotes", () => {
    seedMachines();
    act(() => useStore.setState({ pendingInstances: {
      clone: { source: "clone", status: "pending", requestId: "clone", machineId: "machine-1", instanceName: "repo", gitRemote: "github.com/a/b", startedAt: 1 },
      local: { source: "local", status: "pending", requestId: "local", machineId: "machine-1", path: "/plain/folder", startedAt: 2 },
    } }));
    render(<MachinesPage />);
    expect(screen.getByText("Cloning github.com/a/b…")).toBeTruthy();
    expect(screen.getByText("/plain/folder")).toBeTruthy();
    expect(screen.getByText("Registering existing folder…")).toBeTruthy();
  });
});

describe("views and the address", () => {
  function selectedTab() {
    return screen.getAllByRole("tab").find((tab) => tab.getAttribute("aria-selected") === "true");
  }

  function stoppedMachine(count: number): MachineWithProjects {
    return {
      machine: { machineId: "machine-1", hostname: "build-host", os: "linux", arch: "x64" },
      projects: Array.from({ length: count }, (_, index) => ({
        projectId: `project-${index + 1}`,
        name: `Stopped ${index + 1}`,
        path: `/srv/stopped-${index + 1}`,
        isRunning: false,
      })),
    };
  }

  it("opens Machines for a one-machine link even when another view is asked for", async () => {
    seedMachines();
    window.history.replaceState({}, "", "/machines?view=usage&machine=machine-1&project=project-stopped");
    render(<MachinesPage />);

    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Machines" }));
    await waitFor(() => expect(document.activeElement).toBe(document.getElementById("project-machine-1-project-stopped")));
  });

  it("unfolds the projects a machine is not running to reveal a linked row, and opens nothing", async () => {
    const actions = seedMachines([stoppedMachine(5)]);
    window.history.replaceState({}, "", "/machines?machine=machine-1&project=project-4#project-machine-1-project-4");
    render(<MachinesPage />);

    const row = document.getElementById("project-machine-1-project-4");
    expect(row).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(row));
    expect(screen.getByRole("button", { name: "Hide projects not running here" })).toBeTruthy();
    expect(actions.openProject).not.toHaveBeenCalled();
  });

  it("folds more than three projects a machine is not running behind one line", () => {
    seedMachines([stoppedMachine(5)]);
    render(<MachinesPage />);

    expect(screen.queryByText("Stopped 1")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show 5 projects not running here" }));
    expect(screen.getByText("Stopped 1")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Hide projects not running here" })).toBeTruthy();
  });

  it("moves between views with the arrow, Home and End keys", () => {
    seedMachines();
    render(<MachinesPage />);

    const tablist = screen.getByRole("tablist", { name: "Cluster views" });
    fireEvent.keyDown(tablist, { key: "ArrowRight" });
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Projects" }));
    expect(document.activeElement).toBe(screen.getByRole("tab", { name: "Projects" }));
    fireEvent.keyDown(tablist, { key: "End" });
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Activity" }));
    fireEvent.keyDown(tablist, { key: "ArrowRight" });
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Machines" }));
    fireEvent.keyDown(tablist, { key: "ArrowLeft" });
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Activity" }));
    fireEvent.keyDown(tablist, { key: "Home" });
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Machines" }));
  });

  it("opens the view the address names", async () => {
    seedMachines();
    window.history.replaceState({}, "", "/machines?view=activity");
    render(<MachinesPage />);

    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Activity" }));
    expect(await screen.findByText("Runtime stopped")).toBeTruthy();
  });

  it("writes the view into the address and drops the one-machine link", () => {
    seedMachines();
    window.history.replaceState({}, "", "/machines?cluster_owner=owned&machine=machine-1&project=project-stopped#project-machine-1-project-stopped");
    render(<MachinesPage />);

    openView("Projects");
    const url = new URL(window.location.href);
    expect(url.searchParams.get("view")).toBe("projects");
    expect(url.searchParams.get("cluster_owner")).toBe("owned");
    expect(url.searchParams.has("machine")).toBe(false);
    expect(url.searchParams.has("project")).toBe(false);
    expect(url.hash).toBe("");
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Projects" }));
  });

  it("shows Machines in a shared cluster that offers no asked-for owner view", async () => {
    fetchMock.mockImplementation(async (input: string | URL) => {
      const url = String(input);
      if (url.endsWith("/cluster/contexts")) return apiResponse([
        { owner_user_id: "me", owner_email: "me@example.com", access: "owner" },
        { owner_user_id: "host", owner_email: "host@example.com", access: "member" },
      ]);
      if (url.includes("/cluster/contexts/host/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/contexts/host/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      if (url.endsWith("/cluster/launch-profiles")) return apiResponse([]);
      return apiResponse({ success: true });
    });
    const shared = machineAt("shared-machine", "host-box", "26.08.74");
    shared.clusterOwnerUserId = "host";
    shared.clusterAccess = "member";
    seedMachines([shared]);
    localStorage.setItem("apas_mobile_cluster_owner", "host");
    window.history.replaceState({}, "", "/machines?view=policies");
    render(<MachinesPage />);

    expect(await screen.findByText("Trusted compute boundary")).toBeTruthy();
    expect(screen.queryByRole("tab", { name: "Policies" })).toBeNull();
    expect(selectedTab()).toBe(screen.getByRole("tab", { name: "Machines" }));
    expect(screen.queryByText("Cluster default policy")).toBeNull();
  });

  it("opens the project the address selects", async () => {
    seedMachines();
    window.history.replaceState({}, "", "/machines?view=projects&selected=project-a");
    render(<MachinesPage />);

    expect(await screen.findByText("member@example.com")).toBeTruthy();
    expect(fetchMock).toHaveBeenCalledWith("https://apas.mpaxos.com/cluster/projects/project-a", expect.anything());
    expect(screen.getByRole("button", { name: "Manage mako-soumojit" }).getAttribute("aria-current")).toBe("true");
  });

  it("starts the owner transfer empty and confirms it before sending", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    const input = await screen.findByLabelText("New owner user ID") as HTMLInputElement;
    expect(input.value).toBe("");
    const transfer = screen.getByRole("button", { name: "Transfer" }) as HTMLButtonElement;
    expect(transfer.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "owner-1" } });
    expect(transfer.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "user-9" } });
    fireEvent.click(transfer);
    confirmInDialog("Transfer");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/owner",
      expect.objectContaining({ method: "PATCH", body: JSON.stringify({ user_id: "user-9" }) }),
    ));
    expect(await screen.findByText("Owner changed.")).toBeTruthy();
  });

  it("adds a member who inherits the cluster policy", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Sharing");

    fireEvent.change(await screen.findByLabelText("Member account email"), { target: { value: "new-member@example.com" } });
    fireEvent.click(screen.getByRole("checkbox", { name: "build-host" }));
    const select = screen.getByLabelText("Default AI agent for new member projects") as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("terminal:codex:official:default"));
    fireEvent.change(select, { target: { value: "" } });
    expect(select.value).toBe("");
    fireEvent.click(screen.getByRole("button", { name: "Add member" }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/members",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          email: "new-member@example.com",
          allowed_machine_ids: ["machine-1"],
          default_launch_profile: null,
        }),
      }),
    ));
    expect(await screen.findByText("Member added.")).toBeTruthy();
  });

  it("keeps an unsaved cluster policy change across an unrelated re-render", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Policies");

    const checkbox = await screen.findByRole("checkbox", { name: /Codex Terminal/ }) as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
    fireEvent.click(checkbox);
    expect(checkbox.checked).toBe(false);
    act(() => { useStore.setState({ serverVersion: "26.09.1" }); });
    expect((screen.getByRole("checkbox", { name: /Codex Terminal/ }) as HTMLInputElement).checked).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "Save cluster policy" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/policy/default",
      expect.objectContaining({
        method: "PATCH",
        body: JSON.stringify({ team_available: true, allowed_launch_profiles: [] }),
      }),
    ));
  });

  it("does not load the cluster autonomous worker policy until it is opened", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Policies");

    const toggle = await screen.findByRole("button", { name: /Cluster autonomous worker policy/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(fetchMock.mock.calls.some(([url]) => String(url).endsWith("/cluster/job-policy"))).toBe(false);
    fireEvent.click(toggle);
    await waitFor(() => expect(fetchMock.mock.calls.some(([url]) => String(url).endsWith("/cluster/job-policy"))).toBe(true));
  });

  it("asks before inheriting the deployment policy", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Policies");

    fireEvent.click(await screen.findByRole("button", { name: "Inherit from above" }));
    expect(fetchMock.mock.calls.some(([, init]) => (init as RequestInit | undefined)?.method === "PATCH")).toBe(false);
    confirmInDialog("Inherit from above");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/policy/default",
      expect.objectContaining({ body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }) }),
    ));
  });

  it("shows both limit windows for each provider in Resource use", async () => {
    seedMachines();
    act(() => {
      useStore.setState({
        usageLimits: new Map([["cli-1", {
          claude: {
            fiveHour: { utilization: 0.42, resetsAt: "2099-01-01T10:00:00Z" },
            sevenDay: { utilization: 0.17, resetsAt: "2099-01-05T10:00:00Z" },
            fetchedAt: "2026-06-17T05:00:00Z",
          },
        }]]),
      });
    });
    render(<MachinesPage />);
    openView("Usage");

    const section = (await screen.findByRole("heading", { name: "Resource use" })).closest("section") as HTMLElement;
    expect(within(section).getByText("Claude")).toBeTruthy();
    expect(within(section).getByText("Weekly")).toBeTruthy();
    expect(within(section).getByText("5-Hour")).toBeTruthy();
    expect(within(section).getByText(/^42%/)).toBeTruthy();
    expect(within(section).getByText(/^17%/)).toBeTruthy();
  });

  it("says a list could not be loaded instead of showing it empty", async () => {
    seedMachines();
    installClusterApi();
    const answer = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
      if (String(input).endsWith("/cluster/members")) return apiResponse({ message: "members are down" }, false, 500);
      return answer(input, init);
    });
    render(<MachinesPage />);
    openView("Sharing");

    expect(await screen.findByText("Could not load members.")).toBeTruthy();
    expect(screen.queryByText("No members have access to this cluster.")).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("members are down");
    openView("Usage");
    expect(screen.getByText("Could not load usage.")).toBeTruthy();
    openView("Activity");
    expect(screen.getByText("Could not load activity.")).toBeTruthy();
  });

  it("moves focus to the chosen project's panel and back to its row", async () => {
    seedMachines();
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    // The test page is narrow: the panel takes the list's place.
    expect(document.activeElement).toBe(screen.getByRole("region", { name: "Selected project" }));
    fireEvent.click(await screen.findByRole("button", { name: "All projects" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Manage mako-soumojit" }));
  });

  it("moves focus into the member editor and back to Edit", async () => {
    fetchMock.mockImplementation(async (input: string | URL) => {
      const url = String(input);
      if (url.endsWith("/cluster/members")) return apiResponse([{
        user_id: "member-1",
        user_email: "member@example.com",
        status: "active",
        allowed_machine_ids: null,
        default_launch_profile: null,
      }]);
      if (url.includes("/cluster/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      if (url.endsWith("/cluster/launch-profiles")) return apiResponse([]);
      if (url.includes("/cluster/audit")) return apiResponse({ items: [], limit: 25, offset: 0 });
      return apiResponse({ success: true });
    });
    seedMachines();
    render(<MachinesPage />);
    openView("Sharing");

    fireEvent.click(await screen.findByRole("button", { name: "Edit access for member@example.com" }));
    expect(document.activeElement).toBe(screen.getByRole("checkbox", { name: "All current and future machines" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Edit access for member@example.com" }));
  });

  it("disables Open and Close while the page is not connected to the server", () => {
    seedMachines();
    act(() => { useStore.setState({ connected: false }); });
    render(<MachinesPage />);

    const open = screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" }) as HTMLButtonElement;
    expect(open.disabled).toBe(true);
    expect(open.title).toBe("Not connected to the server");
    expect((screen.getByRole("button", { name: "Close Active API (listed on build-host)" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("says an Open the socket refused was not sent, does not wait for it, and does not send it later", () => {
    const actions = seedMachines();
    actions.openProject.mockReturnValue(false);
    render(<MachinesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" }));
    confirmInDialog("Open");
    const open = screen.getByRole("button", { name: "Open Stopped API (listed on build-host)" }) as HTMLButtonElement;
    expect(open.textContent).not.toContain("Opening…");
    expect(open.disabled).toBe(false);
    expect(screen.getByText(NOTHING_SENT_TEXT)).toBeTruthy();
    expect(NOTHING_SENT_TEXT).toBe("Not connected to the server. Nothing was sent.");

    // A new machine list does not send it for the user.
    act(() => { useStore.setState({ machines: [machineEntry()] }); });
    expect(actions.openProject).toHaveBeenCalledTimes(1);
  });
});

describe("Projects view: open state and deletion", () => {
  /** The cluster's one project, `project-a`, as a machine lists it. */
  function hosting(project: Partial<MachineWithProjects["projects"][number]>): MachineWithProjects {
    return {
      machine: { machineId: "machine-1", hostname: "build-host", os: "linux", arch: "x64" },
      projects: [{ projectId: "project-a", name: "mako-soumojit", path: "/srv/mako", isRunning: false, ...project }],
      clusterAccess: "owner",
      daemonConnected: true,
    };
  }

  function capable() {
    act(() => { useStore.setState({ negotiatedCapabilities: new Set([PROJECT_OPEN_STATE_CAPABILITY]) }); });
  }

  it("shows Open or Closed in the list and the panel, from the machine list", async () => {
    seedMachines([hosting({ openState: "closed", part: "not_started" })]);
    capable();
    render(<MachinesPage />);
    openView("Projects");

    const row = await screen.findByRole("button", { name: "Manage mako-soumojit" });
    expect(within(row).getByTestId("project-open-state").textContent).toBe("Closed");
    fireEvent.click(row);
    const panel = screen.getByRole("region", { name: "Selected project" });
    expect((await within(panel).findByTestId("project-open-state")).textContent).toBe("Closed");

    act(() => { useStore.setState({ machines: [hosting({ isRunning: true, openState: "open", part: "running" })] }); });
    expect(within(row).getByTestId("project-open-state").textContent).toBe("Open");
    expect(within(panel).getByTestId("project-open-state").textContent).toBe("Open");
  });

  it("takes the state from the session list when no machine lists the project", async () => {
    seedMachines();
    capable();
    act(() => {
      useStore.setState({ sessions: [{ id: "project-a", projectId: "project-a", status: "active", isActive: false, openState: "open" }] });
    });
    render(<MachinesPage />);
    openView("Projects");

    const row = await screen.findByRole("button", { name: "Manage mako-soumojit" });
    expect(within(row).getByTestId("project-open-state").textContent).toBe("Open");
  });

  it("shows no state for a project nothing is known about", async () => {
    seedMachines();
    capable();
    render(<MachinesPage />);
    openView("Projects");

    const row = await screen.findByRole("button", { name: "Manage mako-soumojit" });
    expect(within(row).queryByTestId("project-open-state")).toBeNull();
  });

  it("offers Delete project… to the project's owner, naming the machines stopped first", async () => {
    seedMachines([hosting({ isRunning: true, openState: "open", part: "running" })]);
    capable();
    act(() => { useStore.setState({ userId: "owner-1" }); });
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete project…" }));
    const dialog = screen.getByRole("dialog", { name: "Delete project" });
    expect(within(dialog).getByRole("heading", { name: "Delete mako-soumojit?" })).toBeTruthy();
    expect(dialog.textContent).toContain("Agents running in the project are stopped first, on build-host.");
    // Opening the dialog sends nothing, and it needs the name typed.
    expect((within(dialog).getByRole("button", { name: "Delete project" }) as HTMLButtonElement).disabled).toBe(true);
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes("/delete"))).toBe(false);

    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes("/delete"))).toBe(false);
  });

  it("offers no Delete for a project another account owns, and stopping its runtime opens none", async () => {
    seedMachines([hosting({ isRunning: true, openState: "open", part: "running" })]);
    capable();
    act(() => { useStore.setState({ userId: "me" }); });
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    expect(await screen.findByRole("button", { name: "Stop runtime" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Delete project/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Stop runtime" }));
    expect(screen.queryByRole("dialog", { name: "Delete project" })).toBeNull();
    // On a server that records open states, stopping the runtime closes
    // the project, and the confirmation says so before anything is sent.
    expect(screen.getByRole("dialog").textContent).toContain(STOP_RUNTIME_CLOSES_PROJECT);
    confirmInDialog("Stop runtime");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/stop-runtime",
      expect.objectContaining({ method: "POST" }),
    ));
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes("/delete"))).toBe(false);
  });

  it("says a suspension also closes the project, before anything is sent", async () => {
    seedMachines([hosting({ isRunning: true, openState: "open", part: "running" })]);
    capable();
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Suspend project" }));
    expect(screen.getByRole("dialog").textContent).toContain(SUSPEND_CLOSES_PROJECT);
    expect(fetchMock).not.toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/lifecycle",
      expect.anything(),
    );
  });

  it("leaves the project's panel and loads the list again once a deletion is done", async () => {
    let deleted = false;
    const fallback = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/projects/project-a/delete") && init?.method === "POST") {
        deleted = true;
        return { ok: true, status: 202, json: vi.fn().mockResolvedValue({}) };
      }
      if (url.endsWith("/projects/project-a/deletion")) return apiResponse({ message: "gone" }, false, 404);
      if (deleted && url.includes("/cluster/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      return fallback(input, init);
    });
    const actions = seedMachines([hosting({ openState: "closed", part: "not_started" })]);
    capable();
    act(() => { useStore.setState({ userId: "owner-1", forgetProject: vi.fn(), listSessions: vi.fn() }); });
    render(<MachinesPage />);
    openView("Projects");

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete project…" }));
    const dialog = screen.getByRole("dialog", { name: "Delete project" });
    fireEvent.change(within(dialog).getByLabelText("Project name"), { target: { value: "mako-soumojit" } });
    actions.listMachines.mockClear();
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete project" }));

    // The dialog asks the server whether the deletion is done before it says so.
    expect(await screen.findByText("No projects are hosted in your cluster yet.", {}, { timeout: 5000 })).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(actions.listMachines).toHaveBeenCalled();
    expect(fetchMock.mock.calls.filter(([url]) => String(url).endsWith("/projects/project-a/delete"))).toHaveLength(1);
    expect(new URL(window.location.href).searchParams.has("selected")).toBe(false);
  });

  /** The cluster API of a deletion of `project-a` that the test ends by
   *  hand: accepted at once, done when `server.gone` is set. Until then the
   *  status endpoint answers `server.status`. `project-b` is a second
   *  project. */
  function deletionApi(status: number, withSecondProject = false) {
    const server = { gone: false, status };
    const fallback = fetchMock.getMockImplementation()!;
    const second = {
      id: "project-b",
      project_name: "other",
      hostname: "build-host",
      owner_user_id: "owner-1",
      owner_email: "owner@example.com",
      lifecycle_status: "active",
      member_count: 0,
      active_session_count: 0,
      hosting_emails: ["me@example.com"],
      connected: true,
      effective_policy: policy,
    };
    fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/projects/project-a/delete") && init?.method === "POST") {
        return { ok: true, status: 202, json: vi.fn().mockResolvedValue({}) };
      }
      if (url.endsWith("/projects/project-a/deletion")) {
        return server.gone
          ? apiResponse({ message: "gone" }, false, 404)
          : apiResponse({ error: "Deletion status is unavailable" }, server.status < 300, server.status);
      }
      if (url.includes("/cluster/projects?")) {
        const first = (await (await fallback(input, init)).json()).items as unknown[];
        return apiResponse({
          items: [...(server.gone ? [] : first), ...(withSecondProject ? [second] : [])],
          limit: 200,
          offset: 0,
        });
      }
      if (url.match(/\/cluster\/projects\/project-b$/)) {
        return apiResponse({ project: second, members: [], policy });
      }
      return fallback(input, init);
    });
    return server;
  }

  async function startDeletion() {
    const actions = seedMachines([hosting({ openState: "closed", part: "not_started" })]);
    capable();
    act(() => { useStore.setState({ userId: "owner-1", forgetProject: vi.fn(), listSessions: vi.fn() }); });
    const { unmount } = render(<MachinesPage />);
    openView("Projects");
    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Delete project…" }));
    const dialog = screen.getByRole("dialog", { name: "Delete project" });
    fireEvent.change(within(dialog).getByLabelText("Project name"), { target: { value: "mako-soumojit" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete project" }));
    return { actions, dialog, unmount };
  }

  const listLoads = () => fetchMock.mock.calls.filter(([url]) => String(url).includes("/cluster/projects?")).length;
  const statusAsks = () => fetchMock.mock.calls.filter(([url]) => String(url).endsWith("/projects/project-a/deletion")).length;
  const deleteRequests = () => fetchMock.mock.calls.filter(([url]) => String(url).endsWith("/projects/project-a/delete")).length;
  const address = () => `${window.location.pathname}${window.location.search}`;
  const pause = () => new Promise((done) => setTimeout(done, 50));

  /** The server's one report that a project is deleted, as the socket
   *  delivers it to the store. */
  function reportDeleted(projectId: string) {
    act(() => {
      useStore.setState({ forgetProject: initialStore.forgetProject });
      handleServerMessage(
        { type: "project_access_changed", project_id: projectId, change: "deleted" },
        useStore.setState,
        useStore.getState,
      );
    });
  }

  /** A deletion the server accepted and whose dialog gave up and was
   *  closed: the list was loaded once more and still has the project. */
  async function giveUpAndClose() {
    const server = deletionApi(500);
    const started = await startDeletion();
    await within(started.dialog).findByRole("alert", {}, { timeout: 5000 });
    const loads = listLoads();
    fireEvent.click(within(started.dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(listLoads()).toBe(loads + 1));
    await pause();
    expect(screen.getByRole("button", { name: "Delete project…" })).toBeTruthy();
    return { server, ...started };
  }

  // The deletion goes on without its dialog. The project list is loaded
  // over HTTP only when asked, so the page must hear of the end.
  it("drops the project when its deletion ends after the dialog was closed", async () => {
    const server = deletionApi(200);
    const { actions, dialog } = await startDeletion();
    expect((await within(dialog).findByRole("status")).textContent).toContain("Deleting…");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    // Still deleting: the project and its panel are as they were.
    expect(screen.getByRole("button", { name: "Delete project…" })).toBeTruthy();
    actions.listMachines.mockClear();

    server.gone = true;
    expect(await screen.findByText("No projects are hosted in your cluster yet.", {}, { timeout: 5000 })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull();
    expect(actions.listMachines).toHaveBeenCalled();
    expect(new URL(window.location.href).searchParams.has("selected")).toBe(false);
    expect(fetchMock.mock.calls.filter(([url]) => String(url).endsWith("/projects/project-a/delete"))).toHaveLength(1);
  });

  it("keeps another project selected when the deletion ends after the dialog was closed", async () => {
    const server = deletionApi(200, true);
    const { dialog } = await startDeletion();
    expect((await within(dialog).findByRole("status")).textContent).toContain("Deleting…");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    fireEvent.click(screen.getByRole("button", { name: "Manage other" }));
    expect(await screen.findByRole("heading", { name: "other" })).toBeTruthy();

    server.gone = true;
    await waitFor(() => expect(screen.queryByRole("button", { name: "Manage mako-soumojit" })).toBeNull(), { timeout: 5000 });
    // The deleted project left the list; the one now selected keeps its panel.
    expect(screen.getByRole("heading", { name: "other" })).toBeTruthy();
    expect(new URL(window.location.href).searchParams.get("selected")).toBe("project-b");
  });

  it("loads the project list again when the dialog closes without knowing how the deletion went", async () => {
    const server = deletionApi(500);
    const { dialog } = await startDeletion();
    // The wait ended without an answer; the deletion goes on.
    expect((await within(dialog).findByRole("alert", {}, { timeout: 5000 })).textContent)
      .toBe("Deletion status is unavailable");
    const loadsBefore = listLoads();
    // Nothing is loaded while the dialog is open.
    await new Promise((done) => setTimeout(done, 50));
    expect(listLoads()).toBe(loadsBefore);

    server.gone = true;
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(await screen.findByText("No projects are hosted in your cluster yet.", {}, { timeout: 5000 })).toBeTruthy();
    expect(listLoads()).toBe(loadsBefore + 1);
    // The project is gone from the fresh list, so its panel goes too.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull());
    expect(fetchMock.mock.calls.filter(([url]) => String(url).endsWith("/projects/project-a/delete"))).toHaveLength(1);
  });

  it("keeps the project's panel when the list still has it after such a close", async () => {
    deletionApi(500);
    const { dialog } = await startDeletion();
    await within(dialog).findByRole("alert", {}, { timeout: 5000 });
    const loadsBefore = listLoads();
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(listLoads()).toBe(loadsBefore + 1));
    await new Promise((done) => setTimeout(done, 50));
    expect(screen.getByRole("button", { name: "Manage mako-soumojit" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Delete project…" })).toBeTruthy();
  });

  // The address names the selected project only while the Projects view
  // shows. A deletion that ends later must not put that view back into it.
  it("leaves the address alone when a deletion ends while another view shows", async () => {
    const server = deletionApi(200);
    const { actions, dialog } = await startDeletion();
    expect((await within(dialog).findByRole("status")).textContent).toContain("Deleting…");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    openView("Usage");
    expect(address()).toBe("/machines?view=usage");
    actions.listMachines.mockClear();
    const loads = listLoads();

    server.gone = true;
    await waitFor(() => expect(listLoads()).toBe(loads + 1), { timeout: 5000 });
    await pause();
    expect(actions.listMachines).toHaveBeenCalled();
    expect(address()).toBe("/machines?view=usage");
    // The project lost its panel all the same.
    openView("Projects");
    expect(await screen.findByText("No projects are hosted in your cluster yet.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull();
    expect(address()).toBe("/machines?view=projects");
  });

  it("does nothing when a deletion ends after the page is gone", async () => {
    const server = deletionApi(200);
    const { actions, dialog, unmount } = await startDeletion();
    expect((await within(dialog).findByRole("status")).textContent).toContain("Deleting…");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    const forgetProject = useStore.getState().forgetProject as unknown as Mock;
    unmount();
    // Another page is open now, with an address of its own.
    window.history.replaceState({}, "", "/?probe=1&view=usage&selected=other");
    actions.listMachines.mockClear();
    const callsBefore = fetchMock.mock.calls.length;

    server.gone = true;
    await waitFor(() => expect(forgetProject).toHaveBeenCalledWith("project-a"), { timeout: 5000 });
    await pause();
    expect(address()).toBe("/?probe=1&view=usage&selected=other");
    expect(actions.listMachines).not.toHaveBeenCalled();
    // Only the wait for the deletion asked anything.
    expect(fetchMock.mock.calls.slice(callsBefore).map(([url]) => String(url))
      .filter((url) => !url.endsWith("/projects/project-a/deletion"))).toEqual([]);
  });

  it("does nothing when the wait for a deletion gives up after the page is gone", async () => {
    const server = deletionApi(200);
    const { actions, dialog, unmount } = await startDeletion();
    expect((await within(dialog).findByRole("status")).textContent).toContain("Deleting…");
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    unmount();
    window.history.replaceState({}, "", "/?probe=1");
    actions.listMachines.mockClear();
    const callsBefore = fetchMock.mock.calls.length;
    const asks = statusAsks();

    server.status = 500;
    await waitFor(() => expect(statusAsks()).toBeGreaterThan(asks), { timeout: 5000 });
    await pause();
    expect(address()).toBe("/?probe=1");
    expect(actions.listMachines).not.toHaveBeenCalled();
    expect(fetchMock.mock.calls.slice(callsBefore).map(([url]) => String(url))
      .filter((url) => !url.endsWith("/projects/project-a/deletion"))).toEqual([]);
  });

  // A list load can be in flight when the owner leaves the page. When it
  // comes back without the project, the address is another page's.
  it("leaves the address alone when a list load ends after the page is gone", async () => {
    const { server, unmount } = await giveUpAndClose();
    const inner = fetchMock.getMockImplementation()!;
    let release = () => {};
    let held = 0;
    const gate = new Promise<void>((done) => { release = done; });
    fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
      if (String(input).includes("/cluster/projects?")) { held += 1; await gate; }
      return inner(input, init);
    });
    server.gone = true;
    reportDeleted("project-a");
    await waitFor(() => expect(held).toBe(1));
    unmount();
    window.history.replaceState({}, "", "/?probe=1&view=usage&selected=other");
    release();
    await pause();
    await pause();
    expect(address()).toBe("/?probe=1&view=usage&selected=other");
  });

  // After the dialog gave up nothing asks about the deletion any more. The
  // server reports its end once, and the store then forgets the project.
  it("drops the project when the server reports it deleted after the dialog gave up", async () => {
    const { server } = await giveUpAndClose();
    const loads = listLoads();
    const asks = statusAsks();

    // Another project's end says nothing about this list.
    reportDeleted("project-z");
    expect(listLoads()).toBe(loads);

    server.gone = true;
    reportDeleted("project-a");
    expect(listLoads()).toBe(loads + 1);
    expect(await screen.findByText("No projects are hosted in your cluster yet.")).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull());
    expect(address()).toBe("/machines?view=projects");
    // The delete request went out once, and nobody asked about it again.
    expect(deleteRequests()).toBe(1);
    expect(statusAsks()).toBe(asks);

    // The wait is over: a later report loads nothing.
    reportDeleted("project-a");
    expect(listLoads()).toBe(loads + 1);
  });

  it("keeps the project when the store forgets it and the list still has it", async () => {
    await giveUpAndClose();
    const loads = listLoads();

    reportDeleted("project-a");
    await waitFor(() => expect(listLoads()).toBe(loads + 1));
    await pause();
    expect(screen.getByRole("button", { name: "Manage mako-soumojit" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Delete project…" })).toBeTruthy();
    expect(new URL(window.location.href).searchParams.get("selected")).toBe("project-a");
  });

  it("drops the project when a later load of the list no longer has it", async () => {
    const { server } = await giveUpAndClose();

    server.gone = true;
    fireEvent.click(screen.getByRole("button", { name: "Refresh machines" }));
    expect(await screen.findByText("No projects are hosted in your cluster yet.")).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull());
    expect(new URL(window.location.href).searchParams.has("selected")).toBe(false);
    expect(deleteRequests()).toBe(1);
  });

  // The server's report is sent once, to the sockets connected then.
  it("loads the list again when the page reconnects while a deletion is unconfirmed", async () => {
    const server = deletionApi(500);
    const { dialog } = await startDeletion();
    await within(dialog).findByRole("alert", {}, { timeout: 5000 });
    const reconnect = () => {
      act(() => { useStore.setState({ connected: false }); });
      act(() => { useStore.setState({ connected: true }); });
    };
    // A reconnect loads no list while the page waits for no deletion.
    const loads = listLoads();
    reconnect();
    expect(listLoads()).toBe(loads);

    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(listLoads()).toBe(loads + 1));
    await pause();
    expect(screen.getByRole("button", { name: "Delete project…" })).toBeTruthy();

    server.gone = true;
    reconnect();
    expect(listLoads()).toBe(loads + 2);
    expect(await screen.findByText("No projects are hosted in your cluster yet.")).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull());
    expect(deleteRequests()).toBe(1);
  });

  it("keeps waiting while another cluster's list shows, which never had the project", async () => {
    const server = deletionApi(500);
    const own = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/cluster/contexts")) return apiResponse([
        { owner_user_id: "owner-1", owner_email: "owner@example.com", access: "owner" },
        { owner_user_id: "host", owner_email: "host@example.com", access: "member" },
      ]);
      if (url.includes("/cluster/contexts/host/projects?")) return apiResponse({ items: [], limit: 200, offset: 0 });
      if (url.endsWith("/cluster/contexts/host/policy/default")) return apiResponse({ cluster: null, deployment: policy });
      return own(input, init);
    });
    const { dialog } = await startDeletion();
    await within(dialog).findByRole("alert", {}, { timeout: 5000 });
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await pause();

    const hostLoads = () => fetchMock.mock.calls.filter(([url]) => String(url).includes("/cluster/contexts/host/projects?")).length;
    fireEvent.change(screen.getByLabelText("Selected cluster"), { target: { value: "host" } });
    await waitFor(() => expect(hostLoads()).toBe(1));
    await pause();
    expect(screen.queryByRole("button", { name: "Manage mako-soumojit" })).toBeNull();

    // Back in its own cluster the project is still being deleted.
    fireEvent.change(screen.getByLabelText("Selected cluster"), { target: { value: "owner-1" } });
    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    expect(await screen.findByRole("button", { name: "Delete project…" })).toBeTruthy();
    await pause();

    server.gone = true;
    reportDeleted("project-a");
    await waitFor(() => expect(screen.queryByRole("button", { name: "Manage mako-soumojit" })).toBeNull());
    await waitFor(() => expect(screen.queryByRole("button", { name: "Delete project…" })).toBeNull());
    expect(deleteRequests()).toBe(1);
  });
});
