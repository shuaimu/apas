import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore } from "./store";

/**
 * Rebooting a daemon targets a machine, not a project on it, and the client is
 * usually attached to neither. These pin that routing.
 */

type MockWs = {
  send: ReturnType<typeof vi.fn>;
  onmessage?: (event: MessageEvent) => void;
};

async function connected(): Promise<MockWs> {
  localStorage.setItem("apas_token", "test-token");
  useStore.getState().connect();
  await new Promise((resolve) => setTimeout(resolve, 10));
  return useStore.getState().ws as unknown as MockWs;
}

function lastSent(ws: MockWs): Record<string, unknown> {
  const calls = ws.send.mock.calls;
  return JSON.parse(String(calls[calls.length - 1][0]));
}

beforeEach(() => {
  useStore.setState({ toasts: [] });
});

describe("rebootDaemon", () => {
  it("targets the machine it was given", async () => {
    const ws = await connected();
    // A daemon is per-machine: no project on it identifies the right one.
    useStore.setState({ sessionId: "some-session" });

    useStore.getState().rebootDaemon("machine-b");

    expect(lastSent(ws)).toMatchObject({
      type: "reboot_daemon",
      machine_id: "machine-b",
    });
  });

  it("reports rather than pretends when the socket is closed", () => {
    useStore.setState({ ws: null });

    useStore.getState().rebootDaemon("machine-b");

    expect(
      useStore.getState().toasts.some((toast) => toast.kind === "error"),
    ).toBe(true);
  });
});
