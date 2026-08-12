import { FlatList, ScrollView, StyleSheet } from "react-native";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react-native";
import type { CodeEvent, MobileLaunchTarget, MobileSessionSummary, PaneConfig, PaneWorkSummary, WebToServer } from "@apas/protocol";

import CodeHomeScreen from "@/../app/(code)/(tabs)/index";
import NewTaskScreen from "@/../app/(code)/new";
import SessionActivityScreen from "@/../app/(code)/session/[sessionId]/index";
import { ReviewCard } from "@/../app/(code)/session/[sessionId]/review";
import { DecisionActions } from "@/components/DecisionActions";
import { EventCard } from "@/components/EventCard";
import { OfflineBanner } from "@/components/ui";
import { useMobileStore } from "@/state/store";

let mockParams: Record<string, string | undefined> = {};
const mockReadCachedSnapshot = jest.fn();
const mockSend = jest.fn<boolean, [WebToServer]>(() => true);
const mockSendAcknowledged = jest.fn();
const mockLaunchTask = jest.fn();
const mockLoadTaskDraft = jest.fn();
const mockSaveTaskDraft = jest.fn();
const mockDeleteTaskDraft = jest.fn();
const mockReadConversationPosition = jest.fn();
const mockSaveConversationPosition = jest.fn();
const mockReadSelectedConversationPane = jest.fn();
const mockSaveSelectedConversationPane = jest.fn();
const mockReadPaneWorkSummarySnapshot = jest.fn();
const mockRandomUUID = jest.fn(() => "04cbf715-81d0-42ca-86c5-87913e77c2c9");

jest.mock("expo-router", () => ({
  router: { push: jest.fn(), replace: jest.fn() },
  useLocalSearchParams: () => mockParams,
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: jest.requireActual<typeof import("react-native")>("react-native").View,
}));
jest.mock("@/connection/runtime", () => ({
  connectionSupervisor: () => ({ send: mockSend, sendAcknowledged: mockSendAcknowledged }),
}));
jest.mock("@/storage/cache", () => ({
  readCachedSnapshot: (...args: unknown[]) => mockReadCachedSnapshot(...args),
  loadTaskDraft: (...args: unknown[]) => mockLoadTaskDraft(...args),
  saveTaskDraft: (...args: unknown[]) => mockSaveTaskDraft(...args),
  deleteTaskDraft: (...args: unknown[]) => mockDeleteTaskDraft(...args),
  readConversationPosition: (...args: unknown[]) => mockReadConversationPosition(...args),
  saveConversationPosition: (...args: unknown[]) => mockSaveConversationPosition(...args),
  readSelectedConversationPane: (...args: unknown[]) => mockReadSelectedConversationPane(...args),
  saveSelectedConversationPane: (...args: unknown[]) => mockSaveSelectedConversationPane(...args),
  readPaneWorkSummarySnapshot: (...args: unknown[]) => mockReadPaneWorkSummarySnapshot(...args),
}));
jest.mock("@/api/client", () => ({ launchTask: (...args: unknown[]) => mockLaunchTask(...args) }));
jest.mock("expo-crypto", () => ({ randomUUID: () => mockRandomUUID() }));

const session: MobileSessionSummary = {
  id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
  project_id: "9cd95c53-90d1-472a-89c9-a3a008fc15a4",
  project_name: "mobile-test",
  hostname: "builder",
  working_dir: "/workspace/mobile-test",
  status: "active",
  is_active: true,
  latest_update_at: "2026-08-08T12:00:00Z",
};

const launchTarget: MobileLaunchTarget = {
  hostname: "builder",
  instance_path: "/workspace/mobile-test",
  machine_id: "6e9cb4fe-3c03-4937-ae66-c97755939776",
  online: true,
  profiles: [{
    key: "terminal:codex:official:default",
    kind: "terminal",
    label: "Codex terminal",
    mode: "interactive",
    provider: "codex",
  }],
  project_id: "9cd95c53-90d1-472a-89c9-a3a008fc15a4",
  project_name: session.project_name ?? "mobile-test",
};

const terminalPane: PaneConfig = {
  pane_id: 8,
  provider: "codex",
  mode: "interactive",
  kind: "terminal",
  session_id: "terminal-pane-session",
  is_paused: false,
  label: "Codex TTY 8",
  managed: false,
};

function event(index: number): CodeEvent {
  return {
    id: `event-${index}`,
    session_id: session.id,
    pane_id: 3,
    ordering_key: `2026-08-08T12:00:00.000Z:${index.toString().padStart(10, "0")}`,
    created_at: "2026-08-08T12:00:00Z",
    kind: "agent_status",
    summary: `Event ${index}`,
  };
}

function workSummary(paneId: number, status: PaneWorkSummary["status"] = "complete"): PaneWorkSummary {
  return {
    session_id: session.id,
    pane_id: paneId,
    window_start: "2026-08-08T09:00:00Z",
    window_end: "2026-08-08T12:00:00Z",
    status,
    summary: `Pane ${paneId} summarized work`,
    source_message_count: 4,
    provider: "codex",
  };
}

describe("mobile code screens", () => {
  beforeEach(() => {
    cleanup();
    mockParams = {};
    mockSend.mockClear();
    mockSendAcknowledged.mockReset();
    mockLaunchTask.mockReset();
    mockLoadTaskDraft.mockReset().mockResolvedValue(null);
    mockSaveTaskDraft.mockReset().mockResolvedValue(undefined);
    mockDeleteTaskDraft.mockReset().mockResolvedValue(undefined);
    mockReadConversationPosition.mockReset().mockReturnValue(new Promise(() => undefined));
    mockSaveConversationPosition.mockReset().mockResolvedValue(undefined);
    mockReadSelectedConversationPane.mockReset().mockReturnValue(new Promise(() => undefined));
    mockSaveSelectedConversationPane.mockReset().mockResolvedValue(undefined);
    mockReadPaneWorkSummarySnapshot.mockReset().mockResolvedValue(null);
    mockRandomUUID.mockClear();
    mockReadCachedSnapshot.mockReset();
    useMobileStore.setState({
      hydrated: true,
      signedIn: true,
      connection: "ready",
      serverMutationsAllowed: true,
      sessions: [],
      eventsBySession: {},
      panesBySession: {},
      paneStatusesBySession: {},
      negotiatedCapabilities: [],
      paneWorkSummaries: {},
      visibleSummaryPane: null,
      lastUpdatedAt: null,
      launchTargets: [],
      features: {},
    });
  });

  afterEach(cleanup);

  it("renders a useful zero-session state instead of a blank screen", () => {
    const view = render(<CodeHomeScreen />);
    expect(view.getByText("No coding sessions yet")).toBeTruthy();
    expect(view.getByText("Start a task")).toBeTruthy();
  });

  it("keeps the horizontal project selector compact", () => {
    useMobileStore.setState({
      sessions: [
        session,
        {
          ...session,
          id: "9478d3c9-8527-437c-a7c4-93437f3a2e2f",
          project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
          project_name: "second-project",
        },
      ],
    });
    const view = render(<CodeHomeScreen />);
    const selector = view.UNSAFE_getAllByType(FlatList).find(
      (list) => list.props.horizontal && list.props.data?.includes("All projects"),
    );

    expect(selector).toBeTruthy();
    expect(StyleSheet.flatten(selector?.props.style)).toMatchObject({ flexGrow: 0, flexShrink: 0 });
    expect(StyleSheet.flatten(selector?.props.contentContainerStyle)).toMatchObject({ alignItems: "center" });
  });

  it("promotes the session most recently messaged by the user", () => {
    const older = {
      ...session,
      project_name: "older-session",
      last_user_input_at: "2026-08-08T12:00:00Z",
    };
    const newer = {
      ...session,
      id: "9478d3c9-8527-437c-a7c4-93437f3a2e2f",
      project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
      project_name: "newer-session",
      last_user_input_at: "2026-08-09T12:00:00Z",
    };
    useMobileStore.setState({ sessions: [older, newer] });

    const view = render(<CodeHomeScreen />);
    const sessionList = view.UNSAFE_getAllByType(FlatList).find(
      (list) => !list.props.horizontal && list.props.data?.some((item: MobileSessionSummary) => item.id === older.id),
    );

    expect(sessionList?.props.data.map((item: MobileSessionSummary) => item.id)).toEqual([newer.id, older.id]);
  });

  it("updates user-message recency from the acknowledged instruction event", () => {
    const other = {
      ...session,
      id: "9478d3c9-8527-437c-a7c4-93437f3a2e2f",
      project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
      project_name: "just-messaged",
      last_user_input_at: "2026-08-07T12:00:00Z",
    };
    useMobileStore.setState({
      sessions: [{ ...session, last_user_input_at: "2026-08-08T12:00:00Z" }, other],
    });
    useMobileStore.getState().setEvents(other.id, [{
      id: "instruction-1",
      session_id: other.id,
      pane_id: 3,
      ordering_key: "2026-08-09T12:00:00.000Z:0000000001",
      created_at: "2026-08-09T12:00:00Z",
      kind: "instruction",
      summary: "Continue",
    }]);

    const view = render(<CodeHomeScreen />);
    const sessionList = view.UNSAFE_getAllByType(FlatList).find(
      (list) => !list.props.horizontal && list.props.data?.some((item: MobileSessionSummary) => item.id === other.id),
    );

    expect(sessionList?.props.data[0].id).toBe(other.id);
    expect(useMobileStore.getState().sessions.find((item) => item.id === other.id)?.last_user_input_at)
      .toBe("2026-08-09T12:00:00Z");
  });

  it("filters attention sessions in place like the other status controls", () => {
    useMobileStore.setState({
      sessions: [
        { ...session, project_name: "needs-attention", attention_count: 1 },
        {
          ...session,
          id: "dbb94295-01ce-4945-b586-e43f4c798e68",
          project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
          project_name: "no-attention",
          attention_count: 0,
        },
      ],
    });
    const view = render(<CodeHomeScreen />);

    expect(view.getByText("Account")).toBeTruthy();
    fireEvent.press(view.getByText("Attention"));
    expect(view.getByLabelText("Open needs-attention")).toBeTruthy();
    expect(view.queryByLabelText("Open no-attention")).toBeNull();
  });

  it("labels cached offline rendering and disables action assumptions", () => {
    useMobileStore.setState({ connection: "offline", lastUpdatedAt: "2026-08-08T12:00:00Z" });
    const view = render(<OfflineBanner />);
    expect(view.getByText(/Offline · last updated/)).toBeTruthy();
    expect(view.getByText(/actions unavailable/)).toBeTruthy();
  });

  it("falls back safely when a routed session is inaccessible", () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    const view = render(<SessionActivityScreen />);
    expect(view.getByText("Session unavailable")).toBeTruthy();
  });

  it("keeps a high-volume timeline virtualized with bounded batches", async () => {
    const events = Array.from({ length: 2_000 }, (_, index) => event(index));
    mockParams = { sessionId: session.id, paneId: "3" };
    mockReadCachedSnapshot.mockResolvedValue({ events, watermarks: {}, sessions: [session], updatedAt: "2026-08-08T12:00:00Z" });
    useMobileStore.setState({ sessions: [session] });
    const view = render(<SessionActivityScreen />);
    await waitFor(() => {
      const timeline = view.UNSAFE_getAllByType(FlatList).find((list) => list.props.data?.length === 2_000);
      expect(timeline).toBeTruthy();
      expect(timeline?.props.initialNumToRender).toBe(20);
      expect(timeline?.props.maxToRenderPerBatch).toBe(15);
      expect(timeline?.props.windowSize).toBe(9);
    });
  });

  it("remembers a conversation scroll position and follows newest only near the bottom", async () => {
    const events = Array.from({ length: 4 }, (_, index) => event(index));
    mockParams = { sessionId: session.id, paneId: "3" };
    mockReadCachedSnapshot.mockResolvedValue({ events, watermarks: {}, sessions: [session], updatedAt: "2026-08-08T12:00:00Z" });
    mockReadConversationPosition.mockResolvedValue({ offset: 240, followNewest: false });
    useMobileStore.setState({ sessions: [session], eventsBySession: { [session.id]: events } });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(mockReadConversationPosition).toHaveBeenCalledWith(session.id, 3));
    const timeline = view.UNSAFE_getAllByType(FlatList).find((list) => list.props.data?.length === 4);
    expect(timeline).toBeTruthy();
    fireEvent.scroll(timeline!, {
      nativeEvent: {
        contentOffset: { x: 0, y: 310 },
        contentSize: { width: 300, height: 1200 },
        layoutMeasurement: { width: 300, height: 500 },
      },
    });
    timeline?.props.onMomentumScrollEnd();
    await waitFor(() => expect(mockSaveConversationPosition).toHaveBeenCalledWith(session.id, 3, {
      offset: 310,
      followNewest: false,
    }));
  });

  it("shows one pane's conversation and restores the last selected pane", async () => {
    const paneThreeEvent = event(30);
    const paneFourEvent = { ...event(40), pane_id: 4, summary: "Pane four only" };
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    mockReadSelectedConversationPane.mockResolvedValue(4);
    useMobileStore.setState({
      sessions: [session],
      eventsBySession: { [session.id]: [paneThreeEvent, paneFourEvent] },
      panesBySession: {
        [session.id]: [
          { ...terminalPane, pane_id: 3, kind: "agent", label: "Codex 3" },
          { ...terminalPane, pane_id: 4, kind: "agent", label: "Claude 4", provider: "claude" },
        ],
      },
    });
    const view = render(<SessionActivityScreen />);
    const visibleMessageSummaries = () => view
      .getAllByTestId("event-message-line")
      .map((line) => line.props.children[0]);

    await waitFor(() => expect(view.getByText("✓ Claude 4")).toBeTruthy());
    expect(visibleMessageSummaries()).toContain("Pane four only");
    expect(visibleMessageSummaries()).not.toContain("Event 30");

    fireEvent.press(view.getByText("Codex 3"));
    await waitFor(() => expect(visibleMessageSummaries()).toContain("Event 30"));
    expect(visibleMessageSummaries()).not.toContain("Pane four only");
    expect(mockSaveSelectedConversationPane).toHaveBeenCalledWith(session.id, 3);
  });

  it("defaults a terminal pane to conversation and sends typed messages to its TUI", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [session],
      features: { terminal: true, coding_mutations: true },
      panesBySession: { [session.id]: [terminalPane] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Codex TTY 8")).toBeTruthy());
    expect(view.getByText("Raw terminal")).toBeTruthy();
    expect(view.queryByText("Review")).toBeNull();
    expect(view.queryByText("Interrupt")).toBeNull();
    expect(view.queryByText("Summary")).toBeNull();
    expect(view.queryByText(/Conversation mode/)).toBeNull();
    fireEvent.changeText(view.getByPlaceholderText("Message this terminal conversation"), "Please summarize the current state.");
    fireEvent.press(view.getByText("Send message"));

    await waitFor(() => expect(mockSendAcknowledged).toHaveBeenCalledWith({
      type: "terminal_conversation_input",
      session_id: session.id,
      pane_id: 8,
      text: "Please summarize the current state.",
      client_msg_id: "04cbf715-81d0-42ca-86c5-87913e77c2c9",
    }, "04cbf715-81d0-42ca-86c5-87913e77c2c9"));
    expect(mockSend.mock.calls.map(([message]) => message).filter((message) => message.type === "terminal_input")).toEqual([]);
    expect(mockSend.mock.calls.map(([message]) => message).filter(
      (message) => message.type === "list_pane_work_summaries" || message.type === "refresh_pane_work_summary",
    )).toEqual([]);
    expect(view.getByPlaceholderText("Message this terminal conversation").props.value).toBe("");
  });

  it("opens pane-scoped summaries without unmounting or resetting the conversation", async () => {
    const paneThreeEvent = event(3);
    const paneFourEvent = { ...event(4), pane_id: 4, summary: "Pane four conversation" };
    mockParams = { sessionId: session.id, paneId: "3" };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [session],
      eventsBySession: { [session.id]: [paneThreeEvent, paneFourEvent] },
      panesBySession: {
        [session.id]: [
          { ...terminalPane, pane_id: 3, kind: "agent", label: "Codex 3" },
          { ...terminalPane, pane_id: 4, kind: "agent", label: "Claude 4", provider: "claude" },
        ],
      },
      negotiatedCapabilities: ["pane_work_summary_v1"],
    });
    const view = render(<SessionActivityScreen />);
    const timeline = view.UNSAFE_getAllByType(FlatList).find((list) => list.props.data?.[0]?.id === paneThreeEvent.id);
    fireEvent.scroll(timeline!, {
      nativeEvent: {
        contentOffset: { x: 0, y: 260 },
        contentSize: { width: 300, height: 1200 },
        layoutMeasurement: { width: 300, height: 500 },
      },
    });

    fireEvent.press(view.getByText("Summary"));
    await waitFor(() => expect(mockSend).toHaveBeenCalledWith({
      type: "list_pane_work_summaries",
      session_id: session.id,
      pane_id: 3,
      include_current: true,
    }));
    expect(view.getByTestId("event-message-line").props.children[0]).toBe("Event 3");
    expect(mockSaveConversationPosition).not.toHaveBeenCalled();

    act(() => useMobileStore.getState().replacePaneWorkSummaries(
      session.id,
      3,
      [workSummary(3, "failed")],
      "available",
    ));
    await waitFor(() => expect(view.getByText("Pane 3 summarized work")).toBeTruthy());
    fireEvent.press(view.getByText("Refresh current window"));
    expect(mockSend).toHaveBeenCalledWith({
      type: "refresh_pane_work_summary",
      session_id: session.id,
      pane_id: 3,
    });
    act(() => useMobileStore.getState().replacePaneWorkSummaries(
      session.id,
      3,
      [workSummary(3, "failed")],
      "available",
    ));
    fireEvent.press(view.getByText("Retry"));
    expect(mockSend).toHaveBeenCalledWith({
      type: "refresh_pane_work_summary",
      session_id: session.id,
      pane_id: 3,
      window_start: "2026-08-08T09:00:00Z",
    });

    act(() => useMobileStore.getState().replacePaneWorkSummaries(
      session.id,
      4,
      [workSummary(4)],
      "available",
    ));
    const claudePaneButtons = view.getAllByText("Claude 4");
    fireEvent.press(claudePaneButtons.at(-1)!);
    await waitFor(() => expect(mockSend).toHaveBeenCalledWith({
      type: "list_pane_work_summaries",
      session_id: session.id,
      pane_id: 4,
      include_current: true,
    }));
    expect(view.getByText("Pane 4 summarized work")).toBeTruthy();
    expect(view.queryByText("Pane 3 summarized work")).toBeNull();
    expect(view.getByTestId("event-message-line").props.children[0]).toBe("Pane four conversation");
    expect(mockSaveConversationPosition).toHaveBeenCalledWith(session.id, 3, {
      offset: 260,
      followNewest: false,
    });

    fireEvent.press(view.getByText("Close"));
    expect(view.getByTestId("event-message-line").props.children[0]).toBe("Pane four conversation");
    expect(view.getByText("✓ Claude 4")).toBeTruthy();
  });

  it("shows a fresh, pane-isolated cached summary offline with controls disabled", async () => {
    mockParams = { sessionId: session.id, paneId: "3" };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    mockReadPaneWorkSummarySnapshot.mockResolvedValue({
      sessionId: session.id,
      paneId: 3,
      summaries: [workSummary(3)],
      availability: "available",
      updatedAt: "2026-08-12T12:00:00Z",
    });
    useMobileStore.setState({
      connection: "offline",
      sessions: [session],
      panesBySession: { [session.id]: [{ ...terminalPane, pane_id: 3, kind: "agent", label: "Codex 3" }] },
      negotiatedCapabilities: ["pane_work_summary_v1"],
    });
    const view = render(<SessionActivityScreen />);
    fireEvent.press(view.getByText("Summary"));

    await waitFor(() => expect(view.getByText("Pane 3 summarized work")).toBeTruthy());
    expect(view.getByText(/Offline cached view · updated/)).toBeTruthy();
    fireEvent.press(view.getByRole("button", { name: "Refresh current window" }));
    expect(mockSend.mock.calls.some(([message]) => message.type === "refresh_pane_work_summary")).toBe(false);
    expect(mockSend.mock.calls.some(([message]) => message.type === "list_pane_work_summaries")).toBe(false);
  });

  it("shows and clears the selected pane's live working state", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [session],
      panesBySession: { [session.id]: [terminalPane] },
      paneStatusesBySession: { [session.id]: { "8": "Working..." } },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByTestId("pane-working-status")).toBeTruthy());
    expect(view.getByText("Working...")).toBeTruthy();
    expect(view.getByText("✓ Codex TTY 8 · working")).toBeTruthy();

    act(() => useMobileStore.getState().setPaneStatus(session.id, 8, null));
    await waitFor(() => expect(view.queryByTestId("pane-working-status")).toBeNull());
  });

  it("styles user instructions as distinct bubbles without a redundant kind label", () => {
    const view = render(<EventCard
      event={{ ...event(1), kind: "instruction", summary: "Hello, 123" }}
      expanded={false}
      onPress={jest.fn()}
    />);

    expect(view.getByTestId("event-message-line").props.children[0]).toBe("Hello, 123");
    expect(view.queryByText("instruction")).toBeNull();
    expect(view.getByTestId("event-message-line")).toBeTruthy();
    expect(StyleSheet.flatten(view.getByRole("button").props.style)).toMatchObject({
      alignSelf: "flex-end",
      marginLeft: 40,
      backgroundColor: "#eeecff",
    });
  });

  it("keeps acknowledged conversation input for an agent pane", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    mockSendAcknowledged.mockResolvedValue({ type: "mutation_ack", accepted: true });
    useMobileStore.setState({
      sessions: [session],
      panesBySession: { [session.id]: [{ ...terminalPane, pane_id: 3, kind: "agent", label: "Codex 3" }] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Codex 3")).toBeTruthy());
    fireEvent.changeText(view.getByPlaceholderText("Steer this exact session and pane"), "Run the focused tests.");
    fireEvent.press(view.getByText("Send message"));

    await waitFor(() => expect(mockSendAcknowledged).toHaveBeenCalledWith(expect.objectContaining({
      type: "input",
      session_id: session.id,
      pane_id: 3,
      text: "Run the focused tests.",
    }), expect.any(String)));
    expect(mockSend.mock.calls.map(([message]) => message).filter((message) => message.type === "terminal_input")).toEqual([]);
  });

  it("preserves narrow-screen source formatting with horizontal navigation", () => {
    const diffEvent: CodeEvent = {
      ...event(1),
      kind: "diff",
      summary: "Changes ready",
      detail: { diff: "diff --git a/src/file.ts b/src/file.ts\n+const extraordinarilyLongIdentifierThatMustNotWrap = true;" },
    };
    const view = render(<ReviewCard event={diffEvent} />);
    const sourceScroller = view.UNSAFE_getAllByType(ScrollView).find((scroll) => scroll.props.horizontal === true);
    expect(sourceScroller).toBeTruthy();
    expect(view.getByText("src/file.ts")).toBeTruthy();
  });

  it("reuses the retained launch request id after an uncertain failure", async () => {
    useMobileStore.setState({
      launchTargets: [launchTarget],
      features: { coding_mutations: true },
      sessions: [session],
    });
    mockLaunchTask
      .mockRejectedValueOnce(new Error("The launch acknowledgement was lost"))
      .mockResolvedValueOnce({ session_id: session.id, pane_id: 3, replayed: true });
    const view = render(<NewTaskScreen />);
    await waitFor(() => expect(mockLoadTaskDraft).toHaveBeenCalled());

    fireEvent.press(view.getByText(launchTarget.project_name));
    fireEvent.press(view.getByText("Codex terminal"));
    fireEvent.changeText(view.getByPlaceholderText(/diagnose the failing login test/), "Fix the mobile test");
    fireEvent.press(view.getByText("Review task"));
    fireEvent.press(view.getByText("Submit task"));
    await waitFor(() => expect(view.getByText("The launch acknowledgement was lost")).toBeTruthy());
    expect(mockLaunchTask.mock.calls[0]?.[0]).toMatchObject({
      request_id: "04cbf715-81d0-42ca-86c5-87913e77c2c9",
      instruction: "Fix the mobile test",
    });

    fireEvent.press(view.getByText("Retry task safely"));
    await waitFor(() => expect(mockLaunchTask).toHaveBeenCalledTimes(2));
    expect(mockLaunchTask.mock.calls[1]?.[0].request_id).toBe(mockLaunchTask.mock.calls[0]?.[0].request_id);
    expect(mockDeleteTaskDraft).toHaveBeenCalled();
    expect(useMobileStore.getState().sessions[0]?.last_user_input_at).toBeTruthy();
  });

  it("surfaces a stale decision rejection instead of resolving optimistically", async () => {
    const approval: CodeEvent = {
      ...event(9),
      kind: "approval",
      summary: "Approve command?",
      requires_attention: true,
      detail: { type: "output", output_type: { approval_request: { tool_call_id: "tool-9" } } },
    };
    const view = render(<DecisionActions
      event={approval}
      disabled={false}
      onRespond={() => Promise.reject(new Error("This approval was already resolved"))}
    />);

    fireEvent.press(view.getByText("Approve"));
    await waitFor(() => expect(view.getByText("This approval was already resolved")).toBeTruthy());
    expect(view.getByText("Approve command?")).toBeTruthy();
  });

});
