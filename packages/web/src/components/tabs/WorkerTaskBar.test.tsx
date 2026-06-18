import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore, type TeamRecord } from "@/lib/store";
import { WorkerTaskBar } from "./WorkerTaskBar";

const initialStore = useStore.getState();
const NOW = new Date("2026-06-16T12:00:00Z");

function teamRecord(overrides: Partial<TeamRecord>): TeamRecord {
  return {
    ts: "2026-06-16T11:58:00Z",
    kind: "delegation",
    tags: ["delegate-to:568", "task:TODO-051"],
    body: "Implement TODO-051.",
    ...overrides,
  };
}

function seedTeamRecords(teamRecords: TeamRecord[]) {
  act(() => {
    useStore.setState({ teamRecords });
  });
}

describe("WorkerTaskBar", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    seedTeamRecords([]);
  });

  afterEach(() => {
    vi.useRealTimers();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders nothing for unmanaged panes instead of delegation waiting copy", () => {
    const { container } = render(
      <WorkerTaskBar paneId={568} role="developer" managed={false} />,
    );

    expect(container.firstChild).toBeNull();
    expect(screen.queryByText(/waiting for the Tech Lead/i)).toBeNull();
  });

  it.each(["team manager", "tech lead"])(
    "renders nothing for managed coordinator role %s",
    (role) => {
      const { container } = render(
        <WorkerTaskBar paneId={568} role={role} managed={true} />,
      );

      expect(container.firstChild).toBeNull();
      expect(screen.queryByText(/Current task/i)).toBeNull();
      expect(screen.queryByText(/waiting for the Tech Lead/i)).toBeNull();
    },
  );

  it("shows the waiting state for managed worker panes without a delegation", () => {
    seedTeamRecords([
      teamRecord({
        tags: ["delegate-to:999", "task:TODO-OTHER"],
        body: "Work assigned to another pane.",
      }),
    ]);

    render(<WorkerTaskBar paneId={568} role="developer" managed={true} />);

    expect(screen.getByText(/No active task/i)).toBeTruthy();
    expect(screen.getByText(/waiting for the Tech Lead to delegate/i)).toBeTruthy();
  });

  it.each(["developer", "reviewer"])(
    "shows the latest delegated task for a managed %s pane",
    (role) => {
      seedTeamRecords([
        teamRecord({
          ts: "2026-06-16T11:30:00Z",
          tags: ["delegate-to:568", "task:TODO-OLD"],
          body: "Older work.",
        }),
        teamRecord({
          ts: "2026-06-16T11:59:00Z",
          tags: ["delegate-to:999", "task:TODO-OTHER"],
          body: "Other pane work.",
        }),
        teamRecord({
          ts: "2026-06-16T11:58:00Z",
          tags: ["delegate-to:568", "task:TODO-051"],
          body: "Cover unmanaged task bar suppression.",
        }),
      ]);

      render(<WorkerTaskBar paneId={568} role={role} managed={true} />);

      expect(screen.getByText("Current task:")).toBeTruthy();
      expect(screen.getByText("TODO-051")).toBeTruthy();
      expect(screen.getByText(/received 2m ago/i)).toBeTruthy();
      expect(screen.getByTitle("Cover unmanaged task bar suppression.")).toBeTruthy();
    },
  );
});
