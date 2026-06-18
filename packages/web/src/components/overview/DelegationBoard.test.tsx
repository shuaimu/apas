import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useStore, type TeamRecord } from "@/lib/store";
import { DelegationBoard } from "./DelegationBoard";

const initialStore = useStore.getState();

function teamRecord(overrides: Partial<TeamRecord>): TeamRecord {
  return {
    ts: "2026-06-16T21:00:00-04:00",
    pane_id: 178,
    kind: "delegation",
    tags: ["delegate-to:568", "task:TODO-077"],
    body: "Implement TODO-077.",
    ...overrides,
  };
}

function seedTeamRecords(teamRecords: TeamRecord[]) {
  act(() => {
    useStore.setState({
      sessionId: null,
      teamRecordsBySession: new Map(),
      teamRecords,
    });
  });
}

function seedSessionTeamRecords(sessionId: string, recordsBySession: Record<string, TeamRecord[]>) {
  const teamRecordsBySession = new Map(Object.entries(recordsBySession));
  act(() => {
    useStore.setState({
      sessionId,
      teamRecordsBySession,
      teamRecords: teamRecordsBySession.get(sessionId) ?? [],
    });
  });
}

describe("DelegationBoard", () => {
  beforeEach(() => {
    seedTeamRecords([]);
  });

  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("describes the current delegate-to/task tag contract when empty", () => {
    render(<DelegationBoard />);

    expect(screen.getByText(/No delegations seen yet/i)).toBeTruthy();
    expect(screen.getByText("delegate-to:<pane_id>")).toBeTruthy();
    expect(screen.getByText("task:<TODO-NNN>")).toBeTruthy();
    expect(screen.queryByText(/task-id:<uuid>/i)).toBeNull();
  });

  it("renders current task-tag delegations as tracked rows", () => {
    seedTeamRecords([
      teamRecord({
        tags: [
          "delegate-to:568",
          "task:TODO-077 · delegation-board-task-tags",
          "task:TODO-077",
        ],
        body: "Align DelegationBoard with task tags.",
      }),
    ]);

    render(<DelegationBoard />);

    expect(screen.getByText(/pane 178.*pane 568/)).toBeTruthy();
    expect(screen.getByText("TODO-077")).toBeTruthy();
    expect(screen.getByTitle("Align DelegationBoard with task tags.")).toBeTruthy();
    expect(screen.getByText("awaiting reply")).toBeTruthy();
  });

  it("pairs replies by current task tags and task aliases", () => {
    seedTeamRecords([
      teamRecord({
        ts: "2026-06-16T21:00:00-04:00",
        tags: ["delegate-to:568", "task:TODO-077 · delegation-board-task-tags"],
        body: "Delegated current task.",
      }),
      teamRecord({
        ts: "2026-06-16T21:10:00-04:00",
        pane_id: 568,
        kind: "diff",
        tags: ["diff", "task:TODO-077"],
        body: "Diff ready.",
      }),
    ]);

    render(<DelegationBoard />);

    expect(screen.getByText("replied")).toBeTruthy();
    expect(screen.getByText("+10m")).toBeTruthy();
    expect(screen.queryByText("awaiting reply")).toBeNull();
  });

  it("builds delegation rows only from the active session records", () => {
    seedSessionTeamRecords("session-a", {
      "session-a": [
        teamRecord({
          tags: ["delegate-to:568", "task:TODO-127"],
          body: "Session A work.",
        }),
      ],
      "session-b": [
        teamRecord({
          tags: ["delegate-to:568", "task:TODO-999"],
          body: "Session B work.",
        }),
      ],
    });

    render(<DelegationBoard />);

    expect(screen.getByText("TODO-127")).toBeTruthy();
    expect(screen.getByTitle("Session A work.")).toBeTruthy();
    expect(screen.queryByText("TODO-999")).toBeNull();
    expect(screen.queryByTitle("Session B work.")).toBeNull();
  });

  it("retains legacy task-id and reply-to fallback pairing", () => {
    seedTeamRecords([
      teamRecord({
        tags: ["delegate-to:568", "task-id:abc123456789"],
        body: "Legacy task id delegation.",
      }),
      teamRecord({
        ts: "2026-06-16T21:00:30-04:00",
        pane_id: 568,
        kind: "diff",
        tags: ["reply-to:abc123456789", "diff"],
        body: "Legacy reply.",
      }),
    ]);

    render(<DelegationBoard />);

    expect(screen.getByText("abc12345")).toBeTruthy();
    expect(screen.getByText("replied")).toBeTruthy();
    expect(screen.getByText("+30s")).toBeTruthy();
  });

  it("orders delegation rows newest first", () => {
    seedTeamRecords([
      teamRecord({
        ts: "2026-06-16T20:00:00-04:00",
        tags: ["delegate-to:568", "task:TODO-070"],
        body: "Older work.",
      }),
      teamRecord({
        ts: "2026-06-16T21:00:00-04:00",
        tags: ["delegate-to:568", "task:TODO-077"],
        body: "Newer work.",
      }),
    ]);

    render(<DelegationBoard />);

    const rows = screen.getAllByRole("row");
    expect(within(rows[1]).getByText("TODO-077")).toBeTruthy();
    expect(within(rows[1]).getByTitle("Newer work.")).toBeTruthy();
    expect(within(rows[2]).getByText("TODO-070")).toBeTruthy();
  });

  it("caps visible rows at the newest 20 delegations", () => {
    seedTeamRecords(
      Array.from({ length: 25 }, (_, index) => {
        const todoNumber = String(index + 1).padStart(3, "0");
        return teamRecord({
          ts: `2026-06-16T21:${String(index).padStart(2, "0")}:00-04:00`,
          tags: ["delegate-to:568", `task:TODO-${todoNumber}`],
          body: `Work item ${todoNumber}.`,
        });
      }),
    );

    render(<DelegationBoard />);

    expect(screen.getAllByText("awaiting reply")).toHaveLength(20);
    expect(screen.getByText("Showing newest 20 of 25.")).toBeTruthy();
    expect(screen.getByText("TODO-025")).toBeTruthy();
    expect(screen.getByText("TODO-006")).toBeTruthy();
    expect(screen.queryByText("TODO-005")).toBeNull();
  });
});
