import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  subscribeTerminal,
  emitTerminal,
  hasTerminalListener,
  decodeBase64,
  encodeBase64,
  __resetTerminalBus,
  type TerminalEvent,
} from "./terminalBus";

beforeEach(() => {
  __resetTerminalBus();
});

describe("terminalBus", () => {
  it("delivers events only to the matching pane", () => {
    const a: TerminalEvent[] = [];
    const b: TerminalEvent[] = [];
    subscribeTerminal(1, (e) => a.push(e));
    subscribeTerminal(2, (e) => b.push(e));

    emitTerminal(1, { kind: "output", bytes: new Uint8Array([65]), seq: 0 });

    expect(a).toHaveLength(1);
    expect(b).toHaveLength(0);
  });

  it("stops delivering after unsubscribe and forgets the pane", () => {
    const seen: TerminalEvent[] = [];
    const unsubscribe = subscribeTerminal(7, (e) => seen.push(e));
    emitTerminal(7, { kind: "output", bytes: new Uint8Array([1]), seq: 0 });
    unsubscribe();
    emitTerminal(7, { kind: "output", bytes: new Uint8Array([2]), seq: 1 });

    expect(seen).toHaveLength(1);
    expect(hasTerminalListener(7)).toBe(false);
  });

  it("emitting to a pane with no listeners is a no-op", () => {
    expect(() =>
      emitTerminal(99, { kind: "output", bytes: new Uint8Array([1]), seq: 0 }),
    ).not.toThrow();
  });

  it("keeps delivering when one listener throws", () => {
    // A pane unmounting mid-broadcast must not stall the other terminals.
    const good: TerminalEvent[] = [];
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    subscribeTerminal(1, () => {
      throw new Error("boom");
    });
    subscribeTerminal(1, (e) => good.push(e));

    emitTerminal(1, { kind: "output", bytes: new Uint8Array([1]), seq: 0 });

    expect(good).toHaveLength(1);
    consoleError.mockRestore();
  });

  it("round-trips arbitrary bytes through base64", () => {
    // Includes bytes that are invalid UTF-8 on their own — a pty chunk can
    // end mid-sequence, and the encoding must not mangle it.
    const raw = new Uint8Array([0x1b, 0x5b, 0x32, 0x4a, 0x00, 0xff, 0xc3, 0xa9]);
    const round = decodeBase64(encodeBase64(raw));
    expect(Array.from(round)).toEqual(Array.from(raw));
  });

  it("decodes a known base64 payload to the expected bytes", () => {
    expect(Array.from(decodeBase64("aGk="))).toEqual([0x68, 0x69]);
  });
});
