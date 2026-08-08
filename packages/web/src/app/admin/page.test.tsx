import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AdminPage from "./page";

const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
const store = vi.hoisted(() => ({
  state: {
    token: null as string | null,
    clusterRole: null as "admin" | "user" | null,
  },
}));

vi.mock("next/navigation", () => ({ useRouter: () => router }));
vi.mock("@/lib/store", () => ({ useStore: () => store.state }));

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

const policy = {
  team_available: true,
  allowed_launch_profiles: ["agent:codex:official:default"],
  version: 6,
  project_suspended: false,
};

function response(body: unknown, ok = true, status = ok ? 200 : 500) {
  return { ok, status, json: vi.fn().mockResolvedValue(body) };
}

function installApiFixtures() {
  fetchMock.mockImplementation(async (input: string | URL, init?: RequestInit) => {
    const url = String(input);
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
        { key: "agent:codex:official:default", label: "Codex / Official" },
        { key: "agent:claude:glm:glm-5.1", label: "Legacy GLM" },
      ]);
    }
    if (url.endsWith("/admin/policy/default")) return response(policy);
    if (url.includes("/admin/users/invitations")) {
      return response({ registration_url: "http://apas.mpaxos.com/register?invitation=invite-1" });
    }
    if (url.match(/\/admin\/users\/user-1$/) && init?.method === "PATCH") {
      return response({ id: "user-1", email: "member@example.com", cluster_role: "admin", account_status: "active" });
    }
    if (url.includes("/admin/users?")) {
      return response({
        items: [{ id: "user-1", email: "member@example.com", cluster_role: "user", account_status: "active" }],
        limit: 200,
        offset: 0,
      });
    }
    if (url.includes("/admin/projects?")) {
      return response({
        items: [{
          id: "project-a",
          owner_user_id: "owner-1",
          owner_email: "owner@example.com",
          lifecycle_status: "active",
          member_count: 1,
          active_session_count: 1,
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
          owner_user_id: "owner-1",
          owner_email: "owner@example.com",
          lifecycle_status: "active",
          member_count: 1,
          active_session_count: 1,
        },
        members: [{ user_id: "user-1", email: "member@example.com" }],
        policy,
      });
    }
    if (url.includes("/admin/audit?")) {
      return response({
        items: [{
          id: 9,
          actor_user_id: "admin-1",
          action: "project.policy_updated",
          target_type: "project",
          target_id: "project-a",
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

describe("AdminPage", () => {
  beforeEach(() => {
    router.push.mockReset();
    router.replace.mockReset();
    fetchMock.mockReset();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    store.state = { token: null, clusterRole: null };
  });

  afterEach(() => {
    vi.restoreAllMocks();
    globalThis.fetch = originalFetch;
  });

  it("redirects unauthenticated visitors without loading control-plane data", async () => {
    render(<AdminPage />);
    await waitFor(() => expect(router.replace).toHaveBeenCalledWith("/login?redirect=/admin"));
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("uses the persisted cluster role instead of a hard-coded user id", async () => {
    store.state = { token: "user-token", clusterRole: "user" };
    render(<AdminPage />);
    await waitFor(() => expect(router.replace).toHaveBeenCalledWith("/"));
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("loads overview statistics and editable cluster defaults for admins", async () => {
    store.state = { token: "admin-token", clusterRole: "admin" };
    installApiFixtures();
    render(<AdminPage />);

    expect(await screen.findByText("Cluster Administration")).toBeTruthy();
    expect(await screen.findByText("Cluster users")).toBeTruthy();
    expect(screen.getByText("Cluster default policy")).toBeTruthy();
    expect(screen.getByText("Codex / Official")).toBeTruthy();
    expect(screen.queryByText("Legacy GLM")).toBeNull();
    expect(fetchMock).toHaveBeenCalledWith(
      "http://apas.mpaxos.com/admin/stats",
      expect.objectContaining({ headers: expect.objectContaining({ Authorization: "Bearer admin-token" }) }),
    );
  });

  it("supports account invitation and role changes from the Users view", async () => {
    store.state = { token: "admin-token", clusterRole: "admin" };
    installApiFixtures();
    render(<AdminPage />);
    await screen.findByText("Cluster Administration");
    fireEvent.click(screen.getByRole("button", { name: "users" }));

    expect(await screen.findByText("member@example.com")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Invitation email"), { target: { value: "new@example.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Create invitation" }));
    expect(await screen.findByText(/register\?invitation=invite-1/)).toBeTruthy();

    fireEvent.change(screen.getByLabelText("Role for member@example.com"), { target: { value: "admin" } });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "http://apas.mpaxos.com/admin/users/user-1",
      expect.objectContaining({ method: "PATCH", body: JSON.stringify({ cluster_role: "admin" }) }),
    ));
  });

  it("loads metadata-only project administration and can clear an override", async () => {
    store.state = { token: "admin-token", clusterRole: "admin" };
    installApiFixtures();
    render(<AdminPage />);
    await screen.findByText("Cluster Administration");
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    fireEvent.click(await screen.findByRole("button", { name: /project-a/ }));

    expect(await screen.findByText("Project control")).toBeTruthy();
    expect(screen.getByText("member@example.com")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Use cluster defaults" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "http://apas.mpaxos.com/admin/projects/project-a/policy",
      expect.objectContaining({
        method: "PATCH",
        body: JSON.stringify({ team_available: null, allowed_launch_profiles: null }),
      }),
    ));
    expect(fetchMock.mock.calls.some(([url]) => /\/messages|\/terminal|\/diff|\/files/.test(String(url)))).toBe(false);
  });

  it("renders paginated audit metadata", async () => {
    store.state = { token: "admin-token", clusterRole: "admin" };
    installApiFixtures();
    render(<AdminPage />);
    await screen.findByText("Cluster Administration");
    fireEvent.click(screen.getByRole("button", { name: "audit" }));

    expect(await screen.findByText("project.policy_updated")).toBeTruthy();
    expect(screen.getByText("project: project-a")).toBeTruthy();
    expect(screen.getByText("admin-1")).toBeTruthy();
  });

  it("returns to the cluster workspace from Back", async () => {
    store.state = { token: "admin-token", clusterRole: "admin" };
    installApiFixtures();
    render(<AdminPage />);
    await screen.findByText("Cluster Administration");
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(router.push).toHaveBeenCalledWith("/");
  });
});
