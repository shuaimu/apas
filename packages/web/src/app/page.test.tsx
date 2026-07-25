import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Home from "./page";
import { useStore, type CliClient } from "@/lib/store";

const router = vi.hoisted(() => ({
  push: vi.fn(),
}));

const fetchMock = vi.hoisted(() => vi.fn());
const clearAllSnapshotsMock = vi.hoisted(() => vi.fn());
const reloadWindowMock = vi.hoisted(() => vi.fn());

const storeMock = vi.hoisted(() => {
  const initialState = {
    cliClientId: null,
    cliClients: [],
    connect: vi.fn(),
    connected: false,
    disconnect: vi.fn(),
    isAuthenticated: false,
    logout: vi.fn(),
    serverVersion: null,
    sessionId: null,
    setUserEmail: vi.fn(),
    token: null,
    userEmail: null,
    userId: null,
  };
  const state: Record<string, unknown> = { ...initialState };
  const useStoreMock = vi.fn(() => state);

  Object.assign(useStoreMock, {
    getInitialState: () => ({ ...initialState }),
    setState: (partial: Record<string, unknown> | ((current: Record<string, unknown>) => Record<string, unknown>), replace?: boolean) => {
      const nextState = typeof partial === "function" ? partial(state) : partial;
      if (replace) {
        for (const key of Object.keys(state)) {
          delete state[key];
        }
      }
      Object.assign(state, nextState);
    },
  });

  return {
    useStore: useStoreMock,
  };
});

vi.mock("next/navigation", () => ({
  useRouter: () => router,
}));

vi.mock("@/lib/store", () => ({
  useStore: storeMock.useStore,
}));

vi.mock("@/lib/browserActions", () => ({
  reloadWindow: reloadWindowMock,
}));

vi.mock("@/components/Sidebar", () => ({
  Sidebar: ({
    onCollapse,
    width,
  }: {
    onCollapse?: () => void;
    width?: number;
  }) => (
    <aside data-testid="sidebar" data-width={width ?? ""}>
      <button onClick={onCollapse}>Collapse sidebar</button>
    </aside>
  ),
}));

vi.mock("@/components/tabs/TabbedView", () => ({
  TabbedView: () => <div data-testid="tabbed-view" />,
}));

vi.mock("@/lib/sessionCacheDb", () => ({
  clearAllSnapshots: clearAllSnapshotsMock,
  deleteSnapshot: vi.fn(),
  loadAllSnapshots: vi.fn(() => Promise.resolve(new Map())),
  loadSnapshot: vi.fn(() => Promise.resolve(undefined)),
  saveSnapshot: vi.fn(),
}));

const initialStore = useStore.getInitialState();

beforeEach(() => {
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  cleanup();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  act(() => {
    useStore.setState(initialStore, true);
  });
});

function makeCliClient(overrides: Partial<CliClient> & Pick<CliClient, "id">): CliClient {
  return {
    status: "online",
    ...overrides,
  };
}

function setDesktopWidth(width = 1024) {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: width,
  });
}

function seedAuthenticatedState(cliClientId: string | null = "project-a", overrides: Record<string, unknown> = {}) {
  localStorage.setItem("apas_token", "token");
  act(() => {
    useStore.setState({
      cliClientId,
      cliClients: cliClientId
        ? [
            makeCliClient({
              id: cliClientId,
              activeSession: "session-a",
              version: "1.2.3",
            }),
          ]
        : [],
      connect: vi.fn(),
      connected: true,
      disconnect: vi.fn(),
      isAuthenticated: true,
      logout: vi.fn(),
      serverVersion: "test-server",
      sessionId: "session-a",
      setUserEmail: vi.fn(),
      token: "token",
      userEmail: "user@example.com",
      userId: "user-1",
      ...overrides,
    });
  });
}

async function renderHome(cliClientId: string | null = "project-a") {
  renderAuthenticatedHome(cliClientId);
  return screen.findByTestId("sidebar");
}

function renderAuthenticatedHome(cliClientId: string | null = "project-a") {
  setDesktopWidth();
  seedAuthenticatedState(cliClientId);
  render(<Home />);
}

function renderDashboard(overrides: Record<string, unknown> = {}) {
  setDesktopWidth();
  seedAuthenticatedState("project-a", overrides);
  return render(<Home />);
}

function sidebarWidth(): string | null {
  return screen.getByTestId("sidebar").getAttribute("data-width");
}

function seedUnauthenticatedState(overrides: Record<string, unknown> = {}) {
  act(() => {
    useStore.setState({
      cliClientId: null,
      cliClients: [],
      connect: vi.fn(),
      connected: false,
      disconnect: vi.fn(),
      isAuthenticated: false,
      logout: vi.fn(),
      serverVersion: null,
      sessionId: null,
      setUserEmail: vi.fn(),
      token: null,
      userEmail: null,
      userId: null,
      ...overrides,
    });
  });
}

function jsonResponse(body: unknown, ok = true): Response {
  return {
    ok,
    json: vi.fn().mockResolvedValue(body),
  } as unknown as Response;
}

describe("Home auth bootstrap", () => {
  it("redirects to login without calling connect when no token is stored", async () => {
    const connect = vi.fn();
    seedUnauthenticatedState({ connect });

    render(<Home />);

    await waitFor(() => {
      expect(router.push).toHaveBeenCalledWith("/login");
    });
    expect(connect).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("clears the loading state and connects when a token is stored", async () => {
    const connect = vi.fn();
    localStorage.setItem("apas_token", "stored-token");
    seedUnauthenticatedState({ connect, userEmail: "user@example.com" });

    render(<Home />);

    await screen.findByTestId("tabbed-view");
    expect(screen.queryByText("Loading...")).toBeNull();
    expect(connect).toHaveBeenCalledTimes(1);
    expect(router.push).not.toHaveBeenCalledWith("/login");
  });

  it("hydrates missing user email using the stored bearer token", async () => {
    const setUserEmail = vi.fn();
    localStorage.setItem("apas_token", "stored-token");
    fetchMock.mockResolvedValueOnce(jsonResponse({ user_email: "user@example.com" }));
    seedUnauthenticatedState({
      connect: vi.fn(),
      connected: true,
      isAuthenticated: true,
      setUserEmail,
      token: "stored-token",
      userEmail: null,
      userId: "user-1",
    });

    render(<Home />);

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "http://apas.mpaxos.com/auth/me",
        expect.objectContaining({
          headers: { Authorization: "Bearer stored-token" },
        }),
      );
    });
    await waitFor(() => {
      expect(setUserEmail).toHaveBeenCalledWith("user@example.com");
    });
  });

  it.each([
    ["non-OK", () => Promise.resolve(jsonResponse({ user_email: "ignored@example.com" }, false))],
    ["rejected", () => Promise.reject(new Error("network unavailable"))],
  ])("ignores %s user-email hydration without breaking dashboard render", async (_name, authMeResult) => {
    const setUserEmail = vi.fn();
    localStorage.setItem("apas_token", "stored-token");
    fetchMock.mockImplementationOnce(authMeResult);
    seedUnauthenticatedState({
      connect: vi.fn(),
      connected: true,
      isAuthenticated: true,
      setUserEmail,
      token: "stored-token",
      userEmail: null,
      userId: "user-1",
    });

    render(<Home />);

    expect(await screen.findByTestId("tabbed-view")).toBeTruthy();
    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalled();
    });
    expect(setUserEmail).not.toHaveBeenCalled();
    expect(router.push).not.toHaveBeenCalledWith("/login");
  });

  it("logs out and routes back to login", async () => {
    const logout = vi.fn();
    seedAuthenticatedState();
    act(() => {
      useStore.setState({ logout });
    });

    render(<Home />);

    fireEvent.click(await screen.findByTitle("Logout"));

    expect(logout).toHaveBeenCalledTimes(1);
    expect(router.push).toHaveBeenCalledWith("/login");
  });
});

describe("Home reconnect and local cache controls", () => {
  it("runs one reconnect sequence after the intended delays", async () => {
    const connect = vi.fn();
    const disconnect = vi.fn();
    renderDashboard({ connect, connected: true, disconnect });
    await screen.findByTestId("tabbed-view");
    connect.mockClear();
    vi.useFakeTimers();

    const reconnectButton = screen.getByTitle("Click to reconnect");
    fireEvent.click(reconnectButton);

    expect(screen.getByText("Reconnecting...")).toBeTruthy();
    fireEvent.click(reconnectButton);
    act(() => {
      vi.advanceTimersByTime(49);
    });
    expect(disconnect).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(disconnect).toHaveBeenCalledTimes(1);
    expect(connect).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(499);
    });
    expect(connect).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(connect).toHaveBeenCalledTimes(1);
  });

  it("clears reconnecting state when the store reports a connection", async () => {
    const connect = vi.fn();
    const { rerender } = renderDashboard({ connect, connected: false });
    await screen.findByTestId("tabbed-view");
    connect.mockClear();
    vi.useFakeTimers();

    fireEvent.click(screen.getByTitle("Click to connect"));
    expect(screen.getByText("Reconnecting...")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(550);
    });
    expect(connect).toHaveBeenCalledTimes(1);

    act(() => {
      useStore.setState({ connected: true });
    });
    rerender(<Home />);

    expect(screen.queryByText("Reconnecting...")).toBeNull();
    expect(screen.getByText("Connected")).toBeTruthy();
  });

  it("clears reconnecting state after the timeout when still disconnected", async () => {
    renderDashboard({ connected: false });
    await screen.findByTestId("tabbed-view");
    vi.useFakeTimers();

    fireEvent.click(screen.getByTitle("Click to connect"));
    expect(screen.getByText("Reconnecting...")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(4999);
    });
    expect(screen.getByText("Reconnecting...")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByText("Reconnecting...")).toBeNull();
    expect(screen.getByText("Disconnected")).toBeTruthy();
  });

  it("confirms or cancels clearing local cache from Settings", async () => {
    let resolveClear!: () => void;
    clearAllSnapshotsMock.mockReturnValue(new Promise<void>((resolve) => {
      resolveClear = resolve;
    }));
    renderDashboard();
    await screen.findByTestId("tabbed-view");

    fireEvent.click(screen.getByTitle("Settings"));
    fireEvent.click(screen.getByText("Clear local cache"));
    expect(screen.getByText("Confirm clear & reload")).toBeTruthy();

    fireEvent.click(screen.getByText("Cancel"));
    expect(screen.getByText("Clear local cache")).toBeTruthy();
    expect(clearAllSnapshotsMock).not.toHaveBeenCalled();
    expect(reloadWindowMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Clear local cache"));
    fireEvent.click(screen.getByText("Confirm clear & reload"));

    expect(screen.getByText(/Clearing cache and reloading/)).toBeTruthy();
    expect(clearAllSnapshotsMock).toHaveBeenCalledTimes(1);
    expect(reloadWindowMock).not.toHaveBeenCalled();

    await act(async () => {
      resolveClear();
    });
    await waitFor(() => {
      expect(reloadWindowMock).toHaveBeenCalledTimes(1);
    });
  });
});

describe("Home sidebar layout persistence", () => {
  it("hydrates sidebar width from global layout when no project is selected", async () => {
    localStorage.setItem("apas_layout_global_sidebar_width", "288");

    await renderHome(null);

    await waitFor(() => {
      expect(sidebarWidth()).toBe("288");
    });
  });

  it("hydrates project sidebar width before falling back to global layout", async () => {
    localStorage.setItem("apas_layout_global_sidebar_width", "288");
    localStorage.setItem("apas_layout_project-a_sidebar_width", "340");

    await renderHome("project-a");

    await waitFor(() => {
      expect(sidebarWidth()).toBe("340");
    });
  });

  it("clamps rendered ResizeHandle drags and persists width on resize end", async () => {
    localStorage.setItem("apas_layout_project-a_sidebar_width", "250");

    await renderHome("project-a");
    await waitFor(() => {
      expect(sidebarWidth()).toBe("250");
    });

    const handle = screen.getByTitle("Drag to resize");

    fireEvent.mouseDown(handle, { clientX: 100 });
    fireEvent.mouseMove(document, { clientX: 500 });
    await waitFor(() => {
      expect(sidebarWidth()).toBe("400");
    });
    fireEvent.mouseUp(document);

    expect(localStorage.getItem("apas_layout_project-a_sidebar_width")).toBe("400");

    fireEvent.mouseDown(handle, { clientX: 500 });
    fireEvent.mouseMove(document, { clientX: 0 });
    await waitFor(() => {
      expect(sidebarWidth()).toBe("180");
    });
    fireEvent.mouseUp(document);

    expect(localStorage.getItem("apas_layout_project-a_sidebar_width")).toBe("180");
  });

  it("persists project sidebar collapse without disturbing other layout keys", async () => {
    localStorage.setItem("apas_layout_global_sidebar_width", "288");
    localStorage.setItem("apas_layout_project-a_sidebar_width", "310");
    localStorage.setItem("apas_layout_project-a_other_key", "keep-me");

    await renderHome("project-a");

    fireEvent.click(screen.getByText("Collapse sidebar"));

    expect(localStorage.getItem("apas_layout_project-a_sidebar_collapsed")).toBe("true");
    expect(localStorage.getItem("apas_layout_global_sidebar_width")).toBe("288");
    expect(localStorage.getItem("apas_layout_project-a_sidebar_width")).toBe("310");
    expect(localStorage.getItem("apas_layout_project-a_other_key")).toBe("keep-me");

    fireEvent.click(screen.getByTitle("Expand sidebar"));

    expect(localStorage.getItem("apas_layout_project-a_sidebar_collapsed")).toBe("false");
    expect(localStorage.getItem("apas_layout_global_sidebar_width")).toBe("288");
    expect(localStorage.getItem("apas_layout_project-a_sidebar_width")).toBe("310");
    expect(localStorage.getItem("apas_layout_project-a_other_key")).toBe("keep-me");
  });

  it("hydrates project sidebar collapsed state before rendering the shell", async () => {
    localStorage.setItem("apas_layout_project-a_sidebar_width", "310");
    localStorage.setItem("apas_layout_project-a_sidebar_collapsed", "true");

    renderAuthenticatedHome("project-a");

    await screen.findByTestId("tabbed-view");
    await waitFor(() => {
      expect(screen.queryByTestId("sidebar")).toBeNull();
    });
    expect(screen.getByTitle("Expand sidebar")).toBeTruthy();
    expect(localStorage.getItem("apas_layout_project-a_sidebar_collapsed")).toBe("true");
    expect(localStorage.getItem("apas_layout_project-a_sidebar_width")).toBe("310");
  });
});
