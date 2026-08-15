import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MachinesPage from "./page";
import { useStore, type MachineWithProjects } from "@/lib/store";

const routerPush = vi.hoisted(() => vi.fn());

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: routerPush }),
}));

const initialStore = useStore.getState();
const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

const policy = {
  team_available: false,
  allowed_launch_profiles: ["agent:codex:official:default"],
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
          allowed_launch_profiles: ["agent:codex:official:default"],
          version: 3,
          project_suspended: false,
        },
      });
    }
    if (url.endsWith("/cluster/launch-profiles")) {
      return apiResponse([
        { key: "agent:codex:official:default", label: "Codex / Official" },
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
      deepseekBackend: {
        apiBaseUrl: "https://api.deepseek.com/anthropic",
        apiKey: "sk-deepseek",
        apiKeyConfigured: true,
      },
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

function seedMachines(machines: MachineWithProjects[] = [machineEntry()]) {
  const actions = {
    connect: vi.fn(),
    listMachines: vi.fn(),
    startMachineProjectCli: vi.fn(),
    stopMachineProjectCli: vi.fn(),
    setMachineDeepseekConfig: vi.fn(),
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

beforeEach(() => {
  fetchMock.mockReset();
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  installClusterApi();
});

afterEach(() => {
  routerPush.mockReset();
  window.localStorage.clear();
  globalThis.fetch = originalFetch;
  act(() => {
    useStore.setState(initialStore, true);
  });
});

describe("MachinesPage", () => {
  it("renders machine identity, supported backend state, projects, and the empty state", () => {
    const actions = seedMachines();

    const { unmount } = render(<MachinesPage />);

    expect(actions.listMachines).toHaveBeenCalledTimes(1);
    expect(screen.getByText("build-host")).toBeTruthy();
    expect(screen.getByText(/linux\/x64/)).toBeTruthy();
    expect(screen.getByText("DeepSeek Backend (Claude Runtime)")).toBeTruthy();
    expect(screen.getByText("API key configured")).toBeTruthy();
    expect(screen.queryByText(/MiniMax/i)).toBeNull();
    expect(screen.queryByText(/GLM/i)).toBeNull();
    expect(screen.getByText("Active API")).toBeTruthy();
    expect(screen.getByText("Stopped API")).toBeTruthy();
    expect(screen.getByText(/Running.*pid 4242/)).toBeTruthy();
    expect(screen.getByText("Stopped")).toBeTruthy();
    expect(screen.getByText("My Cluster")).toBeTruthy();
    expect(screen.queryByText(/No machines reported yet/)).toBeNull();

    unmount();
    seedMachines([]);
    render(<MachinesPage />);

    expect(screen.getByText(/No machines reported yet/)).toBeTruthy();
  });

  it("wires DeepSeek Save and Clear controls to the retained store action", () => {
    const actions = seedMachines();

    render(<MachinesPage />);

    fireEvent.change(screen.getByLabelText("DeepSeek API key for build-host"), {
      target: { value: "  sk-deepseek-new  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save DeepSeek API key for build-host" }));
    expect(actions.setMachineDeepseekConfig).toHaveBeenCalledWith(
      "machine-1",
      "sk-deepseek-new",
      false,
    );
    fireEvent.click(screen.getByRole("button", { name: "Clear DeepSeek API key for build-host" }));
    expect(actions.setMachineDeepseekConfig).toHaveBeenLastCalledWith("machine-1", undefined, true);
  });

  it("wires Refresh and project Start/Stop controls to store actions", () => {
    const actions = seedMachines();

    render(<MachinesPage />);
    actions.listMachines.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "Refresh machines" }));
    expect(actions.listMachines).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Start Stopped API on build-host" }));
    expect(actions.startMachineProjectCli).toHaveBeenCalledWith("machine-1", "project-stopped");

    fireEvent.click(screen.getByRole("button", { name: "Stop Active API on build-host" }));
    expect(actions.stopMachineProjectCli).toHaveBeenCalledWith("machine-1", "project-running");
  });
});

describe("MachinesPage cluster administration", () => {
  it("lists the projects hosted in this cluster, including one another account owns", async () => {
    seedMachines();
    render(<MachinesPage />);

    expect(await screen.findByText("Projects in this cluster")).toBeTruthy();
    expect(await screen.findByText("mako-soumojit")).toBeTruthy();
    expect(screen.getByText(/Owner owner@example.com/)).toBeTruthy();
  });

  it("suspends a hosted project and stops its runtime", async () => {
    seedMachines();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<MachinesPage />);

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    fireEvent.click(await screen.findByRole("button", { name: "Suspend project" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/lifecycle",
      expect.objectContaining({ method: "PATCH", body: JSON.stringify({ status: "suspended" }) }),
    ));

    fireEvent.click(screen.getByRole("button", { name: "Stop runtime" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/stop-runtime",
      expect.objectContaining({ method: "POST" }),
    ));
  });

  it("manages members and ownership without joining the project", async () => {
    seedMachines();
    render(<MachinesPage />);

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    expect(await screen.findByText("member@example.com")).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Member user ID"), { target: { value: "user-2" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/members",
      expect.objectContaining({ method: "POST", body: JSON.stringify({ user_id: "user-2" }) }),
    ));

    fireEvent.click(screen.getByRole("button", { name: "Remove member@example.com" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/projects/project-a/members/user-1",
      expect.objectContaining({ method: "DELETE" }),
    ));
  });

  it("saves a cluster default policy and hides retired profiles", async () => {
    seedMachines();
    render(<MachinesPage />);

    expect(await screen.findByText("Cluster default policy")).toBeTruthy();
    expect(screen.getAllByText("Codex / Official").length).toBeGreaterThan(0);
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

    expect(await screen.findByText("Cluster activity")).toBeTruthy();
    expect(await screen.findByText("project.runtime_stopped")).toBeTruthy();
  });

  it("surfaces the server's refusal for a project outside this cluster", async () => {
    seedMachines();
    installClusterApi({ projectDenied: true });
    render(<MachinesPage />);

    fireEvent.click(await screen.findByRole("button", { name: "Manage mako-soumojit" }));
    expect(await screen.findByText("That project is not in your cluster")).toBeTruthy();
    expect(screen.queryByText("Members")).toBeNull();
  });
});
