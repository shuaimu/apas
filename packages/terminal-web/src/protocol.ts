export type TerminalBridgeInbound =
  | { type: "reset"; reason?: string }
  | { type: "snapshot"; dataBase64: string; sequence: number; instanceId: string | null; truncated: boolean }
  | { type: "output"; dataBase64: string; sequence: number; instanceId: string | null }
  | { type: "lifecycle"; lifecycle: "unknown" | "running" | "disconnected" | "exited"; status: string | null }
  | { type: "theme"; theme: Record<string, string> }
  | { type: "focus" }
  | { type: "paste"; text: string };

export type TerminalBridgeOutbound =
  | { type: "ready" }
  | { type: "input"; data: string }
  | { type: "resize"; cols: number; rows: number }
  | { type: "paste_request" }
  | { type: "link_request"; url: string };

const allowedInboundKeys: Record<TerminalBridgeInbound["type"], Set<string>> = {
  reset: new Set(["type", "reason"]),
  snapshot: new Set(["type", "dataBase64", "sequence", "instanceId", "truncated"]),
  output: new Set(["type", "dataBase64", "sequence", "instanceId"]),
  lifecycle: new Set(["type", "lifecycle", "status"]),
  theme: new Set(["type", "theme"]),
  focus: new Set(["type"]),
  paste: new Set(["type", "text"]),
};

function plainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseInboundBridgeMessage(value: unknown): TerminalBridgeInbound | null {
  if (!plainObject(value) || typeof value.type !== "string" || !(value.type in allowedInboundKeys)) return null;
  const type = value.type as TerminalBridgeInbound["type"];
  if (Object.keys(value).some((key) => !allowedInboundKeys[type].has(key))) return null;
  switch (type) {
    case "reset": return value.reason === undefined || typeof value.reason === "string" ? value as TerminalBridgeInbound : null;
    case "snapshot":
      return typeof value.dataBase64 === "string" && Number.isSafeInteger(value.sequence) && (typeof value.instanceId === "string" || value.instanceId === null) && typeof value.truncated === "boolean" ? value as TerminalBridgeInbound : null;
    case "output":
      return typeof value.dataBase64 === "string" && Number.isSafeInteger(value.sequence) && (typeof value.instanceId === "string" || value.instanceId === null) ? value as TerminalBridgeInbound : null;
    case "lifecycle":
      return ["unknown", "running", "disconnected", "exited"].includes(String(value.lifecycle)) && (typeof value.status === "string" || value.status === null) ? value as TerminalBridgeInbound : null;
    case "theme":
      return plainObject(value.theme) && Object.values(value.theme).every((item) => typeof item === "string") ? value as TerminalBridgeInbound : null;
    case "focus": return value as TerminalBridgeInbound;
    case "paste": return typeof value.text === "string" && value.text.length <= 1_000_000 ? value as TerminalBridgeInbound : null;
  }
}

export function parseOutboundBridgeMessage(value: unknown): TerminalBridgeOutbound | null {
  if (!plainObject(value) || typeof value.type !== "string") return null;
  switch (value.type) {
    case "ready": return Object.keys(value).length === 1 ? { type: "ready" } : null;
    case "input": return typeof value.data === "string" && Object.keys(value).length === 2 ? { type: "input", data: value.data } : null;
    case "resize": return Number.isInteger(value.cols) && Number.isInteger(value.rows) && Number(value.cols) >= 2 && Number(value.cols) <= 500 && Number(value.rows) >= 1 && Number(value.rows) <= 300 ? value as TerminalBridgeOutbound : null;
    case "paste_request": return Object.keys(value).length === 1 ? { type: "paste_request" } : null;
    case "link_request": {
      if (typeof value.url !== "string") return null;
      try { const url = new URL(value.url); return url.protocol === "https:" ? { type: "link_request", url: url.toString() } : null; } catch { return null; }
    }
    default: return null;
  }
}
