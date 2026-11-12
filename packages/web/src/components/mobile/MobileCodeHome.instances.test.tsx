import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SessionInfo } from "@/lib/store";
import { MobileCodeHome, type MobileCodeHomeProps } from "./MobileCodeHome";

/**
 * The mobile-web project list keeps one card per project however many
 * machines run it, naming the primary host plus the others.
 */

const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";

function props(overrides: Partial<MobileCodeHomeProps> = {}): MobileCodeHomeProps {
  return {
    active: false,
    connected: true,
    machineListReceived: false,
    legacySessions: [],
    token: "token",
    onAccount: vi.fn(),
    onManageMachines: vi.fn(),
    onOpenSession: vi.fn(),
    onRebootDaemon: vi.fn(),
    ...overrides,
  };
}

const live: SessionInfo = {
  id: "session-a",
  projectId: "session-a",
  workingDir: "/workspace/alpha",
  hostname: "zoo-005",
  status: "active",
  isActive: true,
  instances: [
    { machineId: ZOO5, hostname: "zoo-005", primary: true },
    { machineId: ZOO6, hostname: "zoo-006", primary: false },
  ],
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("MobileCodeHome across machines", () => {
  it("names the primary host plus the others from the live list", () => {
    render(<MobileCodeHome {...props({ legacySessions: [live] })} />);
    expect(screen.getByText("zoo-005 +1")).toBeTruthy();
  });

  it("reads the instances of a bootstrap session until the live list has it", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        sessions: [{
          id: "session-b",
          project_name: "beta",
          hostname: "zoo-006",
          working_dir: "/workspace/beta",
          status: "active",
          is_active: true,
          instances: [
            { machine_id: ZOO6, hostname: "zoo-006", primary: true },
            { machine_id: ZOO5, hostname: "zoo-005", primary: false },
            { machine_id: "50000000-0000-4000-8000-000000000007", hostname: "laptop", primary: false },
          ],
        }],
      }),
    }));
    render(<MobileCodeHome {...props({ active: true })} />);
    expect(await screen.findByText("zoo-006 +2")).toBeTruthy();
  });

  it("keeps the plain host for a project on one machine", () => {
    render(<MobileCodeHome {...props({ legacySessions: [{ ...live, instances: [live.instances![0]] }] })} />);
    expect(screen.getByText("zoo-005")).toBeTruthy();
    expect(screen.queryByText(/\+\d/)).toBeNull();
  });

  it("does not list a stopped machine's agents as idle while another machine runs the project", () => {
    // zoo-006 stopped; zoo-005 keeps the project active, and the server still
    // reports zoo-006's panes as not working.
    const session: SessionInfo = {
      ...live,
      instances: [live.instances![0]],
      panes: [
        { pane_id: 1, label: "Claude 1", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO5, hostname: "zoo-005" },
        { pane_id: 3, label: "Claude 3", kind: "terminal", provider: "claude", is_working: false, machine_id: ZOO6, hostname: "zoo-006" },
      ],
    };
    render(<MobileCodeHome {...props({ legacySessions: [session], selectedView: "idle" })} />);
    expect(screen.getByRole("button", { name: "Open Claude 1 in alpha" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open Claude 3 in alpha" })).toBeNull();

    cleanup();
    render(<MobileCodeHome {...props({ legacySessions: [session] })} />);
    expect(screen.getByRole("img", { name: "Claude 1: Idle" })).toBeTruthy();
    expect(screen.getByRole("img", { name: "Claude 3: Not running on zoo-006" })).toBeTruthy();
  });
});
