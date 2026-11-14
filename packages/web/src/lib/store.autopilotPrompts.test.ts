import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage, validateServerMessage } from "@apas/protocol";
import { useStore } from "./store";
import {
  autopilotPromptChangeError,
  autopilotPromptsRequestText,
  PROJECT_AUTOPILOT_PROMPTS_CAPABILITY,
  type AutopilotPromptsRequest,
} from "./autopilotPrompts";

/**
 * A project's saved autopilot prompts in the store: the list each project
 * reports, and changes that are sent once and never again. These drive the
 * real socket handler with the protocol's golden frames, and check every
 * frame the store sends against the schema the server parses.
 */

const SID = "20000000-0000-4000-8000-000000000001";
const OTHER_SID = "20000000-0000-4000-8000-000000000002";

type MockWs = {
  send: Mock;
  onmessage?: (event: MessageEvent) => void;
  onclose?: (event: CloseEvent) => void;
};

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8"));
}

async function openSocket(): Promise<MockWs> {
  localStorage.setItem("apas_token", "test-token");
  useStore.setState({ ws: null });
  useStore.getState().connect();
  await new Promise((done) => setTimeout(done, 10));
  return useStore.getState().ws as unknown as MockWs;
}

function deliver(ws: MockWs, message: Record<string, unknown>) {
  ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify(message) }));
}

function sentFrames(ws: MockWs, type?: string): Record<string, unknown>[] {
  return ws.send.mock.calls
    .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
    .filter((frame) => !type || frame.type === type);
}

function negotiate(capable = true) {
  useStore.setState({
    negotiatedCapabilities: new Set(capable ? [PROJECT_AUTOPILOT_PROMPTS_CAPABILITY] : []),
  });
}

function requests(): AutopilotPromptsRequest[] {
  return Object.values(useStore.getState().autopilotPromptRequests);
}

function receipt(requestId: string, outcome: string, reason?: string): Record<string, unknown> {
  return { type: "autopilot_prompts_receipt", session_id: SID, request_id: requestId, outcome, reason };
}

const SAVE = { kind: "save", name: "Nightly review", prompt: "Review the open pull requests.", replace: false } as const;

const initialState = useStore.getState();

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  useStore.setState(initialState, true);
  useStore.setState({ sessionId: SID, isAttached: true, projectAutopilotPrompts: {}, autopilotPromptRequests: {}, toasts: [] });
});

afterEach(() => {
  vi.useRealTimers();
  useStore.getState().disconnect();
});

describe("negotiation", () => {
  it("advertises project_autopilot_prompts_v1", async () => {
    const ws = await openSocket();
    const [authenticate] = sentFrames(ws, "authenticate");
    expect(authenticate.capabilities).toContain(PROJECT_AUTOPILOT_PROMPTS_CAPABILITY);
    expect(validateClientMessage(authenticate).valid).toBe(true);
  });

  it("sends nothing to a server that did not negotiate saved prompts", async () => {
    const ws = await openSocket();
    negotiate(false);
    const result = useStore.getState().updateAutopilotPrompts(SID, SAVE);
    expect(result).toEqual({ success: false, error: "This APAS server cannot save prompts." });
    expect(sentFrames(ws, "update_autopilot_prompts")).toEqual([]);
    expect(requests()).toEqual([]);
  });

  it("ignores a list from a server that did not negotiate saved prompts", async () => {
    const ws = await openSocket();
    negotiate(false);
    deliver(ws, fixture("server-project-autopilot-prompts.json"));
    expect(useStore.getState().projectAutopilotPrompts).toEqual({});
  });
});

describe("the project's list", () => {
  it("holds each session's list, and each frame replaces it whole", async () => {
    const ws = await openSocket();
    negotiate();
    const frame = fixture("server-project-autopilot-prompts.json");
    expect(validateServerMessage(frame).valid).toBe(true);
    deliver(ws, frame);

    expect(useStore.getState().projectAutopilotPrompts[SID]).toEqual({
      prompts: frame.prompts,
      default: "Nightly review",
      unreadable: false,
      received: true,
    });

    // The default is cleared and one prompt deleted: the next frame says so
    // by leaving them out, and nothing of the earlier list survives.
    deliver(ws, {
      type: "project_autopilot_prompts",
      session_id: SID,
      prompts: [(frame.prompts as unknown[])[1]],
    });
    expect(useStore.getState().projectAutopilotPrompts[SID]).toEqual({
      prompts: [(frame.prompts as unknown[])[1]],
      unreadable: false,
      received: true,
    });

    // An empty list is a project with none, which is known.
    deliver(ws, { type: "project_autopilot_prompts", session_id: SID, prompts: [] });
    expect(useStore.getState().projectAutopilotPrompts[SID]).toEqual({ prompts: [], unreadable: false, received: true });
  });

  it("keeps sessions apart and reports an unreadable list as such", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-project-autopilot-prompts.json"));
    const unreadable = { ...fixture("server-project-autopilot-prompts-unreadable.json"), session_id: OTHER_SID };
    expect(validateServerMessage(unreadable).valid).toBe(true);
    deliver(ws, unreadable);

    expect(useStore.getState().projectAutopilotPrompts[SID]?.prompts).toHaveLength(2);
    expect(useStore.getState().projectAutopilotPrompts[OTHER_SID]).toEqual({
      prompts: [],
      unreadable: true,
      received: true,
    });
  });

  it("drops every list with the socket, to be replayed on attach", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-project-autopilot-prompts.json"));
    ws.onclose?.(new CloseEvent("close", { code: 1000 }));
    expect(useStore.getState().projectAutopilotPrompts).toEqual({});
  });
});

describe("a change", () => {
  it("is sent once, as one schema-valid frame, and tracked until its receipt", async () => {
    const ws = await openSocket();
    negotiate();

    const result = useStore.getState().updateAutopilotPrompts(SID, SAVE);
    expect(result.success).toBe(true);
    const frames = sentFrames(ws, "update_autopilot_prompts");
    expect(frames).toEqual([{ type: "update_autopilot_prompts", session_id: SID, request_id: result.requestId, change: SAVE }]);
    expect(validateClientMessage(frames[0]).valid).toBe(true);
    expect(requests()).toMatchObject([{ requestId: result.requestId, sessionId: SID, change: SAVE, status: "pending" }]);

    // Every change kind the dialog sends is one the server parses.
    for (const change of [
      { kind: "delete", name: "Nightly review" },
      { kind: "set_default", name: "Nightly review" },
      { kind: "set_default", name: null },
    ] as const) {
      useStore.getState().updateAutopilotPrompts(SID, change);
    }
    for (const frame of sentFrames(ws, "update_autopilot_prompts")) {
      expect(validateClientMessage(frame).valid).toBe(true);
    }
    expect(sentFrames(ws, "update_autopilot_prompts")).toHaveLength(4);
  });

  it("is settled by its own receipt only", async () => {
    const ws = await openSocket();
    negotiate();
    const saved = useStore.getState().updateAutopilotPrompts(SID, SAVE).requestId!;
    const refused = useStore.getState().updateAutopilotPrompts(SID, { ...SAVE, name: "Release check" }).requestId!;

    // Someone else's receipt, or one naming another session, settles nothing.
    deliver(ws, receipt("someone-else", "accepted"));
    deliver(ws, { ...receipt(saved, "accepted"), session_id: OTHER_SID });
    expect(requests().every((request) => request.status === "pending")).toBe(true);

    deliver(ws, receipt(saved, "accepted"));
    const refusal = { ...fixture("server-autopilot-prompts-receipt.json"), request_id: refused };
    expect(validateServerMessage(refusal).valid).toBe(true);
    deliver(ws, refusal);

    const state = useStore.getState().autopilotPromptRequests;
    expect(state[saved]).toMatchObject({ status: "saved" });
    expect(autopilotPromptsRequestText(state[saved])).toBe("Saved to the project as \"Nightly review\".");
    expect(state[refused]).toMatchObject({ status: "refused", reason: "A saved prompt named \"Nightly review\" already exists." });
    expect(autopilotPromptsRequestText(state[refused])).toBe("A saved prompt named \"Nightly review\" already exists.");

    // A second receipt for an answered change changes nothing.
    deliver(ws, receipt(saved, "refused", "late"));
    expect(useStore.getState().autopilotPromptRequests[saved]).toMatchObject({ status: "saved" });
    // Nothing was sent again.
    expect(sentFrames(ws, "update_autopilot_prompts")).toHaveLength(2);
  });

  it("is reported as not confirmed after a timeout or an uncertain receipt, and never resent", async () => {
    vi.useFakeTimers();
    const ws = await (async () => {
      localStorage.setItem("apas_token", "test-token");
      useStore.setState({ ws: null });
      useStore.getState().connect();
      await vi.advanceTimersByTimeAsync(10);
      return useStore.getState().ws as unknown as MockWs;
    })();
    negotiate();
    const timedOut = useStore.getState().updateAutopilotPrompts(SID, SAVE).requestId!;
    const uncertain = useStore.getState().updateAutopilotPrompts(SID, { kind: "delete", name: "Old" }).requestId!;
    deliver(ws, receipt(uncertain, "uncertain"));

    await vi.advanceTimersByTimeAsync(15_000);
    const state = useStore.getState().autopilotPromptRequests;
    expect(state[timedOut]).toMatchObject({ status: "unconfirmed", cause: "timeout" });
    expect(state[uncertain]).toMatchObject({ status: "unconfirmed", cause: "uncertain" });
    expect(autopilotPromptsRequestText(state[timedOut])).toMatch(/^Not confirmed\. .*It was not sent again/);

    // A late answer still says what happened.
    deliver(ws, receipt(timedOut, "accepted"));
    expect(useStore.getState().autopilotPromptRequests[timedOut]).toMatchObject({ status: "saved" });

    await vi.advanceTimersByTimeAsync(60_000);
    expect(sentFrames(ws, "update_autopilot_prompts")).toHaveLength(2);
  });

  it("is reported as not confirmed when its connection closes, and the next one sends nothing again", async () => {
    const ws = await openSocket();
    negotiate();
    const requestId = useStore.getState().updateAutopilotPrompts(SID, SAVE).requestId!;
    ws.onclose?.(new CloseEvent("close", { code: 1000 }));
    expect(useStore.getState().autopilotPromptRequests[requestId]).toMatchObject({ status: "unconfirmed", cause: "connection" });

    const next = await openSocket();
    deliver(next, { type: "authenticated", user_id: "u1", negotiated_capabilities: [PROJECT_AUTOPILOT_PROMPTS_CAPABILITY] });
    expect(sentFrames(next, "update_autopilot_prompts")).toEqual([]);
  });

  it("is refused here, with nothing queued, while the page is offline", async () => {
    negotiate();
    useStore.setState({ ws: null });
    expect(useStore.getState().updateAutopilotPrompts(SID, SAVE)).toEqual({
      success: false,
      error: "Not connected to the server.",
    });
    expect(requests()).toEqual([]);
  });
});

describe("the early check", () => {
  const LIST = [{ name: "Nightly review", prompt: "Review.", updated_at: "2026-11-14T09:00:00Z" }];

  it("uses the Rust refusal texts", () => {
    expect(autopilotPromptChangeError({ kind: "save", name: "  ", prompt: "x" }, LIST)).toBe("Give the saved prompt a name.");
    expect(autopilotPromptChangeError({ kind: "save", name: "n".repeat(81), prompt: "x" }, LIST))
      .toBe("A saved prompt's name is at most 80 characters.");
    expect(autopilotPromptChangeError({ kind: "save", name: "é".repeat(80), prompt: "x" }, LIST)).toBeUndefined();
    expect(autopilotPromptChangeError({ kind: "save", name: "a\tb", prompt: "x" }, LIST))
      .toBe("A saved prompt's name cannot contain control characters.");
    expect(autopilotPromptChangeError({ kind: "save", name: "apas WORKER", prompt: "x" }, LIST))
      .toBe("\"APAS worker\" is the name of a built-in template. Choose another name.");
    expect(autopilotPromptChangeError({ kind: "save", name: "nightly review ", prompt: "x" }, LIST))
      .toBe("A saved prompt named \"Nightly review\" already exists.");
    expect(autopilotPromptChangeError({ kind: "save", name: "nightly review", prompt: "x", replace: true }, LIST)).toBeUndefined();
    expect(autopilotPromptChangeError({ kind: "save", name: "Other", prompt: "  " }, LIST)).toBe("Autopilot needs a prompt.");
    const full = Array.from({ length: 50 }, (_, index) => ({ name: `P${index}`, prompt: "x", updated_at: "" }));
    expect(autopilotPromptChangeError({ kind: "save", name: "Another", prompt: "x" }, full))
      .toBe("A project keeps at most 50 saved prompts. Delete one first.");
    expect(autopilotPromptChangeError({ kind: "save", name: "p3", prompt: "x", replace: true }, full)).toBeUndefined();
    expect(autopilotPromptChangeError({ kind: "delete", name: "Gone" }, LIST)).toBe("No saved prompt is named \"Gone\".");
    expect(autopilotPromptChangeError({ kind: "set_default", name: "Gone" }, LIST)).toBe("No saved prompt is named \"Gone\".");
    expect(autopilotPromptChangeError({ kind: "set_default", name: null }, [])).toBeUndefined();
  });
});
