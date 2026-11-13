import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { TabbedView } from "./TabbedView";

const initialStore = useStore.getInitialState();
beforeEach(() => { Element.prototype.scrollIntoView = vi.fn(); });
afterEach(() => { cleanup(); localStorage.clear(); act(() => useStore.setState(initialStore, true)); vi.restoreAllMocks(); });

it.each([false, true])("retains legacy history read-only with stop_requested=%s and no Stop Bot or force-stop send", async (stopRequested) => {
  const send = vi.fn();
  const sendMessageToPane = vi.fn(() => ({ success: true }));
  localStorage.setItem("apas_layout_cli-history_active_tab", "42");
  act(() => useStore.setState({
    connected: true, isAttached: true, isDualPane: true, sessionId: "history", cliClientId: "cli-history",
    ws: { readyState: WebSocket.OPEN, send } as unknown as WebSocket,
    paneConfigs: [{ pane_id: 42, provider: "claude", mode: "deadloop", session_id: "pane-history", is_paused: false, label: "Earlier bot", stop_requested: stopRequested, prompt: "Keep original source" }],
    paneListReceived: true, paneMessages: { "42": [{ id: "old-output", role: "assistant", content: "Retained execution evidence", timestamp: new Date(), outputType: { type: "text" } }] }, paneStatuses: {}, paneModes: {}, pausedPanes: [], messages: [],
    loadPaneMessagesIfNeeded: vi.fn(), loadMoreMessages: vi.fn(), sendMessageToPane, projectPolicies: {},
  }));
  render(<TabbedView />);
  expect(await screen.findByText(/Legacy bot history is read-only/)).toBeTruthy();
  expect(screen.getByText("Retained execution evidence")).toBeTruthy();
  expect(screen.queryByRole("button", { name: /^(Stop Bot|Force Stop|Bot)$/ })).toBeNull();
  send.mockClear();
  fireEvent.click(screen.getByRole("button", { name: "Project overview" }));
  fireEvent.click(screen.getByRole("button", { name: "Back to interactive agents" }));
  expect(send).not.toHaveBeenCalled();
  expect(sendMessageToPane).not.toHaveBeenCalled();
  expect(useStore.getState().paneConfigs[0].prompt).toBe("Keep original source");
  expect(useStore.getState().sessionId).toBe("history");
});
