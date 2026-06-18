import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AdminPage from "./page";

const ADMIN_USER_ID = "88b6016d-a8b4-400c-bdc9-f0120504a4fc";

const router = vi.hoisted(() => ({
  push: vi.fn(),
}));

const store = vi.hoisted(() => ({
  state: {
    isAuthenticated: false,
    token: null as string | null,
    userId: null as string | null,
  },
}));

vi.mock("next/navigation", () => ({
  useRouter: () => router,
}));

vi.mock("@/lib/store", () => ({
  useStore: () => store.state,
}));

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

function seedStore({
  isAuthenticated = true,
  token = "admin-token",
  userId = ADMIN_USER_ID,
}: {
  isAuthenticated?: boolean;
  token?: string | null;
  userId?: string | null;
} = {}) {
  store.state = {
    isAuthenticated,
    token,
    userId,
  };
}

function statsFixture() {
  return {
    total_users: 42,
    recent_users_7d: 3,
    total_sessions: 128,
    active_sessions_24h: 12,
    total_cli_clients: 7,
    online_cli_clients: 2,
    total_shares: 9,
    recent_users: [
      { email: "new-user@example.com", created_at: "2026-06-15T12:00:00Z" },
      { email: "unknown-date@example.com", created_at: null },
    ],
    sessions_per_day: [
      { date: "2026-06-16", count: 5 },
      { date: "2026-06-17", count: 10 },
    ],
  };
}

function jsonResponse(body: unknown, ok = true, status = ok ? 200 : 500) {
  return {
    ok,
    status,
    json: vi.fn().mockResolvedValue(body),
  };
}

describe("AdminPage", () => {
  beforeEach(() => {
    router.push.mockReset();
    fetchMock.mockReset();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    seedStore({ isAuthenticated: false, token: null, userId: null });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    globalThis.fetch = originalFetch;
    document.body.innerHTML = "";
  });

  it("redirects unauthenticated users to login without fetching stats", async () => {
    render(<AdminPage />);

    await waitFor(() => {
      expect(router.push).toHaveBeenCalledWith("/login");
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(document.body.textContent).toBe("");
  });

  it("redirects authenticated non-admin users home without fetching stats", async () => {
    seedStore({ userId: "regular-user" });

    render(<AdminPage />);

    await waitFor(() => {
      expect(router.push).toHaveBeenCalledWith("/");
    });
    expect(fetchMock).not.toHaveBeenCalled();
    expect(document.body.textContent).toBe("");
  });

  it("fetches and renders admin stats, recent users, and session bars", async () => {
    seedStore();
    fetchMock.mockResolvedValueOnce(jsonResponse(statsFixture()));

    render(<AdminPage />);

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith("http://apas.mpaxos.com:8080/admin/stats", {
        headers: {
          Authorization: "Bearer admin-token",
        },
      });
    });

    expect(await screen.findByText("System Dashboard")).toBeTruthy();
    expect(screen.getByText("Total Users")).toBeTruthy();
    expect(screen.getByText("42")).toBeTruthy();
    expect(screen.getByText("+3 this week")).toBeTruthy();
    expect(screen.getByText("Total Sessions")).toBeTruthy();
    expect(screen.getByText("128")).toBeTruthy();
    expect(screen.getByText("12 active (24h)")).toBeTruthy();
    expect(screen.getByText("CLI Clients")).toBeTruthy();
    expect(screen.getByText("7")).toBeTruthy();
    expect(screen.getByText("2 online now")).toBeTruthy();
    expect(screen.getByText("Session Shares")).toBeTruthy();
    expect(screen.getByText("9")).toBeTruthy();
    expect(screen.getByText("new-user@example.com")).toBeTruthy();
    expect(screen.getByText("unknown-date@example.com")).toBeTruthy();
    expect(screen.getByText("N/A")).toBeTruthy();
    expect(screen.getByText("Sessions (Last 14 Days)")).toBeTruthy();
    expect(screen.getByText("2026-06-16")).toBeTruthy();
    expect(screen.getByText("2026-06-17")).toBeTruthy();
    expect(screen.getByText("10")).toBeTruthy();

    const smallerDayRow = screen.getByText("2026-06-16").parentElement;
    const largerDayRow = screen.getByText("2026-06-17").parentElement;
    expect(smallerDayRow?.querySelector(".bg-blue-500")?.getAttribute("style")).toContain(
      "width: 50%",
    );
    expect(largerDayRow?.querySelector(".bg-blue-500")?.getAttribute("style")).toContain(
      "width: 100%",
    );
  });

  it.each([401, 403])("renders access denied for %i stats responses", async (status) => {
    seedStore();
    fetchMock.mockResolvedValueOnce(jsonResponse({}, false, status));

    render(<AdminPage />);

    expect(await screen.findByText("Access denied")).toBeTruthy();
  });

  it("renders a generic failure message for non-auth stats failures", async () => {
    seedStore();
    fetchMock.mockResolvedValueOnce(jsonResponse({}, false, 500));

    render(<AdminPage />);

    expect(await screen.findByText("Failed to fetch stats")).toBeTruthy();
  });

  it("routes home when the Back button is clicked", async () => {
    seedStore();
    fetchMock.mockResolvedValueOnce(jsonResponse(statsFixture()));

    render(<AdminPage />);

    await screen.findByText("System Dashboard");
    router.push.mockClear();
    fireEvent.click(screen.getByRole("button"));

    expect(router.push).toHaveBeenCalledWith("/");
  });
});
