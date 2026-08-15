import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore } from "./store";

/**
 * Rebooting from the session list targets a session the client is usually not
 * attached to. These pin the two things that makes fragile: routing by the
 * session that was tapped, and not demanding an inventory that only ever
 * arrives on attach.
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
  useStore.setState({ cliLifecycleInventories: {}, toasts: [] });
});

describe("rebootSessionCli", () => {
  it("routes to the session it was given, not the attached one", async () => {
    const ws = await connected();
    // The attached session is deliberately a different one: routing by
    // "whatever is attached" is how these controls misfire on mobile.
    useStore.setState({ sessionId: "attached-session" });

    useStore.getState().rebootSessionCli("listed-session");

    expect(lastSent(ws)).toMatchObject({
      type: "reboot_cli",
      session_id: "listed-session",
    });
  });

  it("does not require an inventory the list never receives", async () => {
    const ws = await connected();
    // An inventory only reaches a client attached to that session, so from the
    // list it is absent. Refusing here would report "CLI too old" for a CLI
    // that is perfectly current.
    useStore.setState({ sessionId: null, cliLifecycleInventories: {} });

    useStore.getState().rebootSessionCli("listed-session");

    expect(lastSent(ws)).toMatchObject({ type: "reboot_cli" });
    expect(
      useStore.getState().toasts.some((toast) => toast.kind === "error"),
    ).toBe(false);
  });

  it("uses the correlated path when that session's inventory is known", async () => {
    const ws = await connected();
    useStore.setState({
      sessionId: "attached-session",
      cliLifecycleInventories: {
        "listed-session": { panes: [], generatedAt: 1 } as never,
      },
    });

    const requestId = useStore.getState().rebootSessionCli("listed-session");

    // Correlated requests carry a request id so progress stays visible.
    expect(requestId).toBeTruthy();
    expect(lastSent(ws)).toMatchObject({
      type: "cli_lifecycle_request",
      session_id: "listed-session",
      operation: "reboot_cli",
    });
  });

  it("reports rather than pretends when the socket is closed", () => {
    useStore.setState({ ws: null });

    useStore.getState().rebootSessionCli("listed-session");

    expect(
      useStore.getState().toasts.some((toast) => toast.kind === "error"),
    ).toBe(true);
  });
});
