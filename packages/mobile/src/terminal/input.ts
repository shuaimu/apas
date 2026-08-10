import { fromByteArray } from "base64-js";

export function encodeTerminalInput(value: string): string {
  return fromByteArray(new TextEncoder().encode(value));
}
