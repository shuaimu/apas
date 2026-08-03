import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore } from "./store";

/**
 * Creating an instance makes the daemon clone a repo before it acks, which can
 * take tens of seconds. These cover the local placeholder that gives the user
 * something to look at in the meantime.
 */

type StoreState = ReturnType<typeof useStore.getState>;

type MockWs = {
  send: ReturnType<typeof vi.fn>;
  onmessage?: (event: MessageEvent) => void;
};

/** Connect through the global WebSocket mock and return the live socket. */
async function connected(): Promise<MockWs> {
  // `connect()` reads the token from localStorage, not store state — without
  // it the call bails with "No token found" and leaves `ws` null.
  localStorage.setItem("apas_token", "test-token");
  useStore.getState().connect();
  await new Promise((resolve) => setTimeout(resolve, 10));
  return useStore.getState().ws as unknown as MockWs;
}

function deliver(ws: MockWs, message: Record<string, unknown>) {
  ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify(message) }));
}

function lastSent(ws: MockWs): Record<string, unknown> {
  const calls = ws.send.mock.calls;
  return JSON.parse(String(calls[calls.length - 1][0]));
}

beforeEach(() => {
  useStore.setState({ pendingInstances: {} });
});

describe("pending instance creation", () => {
  it("records a placeholder and reports it immediately, before any ack", async () => {
    const ws = await connected();
    const showToast = vi.fn();
    useStore.setState({ showToast: showToast as StoreState["showToast"] });

    const ok = useStore
      .getState()
      .createProjectInstance("machine-1", "github.com/acme/repo", "my-instance", "apas/x");

    expect(ok).toBe(true);
    // Feedback has to happen on send. The whole complaint was that nothing
    // visible happened until the daemon finished cloning.
    expect(showToast).toHaveBeenCalledTimes(1);
    expect(String(showToast.mock.calls[0][0])).toContain("my-instance");

    const pending = Object.values(useStore.getState().pendingInstances);
    expect(pending).toHaveLength(1);
    expect(pending[0].instanceName).toBe("my-instance");
    expect(pending[0].machineId).toBe("machine-1");
    expect(pending[0].startedAt).toBeGreaterThan(0);

    // And it really did go out on the wire, under the same request id.
    const sent = lastSent(ws);
    expect(sent.type).toBe("create_project_instance");
    expect(sent.request_id).toBe(pending[0].requestId);
  });

  it("clears the placeholder when its ack arrives", async () => {
    const ws = await connected();
    useStore.setState({
      showToast: vi.fn() as StoreState["showToast"],
      listMachines: vi.fn() as StoreState["listMachines"],
    });
    useStore.getState().createProjectInstance("m1", "github.com/a/b", "inst", "apas/x");
    const [{ requestId }] = Object.values(useStore.getState().pendingInstances);

    deliver(ws, {
      type: "project_instance_created",
      machine_id: "m1",
      request_id: requestId,
      project_id: "proj-1",
    });

    expect(useStore.getState().pendingInstances).toEqual({});
  });

  it("clears only the acked creation when several overlap", async () => {
    // Clearing the whole map on any ack would strand the other creations as
    // permanent spinners, which is worse than the bug being fixed.
    const ws = await connected();
    useStore.setState({
      showToast: vi.fn() as StoreState["showToast"],
      listMachines: vi.fn() as StoreState["listMachines"],
    });
    useStore.getState().createProjectInstance("m1", "github.com/a/b", "first", "apas/x");
    useStore.getState().createProjectInstance("m1", "github.com/a/c", "second", "apas/y");

    const all = Object.values(useStore.getState().pendingInstances);
    expect(all).toHaveLength(2);
    const first = all.find((p) => p.instanceName === "first")!;

    deliver(ws, {
      type: "project_instance_created",
      machine_id: "m1",
      request_id: first.requestId,
      project_id: "proj-1",
    });

    const left = Object.values(useStore.getState().pendingInstances);
    expect(left).toHaveLength(1);
    expect(left[0].instanceName).toBe("second");
  });

  it("clears the placeholder on failure too, and names it in the error", async () => {
    const ws = await connected();
    const showToast = vi.fn();
    useStore.setState({ showToast: showToast as StoreState["showToast"] });
    useStore.getState().createProjectInstance("m1", "github.com/a/b", "doomed", "apas/x");
    const [{ requestId }] = Object.values(useStore.getState().pendingInstances);
    showToast.mockClear();

    deliver(ws, {
      type: "project_instance_created",
      machine_id: "m1",
      request_id: requestId,
      error: "clone failed: auth",
    });

    // A stuck spinner after a failure would be the worst outcome — the user
    // would wait forever on something already dead.
    expect(useStore.getState().pendingInstances).toEqual({});
    const [msg, level] = showToast.mock.calls[0];
    expect(String(msg)).toContain("doomed");
    expect(String(msg)).toContain("clone failed: auth");
    expect(level).toBe("error");
  });

  it("does not record a placeholder when the send is dropped", () => {
    useStore.setState({
      ws: { readyState: 3, send: vi.fn() } as unknown as WebSocket,
      showToast: vi.fn() as StoreState["showToast"],
    });

    const ok = useStore
      .getState()
      .createProjectInstance("m1", "github.com/a/b", "never-sent", "apas/x");

    expect(ok).toBe(false);
    expect(useStore.getState().pendingInstances).toEqual({});
  });
});
