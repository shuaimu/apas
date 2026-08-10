import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneGrid } from "./PaneGrid";
import { useStore, type PaneConfig } from "@/lib/store";

const initialStore = useStore.getState();

function pane(
  overrides: Partial<PaneConfig> &
    Pick<PaneConfig, "pane_id" | "label" | "managed">,
): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "deadloop",
    session_id: `pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
    role: overrides.role,
    goal: overrides.goal,
    backstory: overrides.backstory,
    managed: overrides.managed,
    model: overrides.model,
    kind: overrides.kind,
  };
}

function seedPaneGrid(paneConfigs: PaneConfig[], send = vi.fn()) {
  useStore.setState({
    sessionId: "session-123",
    ws: {
      readyState: WebSocket.OPEN,
      send,
    } as unknown as WebSocket,
    paneConfigs,
    paneStatuses: {},
    pausedPanes: [],
    paneMessages: {},
    paneDiffs: {},
    updatePaneManualMode: vi.fn(),
    updatePaneModel: vi.fn(),
  });
  return send;
}

function renderPaneGrid(kind: "managed" | "unmanaged") {
  return render(
    <PaneGrid
      kind={kind}
      onOpenPane={vi.fn()}
      onOpenDiff={vi.fn()}
      onOpenRole={vi.fn()}
      onPausePane={vi.fn()}
      onResumePane={vi.fn()}
      onRemovePane={vi.fn()}
    />,
  );
}

function card(label: string): HTMLElement {
  return screen.getByTitle(`Open ${label}`);
}

describe("PaneGrid side-chat promotion", () => {
  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders Add to team for unmanaged side chats and sends a promotion request", () => {
    const send = seedPaneGrid([
      pane({
        pane_id: 20,
        label: "Side Developer",
        role: "developer",
        goal: "Implement a leaf task",
        backstory: "Manually added helper",
        managed: false,
      }),
    ]);

    renderPaneGrid("unmanaged");

    const sideChatCard = card("Side Developer");
    const promote = within(sideChatCard).getByText("+ Add to team");
    expect(promote).toBeTruthy();

    fireEvent.click(promote);

    expect(send).toHaveBeenCalledWith(
      JSON.stringify({
        type: "promote_pane_to_managed",
        session_id: "session-123",
        pane_id: 20,
      }),
    );
  });

  it("does not offer terminal panes as managed-team workers", () => {
    const send = seedPaneGrid([
      pane({
        pane_id: 21,
        label: "Claude terminal",
        managed: false,
        kind: "terminal",
      }),
    ]);

    renderPaneGrid("unmanaged");

    expect(within(card("Claude terminal")).queryByText("+ Add to team")).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("does not render Add to team for managed cards", () => {
    const send = seedPaneGrid([
      pane({
        pane_id: 30,
        label: "Managed Developer",
        role: "developer",
        managed: true,
      }),
    ]);

    renderPaneGrid("managed");

    expect(
      within(card("Managed Developer")).queryByText("+ Add to team"),
    ).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });
});
