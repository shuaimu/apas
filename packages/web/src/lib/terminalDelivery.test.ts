import { beforeEach, describe, expect, it } from "vitest";
import {
  DELIVERY_GRACE_MS, loadTerminalDeliveries, refusedDeliveries, saveTerminalDeliveries,
  unconfirmedDeliveries, type PendingDelivery,
} from "./terminalDelivery";

const pending: PendingDelivery = {
  id: "send-1", sessionId: "session-a", paneId: 3, text: "hello", sentAt: 1_000_000,
};
beforeEach(() => saveTerminalDeliveries([]));

describe("terminal delivery recovery", () => {
  it("warns after the transcript polling grace period", () => {
    expect(unconfirmedDeliveries([pending], pending.sentAt + DELIVERY_GRACE_MS - 1)).toEqual([]);
    expect(unconfirmedDeliveries([pending], pending.sentAt + DELIVERY_GRACE_MS)).toEqual([pending]);
  });

  it("preserves text and target after a browser reload until confirmed or dismissed", () => {
    saveTerminalDeliveries([pending]);
    expect(loadTerminalDeliveries()).toEqual([pending]);
    saveTerminalDeliveries([]);
    expect(loadTerminalDeliveries()).toEqual([]);
  });

  it("reports a refused send with its reason at once, never as unconfirmed", () => {
    const refused = { ...pending, id: "send-2", error: "zoo-005 is not running this project. Open the project again to start it there." };
    expect(unconfirmedDeliveries([pending, refused], pending.sentAt + DELIVERY_GRACE_MS)).toEqual([pending]);
    expect(refusedDeliveries([pending, refused])).toEqual([refused]);
    saveTerminalDeliveries([refused]);
    expect(loadTerminalDeliveries()).toEqual([refused]);
  });

  it("ignores malformed browser storage", () => {
    localStorage.setItem("apas_terminal_deliveries", "{");
    expect(loadTerminalDeliveries()).toEqual([]);
    localStorage.setItem("apas_terminal_deliveries", JSON.stringify([{}, pending, null, { ...pending, id: "send-3", error: 42 }]));
    expect(loadTerminalDeliveries()).toEqual([pending]);
  });
});
