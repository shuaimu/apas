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
  it("renders the native-style compact session home and filters in place", () => {
    renderHome();

    expect(screen.getByRole("heading", { name: "Coding sessions" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Account" })).toBeTruthy();
    expect(screen.getByRole("button", { name: /New task/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Active" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Open alpha" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open beta" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Completed" }));
    expect(screen.getByRole("button", { name: "Open beta" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open alpha" })).toBeNull();
  });

  it("keeps project selection compact and combines it with status filters", () => {
    renderHome();
    fireEvent.click(screen.getByRole("button", { name: "Recent" }));
    fireEvent.click(screen.getByRole("button", { name: "beta" }));

    expect(screen.getByRole("button", { name: "Open beta" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open alpha" })).toBeNull();
  });

  it("uses server-authoritative attention counts for the Attention filter", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        sessions: [
          {
            id: "session-a",
            project_name: "alpha",
            status: "active",
            is_active: true,
            attention_count: 2,
          },
          {
            id: "session-b",
            project_name: "beta",
            status: "active",
            is_active: true,
            attention_count: 0,
          },
        ],
      }),
    }));
    renderHome({ active: true, legacySessions: [] });

    fireEvent.click(screen.getByRole("button", { name: "Attention" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Open alpha" })).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Open beta" })).toBeNull();
    });
    expect(screen.getByText("2 attention")).toBeTruthy();
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
