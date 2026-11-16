import { FlatList, ScrollView, StyleSheet } from "react-native";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react-native";
import type { CodeEvent, MobileLaunchTarget, MobileSessionSummary, PaneConfig, PaneWorkSummary, SessionInstanceInfo, WebToServer } from "@apas/protocol";
import { router } from "expo-router";

import CodeHomeScreen from "@/../app/(code)/(tabs)/index";
import NewTaskScreen from "@/../app/(code)/new";
import SessionActivityScreen from "@/../app/(code)/session/[sessionId]/index";
import { ReviewCard } from "@/../app/(code)/session/[sessionId]/review";
import { DecisionActions } from "@/components/DecisionActions";
import { EventCard } from "@/components/EventCard";
import { OfflineBanner } from "@/components/ui";
import { MutationRefusedError } from "@/connection/supervisor";
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

const ZOO_005 = "50000000-0000-4000-8000-000000000001";
const ZOO_006 = "50000000-0000-4000-8000-000000000002";
const zoo005: SessionInstanceInfo = { machine_id: ZOO_005, hostname: "zoo-005", working_dir: "/workspace/mobile-test", primary: true, runtime_access: true };
const zoo006: SessionInstanceInfo = { machine_id: ZOO_006, hostname: "zoo-006", working_dir: "/workspace/mobile-test", primary: false, runtime_access: true };
const zoo005Pane: PaneConfig = { ...terminalPane, pane_id: 3, provider: "claude", label: "Claude 3", machine_id: ZOO_005, hostname: "zoo-005" };
const zoo006Pane: PaneConfig = { ...terminalPane, pane_id: 4, label: "Codex 4", machine_id: ZOO_006, hostname: "zoo-006" };

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
    // Reset, not clear: a test that fails before using the ids it queued
    // must not hand them to the next one.
    mockRandomUUID.mockReset().mockImplementation(() => "04cbf715-81d0-42ca-86c5-87913e77c2c9");
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
      instancesBySession: {},
      negotiatedCapabilities: [],
      paneWorkSummaries: {},
      visibleSummaryPane: null,
      lastUpdatedAt: null,
      launchTargets: [],
      knownMachines: [],
      features: {},
    });
  });

  afterEach(cleanup);

  it("renders a useful zero-session state instead of a blank screen", () => {
    const view = render(<CodeHomeScreen />);
    expect(view.getByText("No coding sessions yet")).toBeTruthy();
    expect(view.getByText("Start a task")).toBeTruthy();
  });

  it("offers only all-project and idle-project categories", () => {
    useMobileStore.setState({
      sessions: [
        { ...session, project_name: "working-project", is_working: true },
        {
          ...session,
          id: "9478d3c9-8527-437c-a7c4-93437f3a2e2f",
          project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
          project_name: "idle-project",
          is_working: false,
        },
      ],
    });
    const view = render(<CodeHomeScreen />);

    expect(view.getByText("All projects")).toBeTruthy();
    expect(view.getByText("Idle projects")).toBeTruthy();
    expect(view.queryByText("Active")).toBeNull();
    expect(view.queryByText("Attention")).toBeNull();
    expect(view.queryByText("Completed")).toBeNull();
    expect(view.queryByText("Recent")).toBeNull();

    fireEvent.press(view.getByText("Idle projects"));
    expect(view.getByLabelText("Open idle-project")).toBeTruthy();
    expect(view.queryByLabelText("Open working-project")).toBeNull();
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

  it("uses pending answer, working, idle, and offline session badges", () => {
    useMobileStore.setState({
      sessions: [
        {
          ...session,
          id: "pending",
          project_name: "pending",
          is_working: false,
          panes: [{
            pane_id: 3,
            kind: "agent",
            provider: "claude",
            is_working: false,
            awaiting_answer: true,
          }],
        },
        { ...session, id: "working", project_name: "working", is_working: true },
        { ...session, id: "idle", project_name: "idle", is_working: false },
        { ...session, id: "offline", project_name: "offline", is_active: false, is_working: false },
      ],
      paneStatusesBySession: {},
    });
    const view = render(<CodeHomeScreen />);

    expect(view.getByText("Pending answer")).toBeTruthy();
    expect(view.getByText("Working")).toBeTruthy();
    expect(view.getByText("Idle")).toBeTruthy();
    expect(view.getByText("Offline")).toBeTruthy();
    for (const name of ["pending", "working", "idle", "offline"]) {
      expect(view.getByLabelText(`Open ${name}`).findAllByProps({ children: "Active" })).toHaveLength(0);
    }
  });

  it("keeps pending-answer sessions in the idle-project list", () => {
    useMobileStore.setState({
      sessions: [{ ...session, project_name: "needs-answer", is_working: false }],
      paneStatusesBySession: { [session.id]: { "3": "Pending answer" } },
    });
    const view = render(<CodeHomeScreen />);

    fireEvent.press(view.getByText("Idle projects"));
    expect(view.getByLabelText("Open needs-answer")).toBeTruthy();
    expect(view.getByText("Pending answer")).toBeTruthy();
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

  it("uses live pane status when filtering idle projects", () => {
    useMobileStore.setState({
      sessions: [
        { ...session, project_name: "pane-working", is_working: false },
        {
          ...session,
          id: "dbb94295-01ce-4945-b586-e43f4c798e68",
          project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
          project_name: "actually-idle",
          is_working: false,
        },
      ],
      paneStatusesBySession: { [session.id]: { "3": "Thinking..." } },
    });
    const view = render(<CodeHomeScreen />);

    expect(view.getByText("Account")).toBeTruthy();
    fireEvent.press(view.getByText("Idle projects"));
    expect(view.getByLabelText("Open actually-idle")).toBeTruthy();
    expect(view.queryByLabelText("Open pane-working")).toBeNull();
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

  it("shows each tab's host once the project's tabs span machines", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [session],
      panesBySession: {
        [session.id]: [
          zoo005Pane,
          zoo006Pane,
          // Written before multi-machine instances: one pane the server
          // attributed to zoo-005, and one it never attributed to any host.
          { ...terminalPane, pane_id: 5, label: "Legacy 5", hostname: "zoo-005" },
          { ...terminalPane, pane_id: 6, label: "Legacy 6" },
        ],
      },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Claude 3 · zoo-005")).toBeTruthy());
    expect(view.getByText("Codex 4 · zoo-006")).toBeTruthy();
    expect(view.getByText("Legacy 5 · zoo-005")).toBeTruthy();
    expect(view.getByText("Legacy 6")).toBeTruthy();
  });

  it("keeps tabs unbadged while every tab runs on one host", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [session],
      panesBySession: {
        [session.id]: [zoo005Pane, { ...terminalPane, pane_id: 5, label: "Legacy 5", hostname: "zoo-005" }],
      },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Claude 3")).toBeTruthy());
    expect(view.getByText("Legacy 5")).toBeTruthy();
    expect(view.queryByText(/zoo-005/)).toBeNull();
  });

  it("shows bootstrap tabs with their hosts until a live roster replaces them", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [{
        ...session,
        panes: [
          { pane_id: 3, kind: "terminal", provider: "claude", label: "Claude 3", is_working: false, awaiting_answer: false, machine_id: ZOO_005, hostname: "zoo-005" },
          { pane_id: 4, kind: "terminal", provider: "codex", label: "Codex 4", is_working: false, awaiting_answer: false, machine_id: ZOO_006, hostname: "zoo-006" },
        ],
      }],
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Claude 3 · zoo-005")).toBeTruthy());
    expect(view.getByText("Codex 4 · zoo-006")).toBeTruthy();

    // An explicitly empty live roster means no tabs, not "keep the summary".
    act(() => useMobileStore.getState().setPanes(session.id, []));
    await waitFor(() => expect(view.queryByText(/Codex 4/)).toBeNull());
    expect(view.queryByText("Conversation:")).toBeNull();
  });

  it("refuses a stopped machine's tab with the reason until that machine runs again", async () => {
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    mockSendAcknowledged.mockResolvedValue({ type: "user_input" });
    useMobileStore.setState({
      sessions: [session],
      features: { terminal: true, coding_mutations: true },
      negotiatedCapabilities: ["project_instances_v1"],
      instancesBySession: { [session.id]: [zoo005] },
      panesBySession: { [session.id]: [zoo005Pane, zoo006Pane] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("Codex 4 · zoo-006 · stopped")).toBeTruthy());
    expect(view.queryByTestId("pane-unavailable")).toBeNull();
    fireEvent.press(view.getByText("Codex 4 · zoo-006 · stopped"));
    // The bootstrap listed no connected daemon for zoo-006.
    await waitFor(() => expect(view.getByText("zoo-006 is not running this project because it is offline.")).toBeTruthy());
    expect(view.queryByText(/Start it on/)).toBeNull();
    fireEvent.changeText(view.getByPlaceholderText("Message this terminal conversation"), "Run the suite.");
    expect(view.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.press(view.getByText("Send message"));
    expect(mockSendAcknowledged).not.toHaveBeenCalled();

    // A later synchronization lists zoo-006's daemon as connected: the tab
    // stays refused, with the sentence for a machine that could be started.
    act(() => useMobileStore.setState({ knownMachines: [{ machine_id: ZOO_006, hostname: "zoo-006", connected: true }] }));
    await waitFor(() => expect(view.getByText("zoo-006 is not running this project. Open the project again to start it there.")).toBeTruthy());
    expect(view.queryByText(/because it is offline/)).toBeNull();
    expect(view.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.press(view.getByText("Send message"));
    expect(mockSendAcknowledged).not.toHaveBeenCalled();

    // zoo-006 attaches again, and the live set brings its tab back.
    act(() => useMobileStore.getState().setSessionInstances(session.id, [zoo005, zoo006]));
    await waitFor(() => expect(view.queryByTestId("pane-unavailable")).toBeNull());
    expect(view.getByText("✓ Codex 4 · zoo-006")).toBeTruthy();
    fireEvent.press(view.getByText("Send message"));
    await waitFor(() => expect(mockSendAcknowledged).toHaveBeenCalledWith(expect.objectContaining({
      type: "terminal_conversation_input",
      session_id: session.id,
      pane_id: 4,
      text: "Run the suite.",
    }), expect.any(String)));
  });

  it("keeps a machine this viewer cannot use read-only, decisions included", async () => {
    const approval: CodeEvent = {
      ...event(12),
      pane_id: 4,
      kind: "approval",
      summary: "Approve command?",
      requires_attention: true,
      detail: { type: "output", output_type: { approval_request: { tool_call_id: "tool-12" } } },
    };
    mockParams = { sessionId: session.id, paneId: "4" };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [session],
      features: { terminal: true, coding_mutations: true },
      negotiatedCapabilities: ["project_instances_v1"],
      instancesBySession: { [session.id]: [zoo005, { ...zoo006, runtime_access: false }] },
      panesBySession: { [session.id]: [zoo005Pane, zoo006Pane] },
      eventsBySession: { [session.id]: [approval] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Codex 4 · zoo-006 · no access")).toBeTruthy());
    expect(view.getByText("Your access does not cover zoo-006, so this tab is read-only here.")).toBeTruthy();
    expect(view.getByRole("button", { name: "Approve" })).toBeDisabled();
    fireEvent.press(view.getByText("Approve"));
    expect(mockSendAcknowledged).not.toHaveBeenCalled();
    expect(view.getByText("Claude 3 · zoo-005")).toBeTruthy();
  });

  it("preserves retired bot instructions and provider read-only without a conversion path", async () => {
    mockParams = { sessionId: session.id, paneId: "8" };
    mockReadCachedSnapshot.mockReturnValue(Promise.withResolvers().promise);
    useMobileStore.setState({
      sessions: [session], features: { terminal: true, coding_mutations: true },
      panesBySession: { [session.id]: [{ ...terminalPane, provider: "claude", mode: "deadloop", prompt: "Keep the original responsibility text", min_iteration_interval_minutes: 15 }] },
    });
    const view = render(<SessionActivityScreen />);
    await waitFor(() => expect(view.getByText(/original provider claude/)).toBeTruthy());
    fireEvent.press(view.getByText("Read original bot instructions"));
    expect(view.getByText("Keep the original responsibility text")).toBeTruthy();
    expect(view.getByRole("button", { name: "Send message" })).toBeDisabled();
    fireEvent.press(view.getByText("Send message"));
    expect(mockSendAcknowledged).not.toHaveBeenCalled();
    const selectedSession = useMobileStore.getState().activeSessionId;
    fireEvent.press(view.getByText("Approve a separate fresh worker"));
    expect(router.push).toHaveBeenLastCalledWith({ pathname: "/(code)/workers/new", params: { project: session.project_id } });
    expect(useMobileStore.getState().activeSessionId).toBe(selectedSession);
  });

  it("shows the server's own reason when it refuses typed input for a tab's machine", async () => {
    // A sole instance that reported a machine may or may not own a tab another
    // host recorded, so the screen lets the server decide. The server refuses
    // the input with a mutation_ack carrying its client_msg_id, which settles
    // the send with that reason instead of retrying it into a timeout.
    const refusal = "zoo-005 is not running this project. Open the project again to start it there.";
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    mockSendAcknowledged.mockRejectedValueOnce(new MutationRefusedError(refusal));
    mockRandomUUID
      .mockReturnValueOnce("8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e01")
      .mockReturnValueOnce("8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e02");
    useMobileStore.setState({
      sessions: [session],
      features: { terminal: true, coding_mutations: true },
      negotiatedCapabilities: ["project_instances_v1"],
      instancesBySession: { [session.id]: [{ ...zoo006, primary: true }] },
      panesBySession: { [session.id]: [{ ...terminalPane, pane_id: 5, label: "Legacy 5", hostname: "zoo-005" }] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Legacy 5")).toBeTruthy());
    expect(view.queryByTestId("pane-unavailable")).toBeNull();
    fireEvent.changeText(view.getByPlaceholderText("Message this terminal conversation"), "Run the suite.");
    fireEvent.press(view.getByText("Send message"));

    await waitFor(() => expect(view.getByText(refusal)).toBeTruthy());
    expect(view.queryByText("The terminal conversation message was not acknowledged.")).toBeNull();
    expect(mockSendAcknowledged).toHaveBeenCalledTimes(1);
    expect(mockSendAcknowledged).toHaveBeenCalledWith(expect.objectContaining({
      type: "terminal_conversation_input",
      pane_id: 5,
      client_msg_id: expect.any(String),
    }), expect.any(String));
    // The draft stays, but the refused id is spent: the server records only
    // input it delivers, so the same id sent again once the machine runs
    // would be delivered as new input. Sending again is a new message.
    expect(view.getByText("Send message")).toBeTruthy();
    expect(view.queryByText("Retry message safely")).toBeNull();
    mockSendAcknowledged.mockResolvedValueOnce({ type: "user_input" });
    fireEvent.press(view.getByText("Send message"));
    await waitFor(() => expect(mockSendAcknowledged).toHaveBeenCalledTimes(2));
    const [first, second] = mockSendAcknowledged.mock.calls.map(([message]) => (message as { client_msg_id: string }).client_msg_id);
    expect(first).toBe("8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e01");
    expect(second).toBe("8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e02");
  });

  it("retries an unacknowledged message under the same id", async () => {
    // Nothing answered, so the first send may have landed: the retry must be
    // recognisable as the same message.
    mockParams = { sessionId: session.id };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    mockSendAcknowledged.mockRejectedValueOnce(new Error("The server did not acknowledge the action; retry with the same action identifier"));
    mockRandomUUID.mockReturnValueOnce("8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e03");
    useMobileStore.setState({
      sessions: [session],
      features: { terminal: true, coding_mutations: true },
      panesBySession: { [session.id]: [{ ...terminalPane, pane_id: 5, label: "Legacy 5" }] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Legacy 5")).toBeTruthy());
    fireEvent.changeText(view.getByPlaceholderText("Message this terminal conversation"), "Run the suite.");
    fireEvent.press(view.getByText("Send message"));
    await waitFor(() => expect(view.getByText("Retry message safely")).toBeTruthy());
    mockSendAcknowledged.mockResolvedValueOnce({ type: "user_input" });
    fireEvent.press(view.getByText("Retry message safely"));
    await waitFor(() => expect(mockSendAcknowledged).toHaveBeenCalledTimes(2));
    const ids = mockSendAcknowledged.mock.calls.map(([message]) => (message as { client_msg_id: string }).client_msg_id);
    expect(ids).toEqual(["8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e03", "8f2b7d64-1c1e-4f7a-9e55-2a6c3b0d9e03"]);
  });

  it("calls no machine stopped when the server did not negotiate instances", async () => {
    mockParams = { sessionId: session.id, paneId: "4" };
    mockReadCachedSnapshot.mockReturnValue(new Promise(() => undefined));
    useMobileStore.setState({
      sessions: [{ ...session, instances: [zoo005] }],
      features: { terminal: true, coding_mutations: true },
      panesBySession: { [session.id]: [zoo005Pane, zoo006Pane] },
    });
    const view = render(<SessionActivityScreen />);

    await waitFor(() => expect(view.getByText("✓ Codex 4 · zoo-006")).toBeTruthy());
    expect(view.queryByTestId("pane-unavailable")).toBeNull();
    fireEvent.changeText(view.getByPlaceholderText("Message this terminal conversation"), "Status?");
    expect(view.getByRole("button", { name: "Send message" })).not.toBeDisabled();
  });

  it("names the primary host and how many other machines run the project", async () => {
    useMobileStore.setState({
      sessions: [
        { ...session, project_name: "two-hosts", instances: [zoo005, zoo006] },
        {
          ...session,
          id: "9478d3c9-8527-437c-a7c4-93437f3a2e2f",
          project_id: "a70f916e-a160-4a82-9738-a63c76e2fc33",
          project_name: "one-host",
          hostname: "builder",
        },
      ],
    });
    const view = render(<CodeHomeScreen />);

    expect(view.getByText("zoo-005 +1")).toBeTruthy();
    expect(view.getByText("builder")).toBeTruthy();

    // The live set from session_instances replaces the bootstrap's.
    act(() => useMobileStore.getState().setSessionInstances(session.id, [
      zoo005,
      zoo006,
      { machine_id: "50000000-0000-4000-8000-000000000003", hostname: "laptop", primary: false },
    ]));
    await waitFor(() => expect(view.getByText("zoo-005 +2")).toBeTruthy());
  });

  it("sends the chosen machine and shows the server's reason when it refuses that target", async () => {
    useMobileStore.setState({
      launchTargets: [launchTarget],
      features: { coding_mutations: true },
      sessions: [session],
    });
    mockLaunchTask.mockRejectedValueOnce(new Error("builder cannot run this project beside the instance on zoo-005; reboot builder to update it"));
    const view = render(<NewTaskScreen />);
    await waitFor(() => expect(mockLoadTaskDraft).toHaveBeenCalled());

    fireEvent.press(view.getByText(launchTarget.project_name));
    fireEvent.press(view.getByText("Codex terminal"));
    fireEvent.changeText(view.getByPlaceholderText(/diagnose the failing login test/), "Run the slow suite here");
    fireEvent.press(view.getByText("Review task"));
    fireEvent.press(view.getByText("Submit task"));

    await waitFor(() => expect(view.getByText("builder cannot run this project beside the instance on zoo-005; reboot builder to update it")).toBeTruthy());
    expect(mockLaunchTask).toHaveBeenCalledTimes(1);
    expect(mockLaunchTask.mock.calls[0]?.[0]).toMatchObject({
      machine_id: launchTarget.machine_id,
      project_id: launchTarget.project_id,
      profile_key: "terminal:codex:official:default",
    });
    expect(mockDeleteTaskDraft).not.toHaveBeenCalled();
  });

  it("shows the server's reason as it is when a launch is refused for a closed project with tabs elsewhere", async () => {
    // A launch names one machine and never opens a project on another. This
    // app has no Open control, so the server's sentence is all it shows.
    const refusal = "This project is closed and has tabs on other machines. Open it from the project.";
    useMobileStore.setState({
      launchTargets: [launchTarget],
      features: { coding_mutations: true },
      sessions: [{ ...session, is_active: false, open_state: "closed" }],
    });
    mockLaunchTask.mockRejectedValueOnce(new Error(refusal));
    jest.mocked(router.replace).mockClear();
    const view = render(<NewTaskScreen />);
    await waitFor(() => expect(mockLoadTaskDraft).toHaveBeenCalled());

    fireEvent.press(view.getByText(launchTarget.project_name));
    fireEvent.press(view.getByText("Codex terminal"));
    fireEvent.changeText(view.getByPlaceholderText(/diagnose the failing login test/), "Run the slow suite here");
    fireEvent.press(view.getByText("Review task"));
    fireEvent.press(view.getByText("Submit task"));

    await waitFor(() => expect(view.getByText(refusal)).toBeTruthy());
    expect(mockLaunchTask).toHaveBeenCalledTimes(1);
    expect(mockLaunchTask.mock.calls[0]?.[0]).toMatchObject({
      machine_id: launchTarget.machine_id,
      project_id: launchTarget.project_id,
    });
    // Nothing was started, so nothing is opened and the draft is kept.
    expect(router.replace).not.toHaveBeenCalled();
    expect(mockDeleteTaskDraft).not.toHaveBeenCalled();
    expect(view.getByText("Run the slow suite here")).toBeTruthy();
  });

});
