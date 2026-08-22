import { describe, expect, it } from "vitest";
import { compareRecentlyIdle, type IdlePaneEntry } from "./idlePaneOrdering";

function entry(
  sessionId: string,
  paneId: number,
  idleSince?: string,
): IdlePaneEntry {
  return {
    session: { id: sessionId },
    pane: { pane_id: paneId, idle_since: idleSince },
  };
}

describe("compareRecentlyIdle", () => {
  it("puts the newest known idle transition first", () => {
    const older = entry("older", 1, "2026-08-20T12:00:00Z");
    const newer = entry("newer", 2, "2026-08-20T13:00:00Z");

    expect([older, newer].sort(compareRecentlyIdle)).toEqual([newer, older]);
  });

  it("keeps legacy and invalid timestamps after known transitions", () => {
    const legacy = entry("legacy", 1);
    const invalid = entry("invalid", 2, "not-a-date");
    const known = entry("known", 3, "2026-08-20T13:00:00Z");

    expect([legacy, known, invalid].sort(compareRecentlyIdle)[0]).toBe(known);
  });

  it("uses session and pane identity as a deterministic fallback", () => {
    const laterId = entry("session-b", 2);
    const earlierId = entry("session-a", 3);
    const earlierPane = entry("session-a", 1);

    expect([laterId, earlierId, earlierPane].sort(compareRecentlyIdle)).toEqual([
      earlierPane,
      earlierId,
      laterId,
    ]);
  });
});
