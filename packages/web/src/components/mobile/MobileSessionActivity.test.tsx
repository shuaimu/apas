import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore, type Message, type PaneConfig } from "@/lib/store";
import { MobileSessionActivity, type MobileSessionActivityProps } from "./MobileSessionActivity";

vi.mock("@/components/tabs/TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => <div>Raw terminal pane {paneId}</div>,
}));

const initialStore = useStore.getInitialState();

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id">): PaneConfig {
  return {
    provider: "codex",
    mode: "interactive",
    session_id: `pane-session-${overrides.pane_id}`,
    is_paused: false,
    label: `Codex ${overrides.pane_id}`,
    ...overrides,
  };
}

function message(overrides: Partial<Message> & Pick<Message, "id">): Message {
  return {
    role: "assistant",
    content: "Implemented the requested change.",
    timestamp: new Date("2026-08-09T12:00:00Z"),
    outputType: { type: "text" },
    ...overrides,
  };
}

function seedStore(overrides: Record<string, unknown> = {}) {
  const loadSessionActivity = vi.fn();
  const loadPaneMessagesIfNeeded = vi.fn();
  const loadMoreMessages = vi.fn();
  const sendMessageToPane = vi.fn(() => ({ success: true }));
  const sendTerminalInput = vi.fn();
  const sendTerminalConversationMessage = vi.fn(() => ({ success: true }));
  const interruptPane = vi.fn();
  const approve = vi.fn();
  const reject = vi.fn();
  const answerPlanReview = vi.fn();
  const requestPaneDiff = vi.fn();
  const addPane = vi.fn(() => ({ success: true }));

  act(() => {
    useStore.setState({
      sessionId: "session-a",
      sessions: [{
        id: "session-a",
        projectId: "project-a",
        workingDir: "/workspace/alpha",
        hostname: "builder",
        status: "active",
        isActive: true,
      }],
      paneConfigs: [pane({ pane_id: 3 })],
      paneMessages: { "3": [message({ id: "message-a" })] },
      messages: [],
      paneStatuses: {},
      paneHasMore: {},
      isAttached: true,
      answeredQuestions: new Map(),
      planReviewPending: [],
      projectPolicies: {
        "session-a": {
          teamAvailable: true,
          allowedLaunchProfiles: ["terminal:codex:official:default"],
          version: 1,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
      paneDiffs: {},
      loadSessionActivity,
      loadPaneMessagesIfNeeded,
      loadMoreMessages,
      sendMessageToPane,
      sendTerminalInput,
      sendTerminalConversationMessage,
      interruptPane,
      approve,
      reject,
      answerPlanReview,
      requestPaneDiff,
      addPane,
      ...overrides,
    });
  });

  return {
    addPane,
    answerPlanReview,
    approve,
    interruptPane,
    loadMoreMessages,
    loadPaneMessagesIfNeeded,
    loadSessionActivity,
    reject,
    requestPaneDiff,
    sendMessageToPane,
    sendTerminalConversationMessage,
    sendTerminalInput,
  };
}

function renderActivity(overrides: Partial<MobileSessionActivityProps> = {}) {
  const props: MobileSessionActivityProps = {
    connected: true,
    onAccount: vi.fn(),
    onBack: vi.fn(),
    onReconnect: vi.fn(),
    ...overrides,
  };
  render(<MobileSessionActivity {...props} />);
  return props;
}

beforeEach(() => {
  for (let index = localStorage.length - 1; index >= 0; index -= 1) {
    const key = localStorage.key(index);
    if (key?.startsWith("apas_mobile_activity_scroll:") || key?.startsWith("apas_mobile_selected_pane:")) {
      localStorage.removeItem(key);
    }
  }
  seedStore();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  act(() => useStore.setState(initialStore, true));
});

describe("MobileSessionActivity", () => {
  it("renders a native-style activity timeline instead of the desktop tab view", async () => {
    const actions = seedStore();
    renderActivity();

    expect(screen.getByRole("heading", { name: "alpha" })).toBeTruthy();
    const agentCard = screen.getByText("Implemented the requested change.").closest("article");
    expect(agentCard?.querySelector("[data-message-line] time")).toBeTruthy();
    expect(screen.queryByText("agent")).toBeNull();
    expect(screen.queryByText("Pane 3")).toBeNull();
    expect(screen.getByRole("button", { name: "Codex 3" })).toBeTruthy();
    expect(actions.loadSessionActivity).toHaveBeenCalledWith("session-a");
    await waitFor(() => expect(screen.getByPlaceholderText("Steer this session and pane")).toBeTruthy());
  });

  it("shows the selected pane's live working state above the composer", () => {
    seedStore({ paneStatuses: { "3": "Editing src/session.ts…" } });
    renderActivity();

    expect(screen.getByRole("status").textContent).toBe("Editing src/session.ts…");
    expect(screen.getByRole("button", { name: "Codex 3 · working" })).toBeTruthy();
  });

  it("opens a new conversation at the newest activity", () => {
    const scrollHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => 1000 });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 200 });
    try {
      renderActivity();
      expect((screen.getByRole("log", { name: "Conversation activity" }) as HTMLDivElement).scrollTop).toBe(800);
    } finally {
      if (scrollHeight) Object.defineProperty(HTMLElement.prototype, "scrollHeight", scrollHeight);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>).scrollHeight;
      if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
    }
  });

  it("restores the remembered position instead of returning to the oldest activity", () => {
    localStorage.setItem("apas_mobile_activity_scroll:session-a:3", JSON.stringify({
      scrollTop: 275,
      followNewest: false,
    }));
    const scrollHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => 1000 });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 200 });
    try {
      renderActivity();
      expect((screen.getByRole("log", { name: "Conversation activity" }) as HTMLDivElement).scrollTop).toBe(275);
    } finally {
      if (scrollHeight) Object.defineProperty(HTMLElement.prototype, "scrollHeight", scrollHeight);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>).scrollHeight;
      if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
    }
  });

  it("restores the conversation position after returning from Raw terminal", async () => {
    const scrollHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => 1000 });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 200 });
    try {
      seedStore({
        paneConfigs: [pane({ pane_id: 9, kind: "terminal", label: "Codex TTY" })],
        paneMessages: { "9": [message({ id: "terminal-message" })] },
      });
      renderActivity();
      const activity = await screen.findByRole("log", { name: "Conversation activity" }) as HTMLDivElement;
      activity.scrollTop = 275;
      fireEvent.scroll(activity);

      fireEvent.click(screen.getByRole("button", { name: /Raw terminal/ }));
      expect(await screen.findByRole("region", { name: "Mobile terminal" })).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: /Conversation/ }));

      const restored = await screen.findByRole("log", { name: "Conversation activity" }) as HTMLDivElement;
      expect(restored).not.toBe(activity);
      expect(restored.scrollTop).toBe(275);
    } finally {
      if (scrollHeight) Object.defineProperty(HTMLElement.prototype, "scrollHeight", scrollHeight);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>).scrollHeight;
      if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
    }
  });

  it("shows only the selected pane and restores the last selected pane", async () => {
    seedStore({
      paneConfigs: [
        pane({ pane_id: 3, label: "Codex 3" }),
        pane({ pane_id: 4, label: "Claude 4", provider: "claude" }),
      ],
      paneMessages: {
        "3": [message({ id: "pane-3-message", content: "Only pane three" })],
        "4": [message({ id: "pane-4-message", content: "Only pane four" })],
      },
    });
    renderActivity();

    expect(await screen.findByText("Only pane three")).toBeTruthy();
    expect(screen.queryByText("Only pane four")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Claude 4" }));
    expect(await screen.findByText("Only pane four")).toBeTruthy();
    expect(screen.queryByText("Only pane three")).toBeNull();

    cleanup();
    seedStore({
      paneConfigs: [
        pane({ pane_id: 3, label: "Codex 3" }),
        pane({ pane_id: 4, label: "Claude 4", provider: "claude" }),
      ],
      paneMessages: {
        "3": [message({ id: "pane-3-message", content: "Only pane three" })],
        "4": [message({ id: "pane-4-message", content: "Only pane four" })],
      },
    });
    renderActivity();
    await waitFor(() => expect(screen.getByRole("button", { name: "Claude 4" }).getAttribute("aria-pressed")).toBe("true"));
    expect(screen.getByText("Only pane four")).toBeTruthy();
    expect(screen.queryByText("Only pane three")).toBeNull();
  });

  it("sends follow-up instructions to the selected pane", async () => {
    const actions = seedStore();
    renderActivity();
    const composer = await screen.findByPlaceholderText("Steer this session and pane");

    fireEvent.change(composer, { target: { value: "Please run the focused tests." } });
    fireEvent.click(screen.getByRole("button", { name: "Send follow-up" }));

    expect(actions.sendMessageToPane).toHaveBeenCalledWith("Please run the focused tests.", 3);
    expect((composer as HTMLTextAreaElement).value).toBe("");
  });

  it("shows the text of messages sent by the user instead of a generic title", () => {
    seedStore({
      paneMessages: {
        "3": [message({
          id: "user-message",
          role: "user",
          content: "Hello, 123",
        })],
      },
    });
    renderActivity();

    const card = screen.getByText("Hello, 123").closest("article");
    expect(card).toBeTruthy();
    expect(card?.className).toContain("ml-10");
    expect(card?.className).toContain("bg-[#eeecff]");
    expect(card?.textContent).not.toContain("Pane 3");
    expect(card?.textContent).not.toContain("instruction");
    const messageLine = card?.querySelector("[data-message-line]");
    expect(messageLine?.textContent).toContain("Hello, 123");
    expect(messageLine?.querySelector("time")).toBeTruthy();
    expect(card?.querySelector("[data-message-header]")).toBeNull();
    expect(screen.queryByText("Instruction sent")).toBeNull();
  });

  it("expands agent detail to a full-width panel outside the toggle button", () => {
    seedStore({
      paneMessages: {
        "3": [message({
          id: "tool-message",
          content: "Running a focused command",
          outputType: {
            type: "tool_use",
            tool: "Bash",
            input: { command: "npm test -- MobileSessionActivity" },
          },
        })],
      },
    });
    renderActivity();

    const toggle = screen.getByRole("button", { name: /Using Bash/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);

    const detail = screen.getByText(/npm test -- MobileSessionActivity/);
    expect(detail.tagName).toBe("PRE");
    expect(detail.className).toContain("w-full");
    expect(detail.className).toContain("min-w-0");
    expect(detail.className).toContain("max-w-full");
    expect(toggle.contains(detail)).toBe(false);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
  });

  it("keeps approval decisions actionable in the activity timeline", () => {
    const approval = message({
      id: "approval",
      content: "Permission required",
      outputType: {
        type: "approval_request",
        toolCallId: "tool-call-1",
        tool: "Bash",
        description: "Run npm test",
      },
    });
    const actions = seedStore({ paneMessages: { "3": [approval] } });
    renderActivity();

    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(actions.approve).toHaveBeenCalledWith("tool-call-1");
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    expect(actions.reject).toHaveBeenCalledWith("tool-call-1");
  });

  it("defaults terminal panes to a writable conversation and keeps raw terminal secondary", async () => {
    const actions = seedStore({
      paneConfigs: [pane({ pane_id: 9, kind: "terminal", label: "Codex TTY" })],
      paneMessages: { "9": [] },
    });
    renderActivity();

    await waitFor(() => expect(screen.getByRole("button", { name: /Codex TTY/ }).getAttribute("aria-pressed")).toBe("true"));
    const composer = screen.getByPlaceholderText("Message this terminal conversation");
    expect((composer as HTMLTextAreaElement).disabled).toBe(false);
    expect(screen.queryByText(/Conversation mode/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Review" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Interrupt" })).toBeNull();
    fireEvent.change(composer, { target: { value: "Please summarize the current state." } });
    fireEvent.click(screen.getByRole("button", { name: "Send conversation message" }));
    expect(actions.sendTerminalConversationMessage).toHaveBeenCalledWith(9, "Please summarize the current state.");
    expect(actions.sendTerminalInput).not.toHaveBeenCalled();
    expect(actions.sendMessageToPane).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /Raw terminal/ }));
    expect(await screen.findByRole("region", { name: "Mobile terminal" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Mobile session activity" })).toBeNull();
  });

  it("keeps the conversation field writable while offline so a message can be drafted", async () => {
    const actions = seedStore({
      isAttached: false,
      paneConfigs: [pane({ pane_id: 9, kind: "terminal", label: "Codex TTY" })],
      paneMessages: { "9": [] },
    });
    renderActivity({ connected: false });

    const composer = await screen.findByPlaceholderText("Message this terminal conversation");
    expect((composer as HTMLTextAreaElement).disabled).toBe(false);
    fireEvent.change(composer, { target: { value: "Keep this draft" } });
    expect((composer as HTMLTextAreaElement).value).toBe("Keep this draft");
    expect(screen.getByText(/keep drafting while offline/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send conversation message" }).hasAttribute("disabled")).toBe(true);
    expect(actions.sendTerminalInput).not.toHaveBeenCalled();
  });

  it("creates panes only from profiles allowed by project policy", async () => {
    const actions = seedStore();
    renderActivity();
    fireEvent.click(screen.getByRole("button", { name: /^Pane$/ }));

    expect(screen.getByRole("dialog", { name: "Create pane" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Codex terminal/ }));
    expect(actions.addPane).toHaveBeenCalledWith(
      "codex",
      "interactive",
      "Codex terminal 2",
      undefined,
      undefined,
      false,
      undefined,
      false,
      "terminal",
    );
  });

  it("keeps back, Account, and reconnect controls available without old navigation bars", () => {
    const props = renderActivity({ connected: false });

    fireEvent.click(screen.getByRole("button", { name: "Back to coding sessions" }));
    expect(props.onBack).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Account" }));
    expect(props.onAccount).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByText(/tap to reconnect/));
    expect(props.onReconnect).toHaveBeenCalledTimes(1);
  });
});
