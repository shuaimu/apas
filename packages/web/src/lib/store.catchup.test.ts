// Regression tests for the WS-reconnect / tab-switch catchup machinery.
//
// Why this file exists separately: store.test.ts covers basic message
// add/clear plumbing; the watermark + catchup flow is gnarlier (stale
// IDB hydration, reconnect snapshotting, background-tab updates) and
// has regressed several times in a week, so we lock the contract down
// with focused tests rather than relying on someone to remember the
// invariants.
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  useStore,
  type Message,
  type SessionCacheEntry,
  type PaneConfig,
  paneKey,
} from "./store";

const SID_A = "11111111-1111-4111-8111-111111111111";
const SID_B = "22222222-2222-4222-8222-222222222222";
const PANE_ID = 3;

function makeMsg(id: string, content = "hi"): Message {
  return { id, role: "assistant", content, timestamp: new Date(), outputType: { type: "text" } };
}

function makeStoredMsg(id: string, paneId: number, content = "hi") {
  return {
    id,
    role: "assistant",
    content,
    message_type: "text",
    pane_id: paneId,
    created_at: "2026-06-16T12:00:00Z",
  };
}

function makeCachedEntry(opts: {
  messages?: Message[];
  paneMessages?: Record<string, Message[]>;
  paneConfigs?: PaneConfig[];
  paneModes?: Record<string, "deadloop" | "interactive">;
  lastCreatedAt?: string;
} = {}): SessionCacheEntry {
  return {
    messages: opts.messages ?? [],
    paneMessages: opts.paneMessages ?? {},
    paneHasMore: {},
    paneConfigs: opts.paneConfigs ?? [],
    paneModes: opts.paneModes ?? {},
    hasMoreMessages: false,
    isDualPane: false,
    answeredQuestions: new Map(),
    cachedAt: Date.now(),
    lastCreatedAt: opts.lastCreatedAt,
  };
}

function makePaneConfig(mode: "deadloop" | "interactive"): PaneConfig {
  return {
    pane_id: PANE_ID,
    provider: "claude",
    mode,
    session_id: `pane-${PANE_ID}`,
    is_paused: false,
    label: "Worker",
  };
}

/** Minimal fake WS the store can talk to. Captures `send` payloads. */
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

beforeEach(() => {
  // connect() bails when no token is present in localStorage; tests that
  // exercise the WS-onmessage path call connect() and need a token to
  // actually wire up the socket.
  localStorage.setItem("apas_token", "test-token");
  useStore.setState({
    sessionId: null,
    ws: null,
    isAuthenticated: false,
    connected: false,
    messages: [],
    paneMessages: {},
    paneHasMore: {},
    paneConfigs: [],
    paneModes: {},
    pausedPanes: [],
    paneStatuses: {},
    isDualPane: false,
    deadloopMessages: [],
    interactiveMessages: [],
    sessions: [],
    cliClients: [],
    machines: [],
    sessionCache: new Map(),
    unreadSessions: new Set(),
    sessionLastCreatedAt: new Map(),
    paneLastCreatedAt: new Map(),
    paneLoadingInitial: new Set(),
    reconnectWatermarks: new Map(),
    answeredQuestions: new Map(),
    pendingSends: [],
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("PaneList-authoritative pane modes", () => {
  it("keeps a fresh interactive PaneList mode over replayed deadloop message and status hints", async () => {
    useStore.getState().connect();
    await new Promise((resolve) => setTimeout(resolve, 10));
    useStore.setState({
      isAuthenticated: true,
      sessionId: SID_A,
      isDualPane: true,
    });

    dispatch({
      type: "session_messages",
      session_id: SID_A,
      messages: [{
        ...makeStoredMsg("old-deadloop-1", PANE_ID, "historical bot output"),
        pane_type: "deadloop",
      }],
      has_more: false,
    });

    expect(useStore.getState().paneModes[paneKey(PANE_ID)]).toBe("deadloop");

    dispatch({
      type: "pane_list",
      session_id: SID_A,
      panes: [makePaneConfig("interactive")],
    });

    expect(useStore.getState().paneConfigs[0]?.mode).toBe("interactive");
    expect(useStore.getState().paneModes[paneKey(PANE_ID)]).toBe("interactive");

    dispatch({
      type: "session_messages",
      session_id: SID_A,
      messages: [{
        ...makeStoredMsg("old-deadloop-2", PANE_ID, "another historical bot output"),
        pane_type: "deadloop",
      }],
      has_more: false,
    });
    dispatch({
      type: "pane_status",
      session_id: SID_A,
      pane_id: PANE_ID,
      pane_type: "deadloop",
      status: "replayed stale bot status",
    });

    const state = useStore.getState();
    expect(state.paneConfigs[0]?.mode).toBe("interactive");
    expect(state.paneModes[paneKey(PANE_ID)]).toBe("interactive");
    expect(state.paneStatuses[PANE_ID]).toBe("replayed stale bot status");
  });
});

describe("lazy per-pane initial message load", () => {
  it("caps the initial all-pane session load to the newest 30 messages per pane", () => {
    const { ws, sent } = makeFakeWs();
    useStore.setState({
      ws,
      sessionId: SID_B,
      messages: [makeMsg("existing-session")],
      paneMessages: { [paneKey(PANE_ID)]: [makeMsg("existing-pane")] },
      paneHasMore: { [paneKey(PANE_ID)]: true },
      paneStatuses: { [PANE_ID]: "running" },
      paneModes: { [paneKey(PANE_ID)]: "interactive" },
      paneConfigs: [makePaneConfig("interactive")],
    });

    useStore.getState().loadSessionMessages(SID_A);

    const requests = parseSent(sent).filter((m) => m.type === "get_session_messages");
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({
      session_id: SID_A,
      limit: 30,
    });
    expect(requests[0]).not.toHaveProperty("pane_id");
    expect(useStore.getState().messages).toEqual([]);
    expect(useStore.getState().paneMessages).toEqual({});
    expect(useStore.getState().sessionId).toBe(SID_A);
  });

  it("requests the active pane once, seeds its bucket, and suppresses duplicates while in flight", () => {
    vi.useFakeTimers();
    const { ws, sent } = makeFakeWs();
    useStore.setState({ ws, sessionId: SID_A });

    useStore.getState().loadPaneMessagesIfNeeded(PANE_ID);
    useStore.getState().loadPaneMessagesIfNeeded(PANE_ID);

    const requests = parseSent(sent).filter((m) => m.type === "get_session_messages");
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({
      session_id: SID_A,
      pane_id: PANE_ID,
      limit: 30,
    });
    expect(useStore.getState().paneMessages[paneKey(PANE_ID)]).toEqual([]);
    expect(useStore.getState().paneLoadingInitial.has(PANE_ID)).toBe(true);
  });

  it("applies a pane-filtered response only to that pane and clears its in-flight marker", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      const otherPaneId = 9;
      const otherMsg = makeMsg("other-existing", "do not replace me");
      useStore.setState({
        isAuthenticated: true,
        sessionId: SID_A,
        isDualPane: true,
        paneMessages: {
          [paneKey(PANE_ID)]: [],
          [paneKey(otherPaneId)]: [otherMsg],
        },
        paneLoadingInitial: new Set([PANE_ID, otherPaneId]),
      });

      dispatch({
        type: "session_messages",
        session_id: SID_A,
        messages: [makeStoredMsg("target-1", PANE_ID, "target pane history")],
        has_more: true,
      });

      const state = useStore.getState();
      expect(state.paneMessages[paneKey(PANE_ID)]).toHaveLength(1);
      expect(state.paneMessages[paneKey(PANE_ID)]?.[0].content).toBe("target pane history");
      expect(state.paneMessages[paneKey(otherPaneId)]).toEqual([otherMsg]);
      expect(state.paneHasMore[paneKey(PANE_ID)]).toBe(true);
      expect(state.paneLoadingInitial.has(PANE_ID)).toBe(false);
      expect(state.paneLoadingInitial.has(otherPaneId)).toBe(true);
      resolve();
    }, 10));
  });

  it("keeps empty pane responses safe until the timeout fallback clears loading", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      vi.useFakeTimers();
      useStore.setState({
        isAuthenticated: true,
        sessionId: SID_A,
        isDualPane: true,
      });

      useStore.getState().loadPaneMessagesIfNeeded(PANE_ID);
      dispatch({
        type: "session_messages",
        session_id: SID_A,
        messages: [],
        has_more: false,
      });

      expect(useStore.getState().paneMessages[paneKey(PANE_ID)]).toEqual([]);
      expect(useStore.getState().paneLoadingInitial.has(PANE_ID)).toBe(true);

      vi.advanceTimersByTime(30_000);

      expect(useStore.getState().paneLoadingInitial.has(PANE_ID)).toBe(false);
      resolve();
    }, 10));
  });
});

describe("attachSession + catchup query", () => {
  it("fires get_session_messages with sessionLastCreatedAt when switching to a cached tab", () => {
    const { ws, sent } = makeFakeWs();
    useStore.setState({
      ws,
      sessionId: SID_A,
      // Some non-empty live state so the snapshot-on-leave path runs.
      messages: [makeMsg("a1")],
      sessionCache: new Map([[SID_B, makeCachedEntry({
        paneMessages: { [paneKey(PANE_ID)]: [makeMsg("b1")] },
        lastCreatedAt: "2026-05-26T00:00:00Z",
      })]]),
      sessionLastCreatedAt: new Map([[SID_B, "2026-05-26T00:00:00Z"]]),
      sessions: [{ id: SID_B, status: "active" }],
    });

    useStore.getState().attachSession(SID_B, false);

    const messages = parseSent(sent);
    const attach = messages.find((m) => m.type === "attach_session");
    const catchup = messages.find((m) => m.type === "get_session_messages");
    expect(attach).toMatchObject({ session_id: SID_B });
    expect(catchup).toMatchObject({
      session_id: SID_B,
      after_created_at: "2026-05-26T00:00:00Z",
    });
  });

  it("prefers reconnectWatermarks over sessionLastCreatedAt (frozen pre-disconnect value wins)", () => {
    const { ws, sent } = makeFakeWs();
    useStore.setState({
      ws,
      sessionId: SID_A,
      messages: [makeMsg("a1")],
      sessionCache: new Map([[SID_B, makeCachedEntry({
        paneMessages: { [paneKey(PANE_ID)]: [makeMsg("b1")] },
      })]]),
      // Live watermark has advanced past the disconnect-window messages.
      sessionLastCreatedAt: new Map([[SID_B, "2026-05-26T12:00:00Z"]]),
      // Frozen at reconnect — older, covers the gap.
      reconnectWatermarks: new Map([[SID_B, "2026-05-25T18:00:00Z"]]),
      sessions: [{ id: SID_B, status: "active" }],
    });

    useStore.getState().attachSession(SID_B, false);

    const catchup = parseSent(sent).find((m) => m.type === "get_session_messages");
    expect(catchup?.after_created_at).toBe("2026-05-25T18:00:00Z");
  });

  it("does not fire catchup when target tab has no cache (first visit)", () => {
    const { ws, sent } = makeFakeWs();
    useStore.setState({
      ws,
      sessionId: SID_A,
      messages: [makeMsg("a1")],
      // No cache entry for SID_B; just a watermark from prior stream messages.
      sessionLastCreatedAt: new Map([[SID_B, "2026-05-26T00:00:00Z"]]),
      sessions: [{ id: SID_B, status: "active" }],
    });

    useStore.getState().attachSession(SID_B, false);

    const sentMsgs = parseSent(sent);
    expect(sentMsgs.some((m) => m.type === "attach_session")).toBe(true);
    // Initial-load path will repopulate empty paneMessages — no catchup needed.
    expect(sentMsgs.some((m) => m.type === "get_session_messages")).toBe(false);
  });
});

describe("snapshot-on-leave carries the catchup watermark", () => {
  it("captures sessionLastCreatedAt into the snapshot's lastCreatedAt", () => {
    const { ws } = makeFakeWs();
    useStore.setState({
      ws,
      sessionId: SID_A,
      // Live state non-empty so the snapshot path runs (otherwise
      // attachSession skips caching, see the >0 length guard).
      paneMessages: { [paneKey(PANE_ID)]: [makeMsg("a1")] },
      sessionLastCreatedAt: new Map([[SID_A, "2026-05-26T10:00:00Z"]]),
      sessions: [{ id: SID_B, status: "active" }],
    });

    useStore.getState().attachSession(SID_B, false);

    const snapshot = useStore.getState().sessionCache.get(SID_A);
    expect(snapshot).toBeDefined();
    expect(snapshot?.lastCreatedAt).toBe("2026-05-26T10:00:00Z");
  });
});

describe("background stream_message keeps the cached watermark fresh", () => {
  // Verifies the bug where a user kept a tab open while another tab
  // accumulated messages — without this, the IDB snapshot kept the
  // pre-stream lastCreatedAt and a future hydration would silently miss
  // the streamed-while-backgrounded tail.
  function dispatchStreamMessage(payload: Record<string, unknown>) {
    const ws = useStore.getState().ws as unknown as { onmessage?: (e: MessageEvent) => void };
    ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify(payload) }));
  }

  it("bumps cache.lastCreatedAt when an assistant text streams into a background session", () => {
    // Need a real connect() so the WS onmessage handler is wired up.
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      useStore.setState({
        isAuthenticated: true,
        sessionId: SID_A,
        sessionCache: new Map([[SID_B, makeCachedEntry({
          paneMessages: { [paneKey(PANE_ID)]: [makeMsg("b1")] },
          lastCreatedAt: "2026-05-26T08:00:00Z",
        })]]),
      });
      dispatchStreamMessage({
        type: "stream_message",
        session_id: SID_B,
        created_at: "2026-05-26T09:00:00Z",
        pane_id: PANE_ID,
        message: {
          type: "assistant",
          message: { content: [{ type: "text", text: "new bg msg" }] },
        },
      });
      const entry = useStore.getState().sessionCache.get(SID_B);
      expect(entry?.lastCreatedAt).toBe("2026-05-26T09:00:00Z");
      // And the streamed message landed in the cache, not the live state.
      const cachedMsgs = entry?.paneMessages[paneKey(PANE_ID)] ?? [];
      expect(cachedMsgs).toHaveLength(2);
      expect(cachedMsgs[1].content).toBe("new bg msg");
      // Live state for the current (A) session is untouched.
      expect(useStore.getState().paneMessages[paneKey(PANE_ID)] ?? []).toHaveLength(0);
      resolve();
    }, 10));
  });

  it("does not regress lastCreatedAt when an older stream_message arrives out of order", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      useStore.setState({
        isAuthenticated: true,
        sessionId: SID_A,
        sessionCache: new Map([[SID_B, makeCachedEntry({
          paneMessages: { [paneKey(PANE_ID)]: [] },
          lastCreatedAt: "2026-05-26T10:00:00Z",
        })]]),
      });
      dispatchStreamMessage({
        type: "stream_message",
        session_id: SID_B,
        created_at: "2026-05-26T09:00:00Z", // older than existing
        pane_id: PANE_ID,
        message: {
          type: "assistant",
          message: { content: [{ type: "text", text: "stale" }] },
        },
      });
      const entry = useStore.getState().sessionCache.get(SID_B);
      expect(entry?.lastCreatedAt).toBe("2026-05-26T10:00:00Z");
      resolve();
    }, 10));
  });
});

describe("IDB hydration preserves live PaneList modes", () => {
  it("restores cached messages without clobbering live pane configs or modes", () => {
    const livePane = makePaneConfig("interactive");
    const stalePane = makePaneConfig("deadloop");
    const cached = makeCachedEntry({
      paneMessages: { [paneKey(PANE_ID)]: [makeMsg("cached-1", "cached history")] },
      paneConfigs: [stalePane],
      paneModes: { [paneKey(PANE_ID)]: "deadloop" },
    });

    useStore.setState({
      sessionId: SID_A,
      messages: [],
      paneMessages: {},
      paneConfigs: [livePane],
      paneModes: { [paneKey(PANE_ID)]: "interactive" },
    });

    // Mirrors the same-session restore branch that runs after
    // loadAllSnapshotsIdb() resolves. The live PaneList may already have
    // arrived before disk messages hydrate, so paneConfigs/paneModes win.
    const state = useStore.getState();
    const isEmpty =
      state.messages.length === 0 &&
      Object.keys(state.paneMessages).length === 0;
    if (isEmpty) {
      useStore.setState({
        messages: cached.messages,
        paneMessages: cached.paneMessages,
        paneHasMore: cached.paneHasMore,
        paneConfigs:
          state.paneConfigs.length > 0
            ? state.paneConfigs
            : cached.paneConfigs,
        paneModes:
          Object.keys(state.paneModes).length > 0
            ? state.paneModes
            : cached.paneModes,
        hasMoreMessages: cached.hasMoreMessages,
        isDualPane: cached.isDualPane,
        answeredQuestions: cached.answeredQuestions,
        deadloopMessages: cached.paneMessages[paneKey(1)] ?? [],
        interactiveMessages: cached.paneMessages[paneKey(2)] ?? [],
      });
    }

    const hydrated = useStore.getState();
    expect(hydrated.paneMessages[paneKey(PANE_ID)]?.[0].content).toBe("cached history");
    expect(hydrated.paneConfigs).toEqual([livePane]);
    expect(hydrated.paneModes[paneKey(PANE_ID)]).toBe("interactive");
  });
});

describe("IDB hydration seeds sessionLastCreatedAt", () => {
  // Mirrors the setState block inside loadAllSnapshotsIdb().then(...)
  // (extracted by mimicking its logic) — the actual hydration runs at
  // module load and isn't easily re-triggerable per test.
  it("populates sessionLastCreatedAt from each hydrated entry's lastCreatedAt", () => {
    const diskCache = new Map<string, SessionCacheEntry>([
      [SID_A, makeCachedEntry({ lastCreatedAt: "2026-05-26T08:00:00Z" })],
      [SID_B, makeCachedEntry({ lastCreatedAt: "2026-05-26T10:00:00Z" })],
    ]);

    useStore.setState((state) => {
      const merged = new Map(diskCache);
      for (const [k, v] of state.sessionCache) merged.set(k, v);
      const seededLast = new Map(state.sessionLastCreatedAt);
      for (const [k, v] of diskCache) {
        if (!v.lastCreatedAt) continue;
        const existing = seededLast.get(k);
        if (!existing || v.lastCreatedAt > existing) seededLast.set(k, v.lastCreatedAt);
      }
      return { sessionCache: merged, sessionLastCreatedAt: seededLast };
    });

    const state = useStore.getState();
    expect(state.sessionLastCreatedAt.get(SID_A)).toBe("2026-05-26T08:00:00Z");
    expect(state.sessionLastCreatedAt.get(SID_B)).toBe("2026-05-26T10:00:00Z");
  });

  it("does not overwrite a fresher in-memory sessionLastCreatedAt", () => {
    useStore.setState({
      sessionLastCreatedAt: new Map([[SID_A, "2026-05-26T12:00:00Z"]]),
    });
    const diskCache = new Map<string, SessionCacheEntry>([
      [SID_A, makeCachedEntry({ lastCreatedAt: "2026-05-26T08:00:00Z" })],
    ]);

    useStore.setState((state) => {
      const seededLast = new Map(state.sessionLastCreatedAt);
      for (const [k, v] of diskCache) {
        if (!v.lastCreatedAt) continue;
        const existing = seededLast.get(k);
        if (!existing || v.lastCreatedAt > existing) seededLast.set(k, v.lastCreatedAt);
      }
      return { sessionLastCreatedAt: seededLast };
    });

    expect(useStore.getState().sessionLastCreatedAt.get(SID_A)).toBe("2026-05-26T12:00:00Z");
  });
});

describe("catchup reply clears reconnectWatermarks for that session", () => {
  it("removes the per-session frozen watermark after a current-session catchup reply lands", () => {
    useStore.getState().connect();
    return new Promise<void>((resolve) => setTimeout(() => {
      useStore.setState({
        isAuthenticated: true,
        sessionId: SID_A,
        reconnectWatermarks: new Map([
          [SID_A, "2026-05-25T18:00:00Z"],
          [SID_B, "2026-05-25T18:00:00Z"],
        ]),
      });
      const ws = useStore.getState().ws as unknown as { onmessage?: (e: MessageEvent) => void };
      ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify({
        type: "session_messages",
        session_id: SID_A,
        catchup: true,
        messages: [],
        has_more: false,
      }) }));
      const state = useStore.getState();
      expect(state.reconnectWatermarks.has(SID_A)).toBe(false);
      // SID_B's frozen watermark is untouched.
      expect(state.reconnectWatermarks.get(SID_B)).toBe("2026-05-25T18:00:00Z");
      resolve();
    }, 10));
  });
});
