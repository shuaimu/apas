// @vitest-environment node
import { Terminal } from "@xterm/headless";
import { describe, expect, it } from "vitest";
import fixtures from "./terminalCheckpoints.fixture.json";

const write = (terminal: Terminal, bytes: string | Uint8Array) =>
  new Promise<void>((resolve) => terminal.write(bytes, resolve));
const read = (terminal: Terminal) => {
  const buffer = terminal.buffer.active;
  const lines = Array.from({ length: terminal.rows }, (_, row) => {
    const line = buffer.getLine(buffer.baseY + row);
    return Array.from({ length: terminal.cols }, (_, col) => {
      const cell = line?.getCell(col);
      return (
        cell && [
          cell.getChars(),
          cell.getWidth(),
          cell.getFgColor(),
          cell.getBgColor(),
          cell.isBold(),
          cell.isInverse(),
        ]
      );
    });
  });
  return {
    lines,
    cursorX: buffer.cursorX,
    cursorY: buffer.cursorY,
    type: buffer.type,
    modes: terminal.modes,
  };
};

describe("PTY checkpoint compatibility with xterm", () => {
  for (const fixture of fixtures)
    it(fixture.name, async () => {
      const { cols, rows } = fixture.checkpoint.screen;
      const original = new Terminal({
        cols,
        rows,
        allowProposedApi: true,
        scrollback: 1000,
      });
      const restored = new Terminal({
        cols,
        rows,
        allowProposedApi: true,
        scrollback: 1000,
      });
      try {
        await write(
          original,
          fixture.prefixBase64
            ? Buffer.from(fixture.prefixBase64, "base64")
            : (fixture.prefix ?? "").repeat(fixture.repeat ?? 1) +
                (fixture.suffix ?? ""),
        );
        await write(
          restored,
          Buffer.from(fixture.checkpoint.data_b64, "base64"),
        );
        expect(read(restored)).toEqual(read(original));
        const tail = fixture.tailBase64
          ? Buffer.from(fixture.tailBase64, "base64")
          : (fixture.tail ?? "");
        await write(original, tail);
        await write(restored, tail);
        expect(read(restored)).toEqual(read(original));
        const history = (terminal: Terminal) =>
          Array.from({ length: terminal.buffer.normal.length }, (_, row) =>
            terminal.buffer.normal.getLine(row)?.translateToString(true),
          );
        expect(history(restored)).toEqual(history(original));
      } finally {
        original.dispose();
        restored.dispose();
      }
    });
});
