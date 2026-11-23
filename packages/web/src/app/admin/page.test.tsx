import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SystemAdminPage from "./page";
import { STOP_RUNTIME_CLOSES_PROJECT, SUSPEND_CLOSES_PROJECT } from "@/lib/projectOpenState";

const TOKEN_KEY = "apas_system_admin_token";

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

const policy = {
  team_available: true,
  allowed_launch_profiles: ["terminal:codex:official:default"],
  version: 6,
  project_suspended: false,
};

function response(body: unknown, ok = true, status = ok ? 200 : 500) {
  return { ok, status, json: vi.fn().mockResolvedValue(body) };
}

function installApiFixtures({ bootstrapPending = false, loadFails = false } = {}) {
  fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith("/admin/auth/login")) {
      return response({ token: "system-token", username: "admin", bootstrap_pending: bootstrapPending });
    }
    if (url.endsWith("/admin/auth/me")) {
      return response({ username: "admin", bootstrap_pending: bootstrapPending });
    }
    if (url.endsWith("/admin/auth/password")) {
      return response({ token: "rotated-token", username: "admin", bootstrap_pending: false });
    }
    if (url.endsWith("/admin/load")) {
      if (loadFails) return response({}, false);
      return response({
        sampled_at: "2026-11-22T12:00:00Z",
        activity: { online_users: 2, viewer_connections: 5, connected_sessions: 6, working_sessions: 3, connected_daemons: 4 },
        host_cpu_percent: null,
        host_memory: { used_bytes: 2 ** 30, total_bytes: 4 * 2 ** 30 },
      });
    }
    if (url.endsWith("/admin/stats")) {
      return response({
        total_users: 4,
        recent_users_7d: 1,
        total_sessions: 8,
        active_sessions_24h: 2,
        total_cli_clients: 3,
        online_cli_clients: 2,
        total_shares: 0,
      });
    }
    if (url.endsWith("/admin/launch-profiles")) {
      return response([
        { key: "terminal:codex:official:default", label: "Codex Terminal" },
        { key: "agent:claude:glm:glm-5.1", label: "Legacy GLM" },
      ]);
    }
    if (url.endsWith("/admin/policy/default")) return response(policy);
    if (url.endsWith("/admin/clusters")) {
      return response({
        items: [{
          user_id: "host-1",
          email: "host@example.com",
          account_status: "active",
          hosted_project_count: 2,
          owned_project_count: 1,
          active_session_count: 3,
          last_activity: "2026-08-14T10:00:00Z",
        }],
        limit: 1,
        offset: 0,
      });
    }
    if (url.includes("/admin/users/invitations")) {
      return response({ registration_url: "https://apas.mpaxos.com/register?invitation=invite-1" });
    }
    if (url.match(/\/admin\/users\/user-1$/) && init?.method === "PATCH") {
      return response({ id: "user-1", email: "member@example.com", account_status: "suspended" });
    }
    if (url.includes("/admin/users?")) {
      return response({
        items: [{ id: "user-1", email: "member@example.com", account_status: "active" }],
        limit: 200,
        offset: 0,
      });
    }
    if (url.includes("/admin/projects?")) {
      return response({
        items: [{
          id: "project-a",
          project_name: "mako-soumojit",
          hostname: "zoo-002",
          owner_user_id: "owner-1",
          owner_email: "owner@example.com",
          lifecycle_status: "active",
          member_count: 1,
          active_session_count: 1,
          hosting_emails: ["host@example.com"],
          connected: true,
          effective_policy: policy,
        }],
        limit: 200,
        offset: 0,
      });
    }
    if (url.match(/\/admin\/projects\/project-a$/)) {
      return response({
        project: {
          id: "project-a",
          project_name: "mako-soumojit",
          hostname: "zoo-002",
          owner_user_id: "owner-1",
          owner_email: "owner@example.com",
          lifecycle_status: "active",
          member_count: 1,
          active_session_count: 1,
          hosting_emails: ["host@example.com"],
        },
        members: [{ user_id: "user-1", email: "member@example.com" }],
        policy,
      });
    }
    if (url.includes("/admin/audit?")) {
      return response({
        items: [{
          id: 9,
          actor_kind: "system_admin",
          actor_user_id: "system-admin",
          action: "project.policy_updated",
          target_type: "project",
          target_id: "project-a",
          cluster_user_id: null,
          details: "{\"version\":6}",
          created_at: "2026-08-07T10:00:00Z",
        }],
        limit: 50,
        offset: 0,
      });
    }
    return response({ success: true });
  });
}

async function signIn() {
  fireEvent.change(screen.getByLabelText("System administrator username"), { target: { value: "admin" } });
  fireEvent.change(screen.getByLabelText("System administrator password"), { target: { value: "secret" } });
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  await screen.findByText("APAS System Administration");
}

describe("SystemAdminPage", () => {
  beforeEach(() => {
    fetchMock.mockReset();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    sessionStorage.clear();
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    globalThis.fetch = originalFetch;
  });

  it("presents its own sign-in and loads nothing until it succeeds", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);

    expect(await screen.findByText("APAS system administration")).toBeTruthy();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.queryByText("APAS System Administration")).toBeNull();
  });

  it("does not accept an ordinary account session", async () => {
    // An account signed in to the app has these set; the surface must still
    // demand its own credential.
    localStorage.setItem("apas_token", "user-token");
    localStorage.setItem("apas_user_email", "member@example.com");
    installApiFixtures();
    render(<SystemAdminPage />);

    expect(await screen.findByLabelText("System administrator password")).toBeTruthy();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("keeps its token out of localStorage", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();

    expect(sessionStorage.getItem(TOKEN_KEY)).toBe("system-token");
    expect(localStorage.getItem(TOKEN_KEY)).toBeNull();
    expect(Object.keys(localStorage)).not.toContain(TOKEN_KEY);
  });

  it("loads deployment statistics and the deployment default policy", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();

    expect(await screen.findByText("Accounts")).toBeTruthy();
    const load = within(screen.getByRole("region", { name: "Server load" }));
    expect(await load.findByText("5 signed-in viewer connections")).toBeTruthy();
    expect(load.getByText("6 connected sessions · 4 machine daemons online")).toBeTruthy();
    expect(load.getByText("Unavailable")).toBeTruthy();
    expect(load.getByText("25.0%")).toBeTruthy();
    expect(load.getByText("1.0 of 4.0 GiB used")).toBeTruthy();
    expect(screen.getByText("Deployment default policy")).toBeTruthy();
    expect(screen.getByText("Codex Terminal")).toBeTruthy();
    expect(screen.queryByText("Legacy GLM")).toBeNull();
    expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/admin/stats",
      expect.objectContaining({ headers: expect.objectContaining({ Authorization: "Bearer system-token" }) }),
    );
  });

  it("keeps the last sample on failure and clears the warning after a refresh", async () => {
    installApiFixtures();
    sessionStorage.setItem(TOKEN_KEY, "system-token");
    render(<SystemAdminPage />);
    await screen.findByText("5 signed-in viewer connections");

    installApiFixtures({ loadFails: true });
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText(/Refresh failed/)).toBeTruthy();
    expect(screen.getByText("5 signed-in viewer connections")).toBeTruthy();

    installApiFixtures();
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(screen.queryByText(/Refresh failed/)).toBeNull());
    const requests = fetchMock.mock.calls.filter(([path]) => String(path).endsWith("/admin/load"));
    expect(requests).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "users" }));
    expect(requests[2][1].signal.aborted).toBe(true);
  });

  it("lists every virtual cluster in the deployment", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();
    fireEvent.click(screen.getByRole("button", { name: "clusters" }));

    expect(await screen.findByText("host@example.com")).toBeTruthy();
    expect(screen.getByText("2")).toBeTruthy();
  });

  it("invites accounts and suspends them, with no cluster role to set", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();
    fireEvent.click(screen.getByRole("button", { name: "users" }));

    expect(await screen.findByText("member@example.com")).toBeTruthy();
    expect(screen.queryByLabelText("Role for member@example.com")).toBeNull();

    fireEvent.change(screen.getByLabelText("Invitation email"), { target: { value: "new@example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Create invitation" }));
    expect(await screen.findByText(/register\?invitation=invite-1/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "active" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/admin/users/user-1",
      expect.objectContaining({ method: "PATCH", body: JSON.stringify({ account_status: "suspended" }) }),
    ));
  });

  it("shows metadata-only project administration including the hosting cluster", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();
    fireEvent.click(screen.getByRole("button", { name: "projects" }));

    expect(await screen.findByText("mako-soumojit")).toBeTruthy();
    expect(screen.getByText(/Cluster host@example.com/)).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: /project-a/ }));

    expect(await screen.findByText("Project control")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Inherit from above" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/admin/projects/project-a/policy",
      expect.objectContaining({
        method: "PATCH",
        body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }),
      }),
    ));
    expect(fetchMock.mock.calls.some(([url]) => /\/messages|\/terminal|\/diff|\/files/.test(String(url)))).toBe(false);
  });

  it("asks in the page before suspending or stopping a runtime, and says both close the project", async () => {
    installApiFixtures();
    const confirm = vi.spyOn(window, "confirm");
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    fireEvent.click(await screen.findByRole("button", { name: /project-a/ }));
    await screen.findByText("Project control");
    const sent = (path: string) => fetchMock.mock.calls.filter(([url]) => String(url).endsWith(path));

    fireEvent.click(screen.getByRole("button", { name: "Stop runtime" }));
    let dialog = screen.getByRole("dialog", { name: "Stop this project's current runtime?" });
    expect(dialog.textContent).toContain(STOP_RUNTIME_CLOSES_PROJECT);
    expect(sent("/admin/projects/project-a/stop-runtime")).toHaveLength(0);
    // Declining sends nothing.
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(sent("/admin/projects/project-a/stop-runtime")).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: "Stop runtime" }));
    dialog = screen.getByRole("dialog", { name: "Stop this project's current runtime?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Stop runtime" }));
    await waitFor(() => expect(sent("/admin/projects/project-a/stop-runtime")).toHaveLength(1));

    fireEvent.click(screen.getByRole("button", { name: "Suspend project" }));
    dialog = screen.getByRole("dialog", { name: "Suspend this project?" });
    expect(dialog.textContent).toContain(SUSPEND_CLOSES_PROJECT);
    expect(sent("/admin/projects/project-a/lifecycle")).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole("button", { name: "Suspend project" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/admin/projects/project-a/lifecycle",
      expect.objectContaining({ method: "PATCH", body: JSON.stringify({ status: "suspended" }) }),
    ));
    expect(confirm).not.toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("attributes audit records to the system administrator", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();
    fireEvent.click(screen.getByRole("button", { name: "audit" }));

    expect(await screen.findByText("project.policy_updated")).toBeTruthy();
    expect(screen.getByText("system administrator")).toBeTruthy();
  });

  it("demands rotation while the bootstrap credential is unchanged", async () => {
    installApiFixtures({ bootstrapPending: true });
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();

    expect(await screen.findByText(/still the one from the server configuration/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Current password"), { target: { value: "secret" } });
    fireEvent.change(screen.getByLabelText("New password"), { target: { value: "a-much-longer-secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Rotate" }));

    await waitFor(() => expect(sessionStorage.getItem(TOKEN_KEY)).toBe("rotated-token"));
  });

  it("drops the token on sign out", async () => {
    installApiFixtures();
    render(<SystemAdminPage />);
    await screen.findByText("APAS system administration");
    await signIn();
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));

    await screen.findByLabelText("System administrator password");
    expect(sessionStorage.getItem(TOKEN_KEY)).toBeNull();
  });
});
