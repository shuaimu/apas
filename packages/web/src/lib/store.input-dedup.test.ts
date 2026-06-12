// Regression tests for duplicate user-input display/storage.
//
// Root cause (2026-06-12, chiral-network session): a stale web connection
// stalled the server's broadcast loop past the client's 3s retransmit
// deadline, so the retry was stored and echoed as a brand-new message.
// The fix threads a client_msg_id through send → server idempotency →
// echo, and these tests lock down the client half: the id rides on every
// send, the echo claims the optimistic slot by id, and a duplicate echo
// of an already-claimed send is dropped instead of appended.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore, type Message, paneKey } from "./store";

const SID_A = "11111111-1111-4111-8111-111111111111";
const PANE_ID = 3;

function makeFakeWs() {
  const sent: string[] = [];
  const ws = {
    readyState: 1, // OPEN
    send: vi.fn((data: string) => { sent.push(data); }),
    close: vi.fn(),
  } as unknown as WebSocket;
  return { ws, sent };
}

function parseSent(sent: string[]): Array<Record<string, unknown>> {
  return sent.map((s) => JSON.parse(s));
}

function dispatch(payload: Record<string, unknown>) {
  const ws = useStore.getState().ws as unknown as { onmessage?: (e: MessageEvent) => void };
  ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify(payload) }));
}

function paneBucket(): Message[] {
  return useStore.getState().paneMessages[paneKey(PANE_ID)] ?? [];
}

beforeEach(() => {
  localStorage.setItem("apas_token", "test-token");
  localStorage.removeItem("apas_pending_sends");
  useStore.setState({
    sessionId: null,
    ws: null,
    isAuthenticated: false,
    connected: false,
    isAttached: false,
    messages: [],
    paneMessages: {},
    paneHasMore: {},
    paneConfigs: [],
    paneModes: {},
    isDualPane: false,
    deadloopMessages: [],
    interactiveMessages: [],
    pendingSends: [],
    sessionCache: new Map(),
    unreadSessions: new Set(),
    sessionLastCreatedAt: new Map(),
    reconnectWatermarks: new Map(),
    answeredQuestions: new Map(),
  });
});

describe("sendMessageToPane carries client_msg_id", () => {
  it("puts the same id on the wire, the optimistic message, and the pending-send entry", () => {
    const { ws, sent } = makeFakeWs();
    useStore.setState({ ws, sessionId: SID_A, isAttached: true, isDualPane: true });

    const result = useStore.getState().sendMessageToPane("hello there", PANE_ID);
    expect(result.success).toBe(true);

    const input = parseSent(sent).find((m) => m.type === "input");
    expect(input).toBeDefined();
    const id = input!.client_msg_id as string;
    expect(id).toBeTruthy();

    const bucket = paneBucket();
    expect(bucket).toHaveLength(1);
    expect(bucket[0].id).toBe(`optimistic-${id}`);

    const pending = useStore.getState().pendingSends;
    expect(pending).toHaveLength(1);
    expect(pending[0].id).toBe(id);
  });
});

describe("user_input echo dedup by client_msg_id", () => {
  function seedOptimisticSend(id: string, text: string) {
    useStore.setState({
      isAuthenticated: true,
      sessionId: SID_A,
      isDualPane: true,
      paneMessages: {
        [paneKey(PANE_ID)]: [{
          id: `optimistic-${id}`,
          role: "user",
          content: text,
          timestamp: new Date(),
          outputType: { type: "text" },
        }],
      },
      pendingSends: [{
        id,
        sessionId: SID_A,
        paneId: PANE_ID,
        paneType: undefined,
        text,
        createdAt: Date.now(),
        attempts: 1,
      }],
    });
  }

  it("claims the optimistic slot by id and a duplicate echo does not append a second copy", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      seedOptimisticSend("send-1", "hello there");

      const echo = {
        type: "user_input",
        session_id: SID_A,
        text: "hello there",
        pane_id: PANE_ID,
        created_at: "2026-06-12T19:30:08Z",
        client_msg_id: "send-1",
      };
      dispatch(echo);

      let bucket = paneBucket();
      expect(bucket).toHaveLength(1);
      expect(bucket[0].id).toBe("send-1"); // prefix stripped, not re-appended
      expect(useStore.getState().pendingSends).toHaveLength(0);

      // The server re-acks retransmits with the same id — must be a no-op.
      dispatch(echo);
      bucket = paneBucket();
      expect(bucket).toHaveLength(1);
      resolve();
    }, 10));
  });

  it("appends once when no optimistic placeholder exists (post-refresh echo)", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      useStore.setState({ isAuthenticated: true, sessionId: SID_A, isDualPane: true });

      dispatch({
        type: "user_input",
        session_id: SID_A,
        text: "typed before refresh",
        pane_id: PANE_ID,
        created_at: "2026-06-12T19:30:08Z",
        client_msg_id: "send-2",
      });

      expect(paneBucket()).toHaveLength(1);
      expect(paneBucket()[0].content).toBe("typed before refresh");
      resolve();
    }, 10));
  });

  it("still claims by content+recency when the echo has no client_msg_id (old server)", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      seedOptimisticSend("send-3", "legacy path");

      dispatch({
        type: "user_input",
        session_id: SID_A,
        text: "legacy path",
        pane_id: PANE_ID,
        created_at: "2026-06-12T19:30:08Z",
      });

      const bucket = paneBucket();
      expect(bucket).toHaveLength(1);
      expect(bucket[0].id).toBe("send-3");
      resolve();
    }, 10));
  });
});
