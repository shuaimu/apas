import { describe, expect, it } from "vitest";
import {
  DELIVERY_GRACE_MS,
  isDeliveryConfirmed,
  pruneConfirmed,
  unconfirmedDeliveries,
  type DeliveryTurn,
  type PendingDelivery,
} from "./terminalDelivery";

const SENT_AT = 1_000_000;
const pending = (text: string, paneId = 3): PendingDelivery => ({
  paneId,
  text,
  sentAt: SENT_AT,
});
const turn = (
  role: string,
  content: string,
  offsetMs = 500,
): DeliveryTurn => ({ role, content, timestampMs: SENT_AT + offsetMs });

describe("confirming a message landed", () => {
  it("is confirmed by the provider recording it", () => {
    expect(isDeliveryConfirmed(pending("hello"), [turn("user", "hello")])).toBe(true);
  });

  it("tolerates framing the provider adds around the text", () => {
    expect(
      isDeliveryConfirmed(pending("hello"), [turn("user", "> hello\n")]),
    ).toBe(true);
  });

  it("ignores whitespace differences", () => {
    expect(
      isDeliveryConfirmed(pending("run   the  tests"), [turn("user", "run the tests")]),
    ).toBe(true);
  });

  it("is not confirmed by the agent merely saying it back", () => {
    // An assistant echoing the text proves nothing about what the pty received.
    expect(
      isDeliveryConfirmed(pending("hello"), [turn("assistant", "hello")]),
    ).toBe(false);
  });

  it("is not confirmed by a turn recorded before it was sent", () => {
    // The same word from earlier in the conversation must not stand in for it.
    expect(
      isDeliveryConfirmed(pending("hello"), [turn("user", "hello", -5_000)]),
    ).toBe(false);
  });

  it("is not confirmed by nothing at all", () => {
    expect(isDeliveryConfirmed(pending("hello"), [])).toBe(false);
  });
});

describe("what gets reported as unconfirmed", () => {
  it("says nothing while still within the grace window", () => {
    // The transcript is polled; warning immediately would fire on every message.
    const now = SENT_AT + DELIVERY_GRACE_MS - 1;
    expect(unconfirmedDeliveries([pending("hello")], [], now)).toEqual([]);
  });

  it("reports a message the provider never recorded", () => {
    const now = SENT_AT + DELIVERY_GRACE_MS;
    expect(unconfirmedDeliveries([pending("hello")], [], now)).toEqual([
      pending("hello"),
    ]);
  });

  it("stays quiet once the message is recorded, however late", () => {
    const now = SENT_AT + DELIVERY_GRACE_MS * 10;
    expect(
      unconfirmedDeliveries([pending("hello")], [turn("user", "hello")], now),
    ).toEqual([]);
  });

  it("reports only the messages that are missing", () => {
    const now = SENT_AT + DELIVERY_GRACE_MS;
    const landed = pending("landed");
    const lost = pending("lost");
    expect(
      unconfirmedDeliveries([landed, lost], [turn("user", "landed")], now),
    ).toEqual([lost]);
  });
});

describe("pruning", () => {
  it("drops confirmed entries so the list cannot grow without bound", () => {
    const landed = pending("landed");
    const lost = pending("lost");
    expect(pruneConfirmed([landed, lost], [turn("user", "landed")])).toEqual([lost]);
  });

  it("keeps everything while nothing is confirmed", () => {
    const entries = [pending("a"), pending("b")];
    expect(pruneConfirmed(entries, [])).toEqual(entries);
  });
});
