import { beforeEach, describe, expect, it } from "vitest";
import {
  DELIVERY_GRACE_MS, loadTerminalDeliveries, saveTerminalDeliveries,
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

  it("ignores malformed browser storage", () => {
    localStorage.setItem("apas_terminal_deliveries", "{");
    expect(loadTerminalDeliveries()).toEqual([]);
    localStorage.setItem("apas_terminal_deliveries", JSON.stringify([{}, pending, null]));
    expect(loadTerminalDeliveries()).toEqual([pending]);
  });
});
