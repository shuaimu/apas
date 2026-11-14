import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { TabbedView } from "./TabbedView";

const initialStore = useStore.getInitialState();
beforeEach(() => { Element.prototype.scrollIntoView = vi.fn(); });
afterEach(() => { cleanup(); localStorage.clear(); act(() => useStore.setState(initialStore, true)); vi.restoreAllMocks(); });

it("keeps saved bot instructions as data without offering a new bot or sending on Overview navigation", async () => {
  const send = vi.fn();
  localStorage.setItem("apas_layout_cli-cutover_active_tab", "42");
  act(() => useStore.setState({
    connected: true, isAttached: true, isDualPane: true, sessionId: "cutover", cliClientId: "cli-cutover",
    ws: { readyState: WebSocket.OPEN, send } as unknown as WebSocket,
    paneConfigs: [{ pane_id: 42, provider: "claude", mode: "interactive", session_id: "pane-cutover", is_paused: false, label: "Retained pane", prompt: "Original instructions", effort: "high" }],
    paneListReceived: true, paneMessages: { "42": [] }, paneStatuses: {}, paneModes: {}, pausedPanes: [], messages: [],
    loadPaneMessagesIfNeeded: vi.fn(), loadMoreMessages: vi.fn(), projectPolicies: {},
  }));
  render(<TabbedView />);
  await screen.findByText("Retained pane");
  expect(screen.queryByTitle("Start autonomous bot execution in this tab")).toBeNull();
  expect(screen.queryByRole("button", { name: /^(Start Bot|Bot)$/ })).toBeNull();
  expect(screen.queryByPlaceholderText("Enter bot loop prompt...")).toBeNull();
  send.mockClear();
  fireEvent.click(screen.getByRole("button", { name: "Project overview" }));
  expect(await screen.findByRole("region", { name: "Project overview" })).toBeTruthy();
  // The Overview has no button back to the tabs: the tab bar selects one.
  expect(screen.queryByRole("button", { name: "Back to interactive agents" })).toBeNull();
  fireEvent.click(document.querySelector<HTMLButtonElement>('button[data-tab-id="42"]')!);
  expect(screen.queryByRole("region", { name: "Project overview" })).toBeNull();
  expect(send).not.toHaveBeenCalled();
  expect(useStore.getState().paneConfigs[0]).toMatchObject({ prompt: "Original instructions", effort: "high", mode: "interactive" });
  expect(useStore.getState().sessionId).toBe("cutover");
});
