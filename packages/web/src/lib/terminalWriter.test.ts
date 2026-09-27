import { expect, it, vi } from "vitest";
import { TerminalWriter } from "./terminalWriter";
it("orders reset, dimensions, checkpoint, and live output after an in-flight write", () => {
  const callbacks: Array<() => void> = [],
    calls: unknown[] = [];
  const terminal = {
    write: (bytes: Uint8Array, done: () => void) => {
      calls.push(Array.from(bytes));
      callbacks.push(done);
    },
    reset: () => calls.push("reset"),
    resize: (cols: number, rows: number) => calls.push([cols, rows]),
  };
  const writer = new TerminalWriter(terminal, vi.fn());
  writer.write(new Uint8Array([1]));
  writer.write(new Uint8Array([2]));
  writer.reset();
  writer.resize(90, 30);
  writer.write(new Uint8Array([3]));
  writer.write(new Uint8Array([4]));
  expect(writer.restoring).toBe(true);
  expect(calls).toEqual([[1]]);
  callbacks.shift()!();
  expect(calls).toEqual([[1], "reset", [90, 30], [3]]);
  expect(writer.restoring).toBe(true);
  callbacks.shift()!();
  expect(calls).toEqual([[1], "reset", [90, 30], [3], [4]]);
  expect(writer.restoring).toBe(false);
});
