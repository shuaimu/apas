import { describe, expect, it } from "vitest";

import { parseInboundBridgeMessage, parseOutboundBridgeMessage } from "../src/protocol";

describe("terminal WebView bridge", () => {
  it("accepts only the narrow inbound schema", () => {
    expect(parseInboundBridgeMessage({ type: "output", dataBase64: "YQ==", sequence: 1, instanceId: null })).not.toBeNull();
    expect(parseInboundBridgeMessage({ type: "output", dataBase64: "YQ==", sequence: 1, instanceId: null, accessToken: "secret" })).toBeNull();
    expect(parseInboundBridgeMessage({ type: "paste", text: "x".repeat(1_000_001) })).toBeNull();
  });

  it("rejects unsafe links and malformed resize/input messages", () => {
    expect(parseOutboundBridgeMessage({ type: "link_request", url: "javascript:alert(1)" })).toBeNull();
    expect(parseOutboundBridgeMessage({ type: "link_request", url: "https://example.com/path" })).not.toBeNull();
    expect(parseOutboundBridgeMessage({ type: "resize", cols: 0, rows: 24 })).toBeNull();
    expect(parseOutboundBridgeMessage({ type: "input", data: "ls\r", refresh_token: "secret" })).toBeNull();
  });
});
