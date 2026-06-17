import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import Home from "./page";
import { useStore, type CliClient } from "@/lib/store";

const router = vi.hoisted(() => ({
  push: vi.fn(),
}));

vi.mock("next/navigation", () => ({
  useRouter: () => router,
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
  clearAllSnapshots: vi.fn(),
  deleteSnapshot: vi.fn(),
  loadAllSnapshots: vi.fn(() => Promise.resolve(new Map())),
  loadSnapshot: vi.fn(() => Promise.resolve(undefined)),
  saveSnapshot: vi.fn(),
}));

const initialStore = useStore.getInitialState();

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

function seedAuthenticatedState(cliClientId: string | null = "project-a") {
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

function sidebarWidth(): string | null {
  return screen.getByTestId("sidebar").getAttribute("data-width");
}

describe("Home sidebar layout persistence", () => {
  afterEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    document.body.innerHTML = "";
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

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
