import { describe, expect, it } from "vitest";
import { deriveInitialActiveTabId, OVERVIEW_PANE_ID } from "./TabbedView";

describe("deriveInitialActiveTabId", () => {
  const base = {
    activeTabId: null,
    clientChanged: true,
    managerTabId: null,
    paneConfigsLength: 2,
    savedActiveTab: "",
    tabIds: [10, 20],
  };

  it("keeps a valid saved active tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "20",
      }),
    ).toBe(20);
  });

  it("prefers the interactive Manager tab when there is no valid saved tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "",
      }),
    ).toBe(10);
  });

  it("falls back to Overview when there is no Manager and no valid saved tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: null,
        savedActiveTab: "",
      }),
    ).toBe(OVERVIEW_PANE_ID);
  });

  it("ignores stale saved tabs and still uses Manager or Overview fallback", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "999",
      }),
    ).toBe(10);

    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: null,
        savedActiveTab: "999",
      }),
    ).toBe(OVERVIEW_PANE_ID);
  });
});
