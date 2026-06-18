import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ScratchpadTicker } from "./ScratchpadTicker";
import { useStore, type TeamRecord } from "@/lib/store";

const NOW = "2026-06-16T12:00:00Z";

function record(overrides: Partial<TeamRecord>): TeamRecord {
  return {
    ts: "2026-06-16T11:00:00Z",
    kind: "status",
    pane_id: undefined,
    tags: [],
    body: "body",
    ...overrides,
  };
}

function seedTeamRecords(teamRecords: TeamRecord[]) {
  act(() => {
    useStore.setState({ teamRecords });
  });
}

describe("ScratchpadTicker", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(NOW));
    seedTeamRecords([]);
  });

  afterEach(() => {
    seedTeamRecords([]);
    vi.useRealTimers();
  });

  it("renders the empty scratchpad state", () => {
    render(<ScratchpadTicker />);

    expect(screen.getByText(/No scratchpad records yet/)).toBeTruthy();
    expect(screen.getByText(".apas-team.jsonl")).toBeTruthy();
  });

  it("renders newest records first with metadata, tag overflow, and kind filters", () => {
    seedTeamRecords([
      record({
        ts: "2026-06-16T08:00:00Z",
        kind: "delegation",
        pane_id: 178,
        tags: ["delegate-to:568"],
        body: "old delegation",
      }),
      record({
        ts: "2026-06-16T10:00:00Z",
        kind: "diff",
        pane_id: 568,
        tags: ["task:TODO-076", "diff", "reply-to:TODO-076", "extra"],
        body: "published diff",
      }),
      record({
        ts: "2026-06-16T11:30:00Z",
        kind: "review",
        pane_id: 4,
        tags: ["approves:568"],
        body: "approved diff",
      }),
      record({
        ts: "2026-06-16T11:59:00Z",
        kind: "decision",
        pane_id: 568,
        tags: ["pr-opened"],
        body: "opened PR",
      }),
    ]);

    render(<ScratchpadTicker />);

    expect(screen.getByRole("button", { name: "all (4)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "diff (1)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "review (1)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "decision (1)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "status" })).toHaveProperty(
      "disabled",
      true,
    );

    const rows = screen.getAllByRole("listitem");
    expect(within(rows[0]).getByText("decision")).toBeTruthy();
    expect(within(rows[0]).getByText("opened PR")).toBeTruthy();
    expect(within(rows[0]).getByText("1m ago")).toBeTruthy();
    expect(within(rows[0]).getByText("pane 568")).toBeTruthy();

    expect(within(rows[1]).getByText("review")).toBeTruthy();
    expect(within(rows[1]).getByText("approved diff")).toBeTruthy();
    expect(within(rows[1]).getByText("30m ago")).toBeTruthy();
    expect(within(rows[1]).getByText("pane 4")).toBeTruthy();

    expect(within(rows[2]).getByText("2h ago")).toBeTruthy();
    expect(within(rows[2]).getByText("task:TODO-076")).toBeTruthy();
    expect(within(rows[2]).getAllByText("diff")).toHaveLength(2);
    expect(within(rows[2]).getByText("reply-to:TODO-076")).toBeTruthy();
    expect(within(rows[2]).getByText("+1 more")).toBeTruthy();
    expect(within(rows[2]).queryByText("extra")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "review (1)" }));

    const filteredRows = screen.getAllByRole("listitem");
    expect(filteredRows).toHaveLength(1);
    expect(within(filteredRows[0]).getByText("review")).toBeTruthy();
    expect(within(filteredRows[0]).getByText("approved diff")).toBeTruthy();
    expect(screen.queryByText("opened PR")).toBeNull();
  });

  it("limits the inline timeline to the newest 20 records with a summary", () => {
    const records = Array.from({ length: 25 }, (_, index) =>
      record({
        ts: `2026-06-16T11:${String(index).padStart(2, "0")}:00Z`,
        kind: "status",
        pane_id: index,
        tags: [`tag-${index}`],
        body: `record-${index}`,
      }),
    );
    seedTeamRecords(records);

    render(<ScratchpadTicker />);

    const rows = screen.getAllByRole("listitem");
    expect(rows).toHaveLength(20);
    expect(within(rows[0]).getByText("record-24")).toBeTruthy();
    expect(within(rows[19]).getByText("record-5")).toBeTruthy();
    expect(screen.queryByText("record-4")).toBeNull();
    expect(screen.getByText(/Showing newest 20 of 25/)).toBeTruthy();
  });
});
