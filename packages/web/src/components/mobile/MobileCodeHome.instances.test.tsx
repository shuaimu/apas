import { cleanup, render, screen, waitFor } from "@testing-library/react";
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

/**
 * This screen loads its sessions over HTTP once (the bootstrap) and merges
 * the live list into them. A project that is deleted, in this window or in
 * another, simply stops being named by the live list, and its bootstrap
 * entry used to stay as a live card.
 */
describe("MobileCodeHome and a project the store forgot", () => {
  const bootstrapSession = (id: string, name: string) => ({
    id,
    project_id: id,
    project_name: name,
    hostname: "zoo-005",
    working_dir: `/workspace/${name}`,
    status: "active",
    is_active: true,
    // Only the bootstrap carries a summary: its text shows that the
    // snapshot has been loaded.
    latest_summary: `Summary of ${name}`,
    panes: [{ pane_id: 1, label: "Claude 1", kind: "terminal", provider: "claude", is_working: false }],
  });
  const liveSession = (id: string, name: string): SessionInfo => ({
    id,
    projectId: id,
    workingDir: `/workspace/${name}`,
    hostname: "zoo-005",
    status: "active",
    isActive: true,
    panes: [{ pane_id: 1, label: "Claude 1", kind: "terminal", provider: "claude", is_working: false }],
  });
  function bootstrap(sessions: unknown[]) {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ sessions, machines: [] }) });
    vi.stubGlobal("fetch", fetchMock);
    return fetchMock;
  }

  it("drops the card of a project deleted in another window, without a reload", async () => {
    bootstrap([bootstrapSession("os-delete", "os-delete"), bootstrapSession("os-keep", "os-keep")]);
    const before = props({
      active: true,
      legacySessions: [liveSession("os-delete", "os-delete"), liveSession("os-keep", "os-keep")],
      forgottenProjects: {},
    });
    const view = render(<MobileCodeHome {...before} />);
    // The bootstrap is in: the card is the merged one.
    await waitFor(() => expect(screen.getByRole("button", { name: "Open os-delete" }).textContent).toContain("Summary of os-delete"));
    expect(screen.getByRole("button", { name: "Open os-delete" }).textContent).toContain("Idle");

    // `project_access_changed` "deleted": the store forgets the project, and
    // the sessions frames that follow no longer list it.
    view.rerender(<MobileCodeHome {...before} legacySessions={[liveSession("os-keep", "os-keep")]} forgottenProjects={{ "os-delete": 1 }} />);
    expect(screen.queryByRole("button", { name: "Open os-delete" })).toBeNull();
    expect(screen.queryByText("os-delete")).toBeNull();
    expect(screen.getByRole("button", { name: "Open os-keep" })).toBeTruthy();

    // And not among the idle agents either.
    view.rerender(<MobileCodeHome {...before} selectedView="idle" legacySessions={[liveSession("os-keep", "os-keep")]} forgottenProjects={{ "os-delete": 1 }} />);
    expect(screen.queryByRole("button", { name: "Open Claude 1 in os-delete" })).toBeNull();
    expect(screen.getByRole("button", { name: "Open Claude 1 in os-keep" })).toBeTruthy();
  });

  it("keeps a bootstrap session the live list does not name while the store has not forgotten it", async () => {
    bootstrap([bootstrapSession("os-quiet", "os-quiet")]);
    render(<MobileCodeHome {...props({ active: true, forgottenProjects: { "another-project": 3 } })} />);
    expect(await screen.findByRole("button", { name: "Open os-quiet" })).toBeTruthy();
  });

  it("does not bring a forgotten project back from an answer that was asked for before it was forgotten", async () => {
    let answer: (value: unknown) => void = () => {};
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise((resolve) => { answer = resolve; })));
    const base = props({ active: true, forgottenProjects: {} });
    const view = render(<MobileCodeHome {...base} />);
    // Forgotten while the request is under way; its answer still has it.
    view.rerender(<MobileCodeHome {...base} forgottenProjects={{ "os-delete": 1 }} />);
    answer({ ok: true, json: async () => ({ sessions: [bootstrapSession("os-delete", "os-delete"), bootstrapSession("os-keep", "os-keep")], machines: [] }) });
    expect(await screen.findByRole("button", { name: "Open os-keep" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open os-delete" })).toBeNull();
  });

  it("shows a project again when the live list names it after it was forgotten", async () => {
    // A machine that was offline during the deletion recreates the project.
    bootstrap([bootstrapSession("os-delete", "os-delete")]);
    const base = props({ active: true, forgottenProjects: {} });
    const view = render(<MobileCodeHome {...base} />);
    expect(await screen.findByText("Summary of os-delete")).toBeTruthy();
    view.rerender(<MobileCodeHome {...base} forgottenProjects={{ "os-delete": 1 }} />);
    expect(screen.queryByRole("button", { name: "Open os-delete" })).toBeNull();
    view.rerender(<MobileCodeHome {...base} forgottenProjects={{ "os-delete": 1 }} legacySessions={[liveSession("os-delete", "os-delete")]} />);
    expect(screen.getByRole("button", { name: "Open os-delete" })).toBeTruthy();
  });
});
