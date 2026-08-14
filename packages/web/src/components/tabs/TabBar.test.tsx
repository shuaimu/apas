import { createEvent, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";
import type { CliLifecycleInventory, CliLifecycleStatus, PaneConfig } from "@/lib/store";

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label" | "role">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: "deadloop",
    session_id: `pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
    role: overrides.role,
    managed: overrides.managed,
    model: overrides.model,
  };
}

function renderTabBar(
  overrides: {
    activeTabId?: number;
    onSelectTab?: ReturnType<typeof vi.fn>;
    onAddTab?: ReturnType<typeof vi.fn>;
    onReconnectCli?: ReturnType<typeof vi.fn>;
    onRebootCli?: ReturnType<typeof vi.fn>;
    lifecycleInventory?: CliLifecycleInventory;
    lifecycleStatus?: CliLifecycleStatus;
    onRenameTab?: ReturnType<typeof vi.fn>;
    onReorderTabs?: ReturnType<typeof vi.fn>;
    showRebootButton?: boolean;
    showOverview?: boolean;
    tabs?: PaneConfig[];
  } = {},
) {
  const onSelectTab = overrides.onSelectTab ?? vi.fn();
  const onCloseTab = vi.fn();
  const onAddTab = overrides.onAddTab ?? vi.fn();
  const onRenameTab = overrides.onRenameTab ?? vi.fn();
  const onReorderTabs = overrides.onReorderTabs ?? vi.fn();
  const tabs = overrides.tabs ?? [
    pane({ pane_id: 10, label: "Managed Manager", role: "team manager", managed: true }),
    pane({ pane_id: 11, label: "Managed Tech Lead", role: "tech lead", managed: true }),
    pane({ pane_id: 12, label: "Side Manager", role: "manager", managed: false }),
    pane({ pane_id: 13, label: "Side Tech Lead", role: "tech lead", managed: false }),
  ];

  const result = render(
    <TabBar
      tabs={tabs}
      activeTabId={overrides.activeTabId ?? 12}
      onSelectTab={onSelectTab as Parameters<typeof TabBar>[0]["onSelectTab"]}
      onCloseTab={onCloseTab}
      onAddTab={onAddTab as Parameters<typeof TabBar>[0]["onAddTab"]}
      onRenameTab={onRenameTab as Parameters<typeof TabBar>[0]["onRenameTab"]}
      onReorderTabs={onReorderTabs as Parameters<typeof TabBar>[0]["onReorderTabs"]}
      onReconnectCli={overrides.onReconnectCli as Parameters<typeof TabBar>[0]["onReconnectCli"]}
      onRebootCli={overrides.onRebootCli as Parameters<typeof TabBar>[0]["onRebootCli"]}
      lifecycleInventory={overrides.lifecycleInventory}
      lifecycleStatus={overrides.lifecycleStatus}
      showRebootButton={overrides.showRebootButton}
      showOverview={overrides.showOverview}
      paneStatuses={{}}
      pausedPanes={[]}
    />,
  );

  return { ...result, onAddTab, onCloseTab, onRenameTab, onReorderTabs, onSelectTab };
}

function closeButton(container: HTMLElement, paneId: number): Element | null {
  return container.querySelector(`[data-tab-id="${paneId}"] [title="Close tab"]`);
}

function tabButton(container: HTMLElement, paneId: number): Element {
  const tab = container.querySelector(`[data-tab-id="${paneId}"]`);
  expect(tab).toBeTruthy();
  return tab as Element;
}

function basicTabs(): PaneConfig[] {
  return [
    pane({ pane_id: 1, label: "Alpha", role: "developer" }),
    pane({ pane_id: 2, label: "Beta", role: "reviewer" }),
    pane({ pane_id: 3, label: "Gamma", role: "qa" }),
  ];
}

function dataTransfer() {
  return {
    dropEffect: "",
    effectAllowed: "",
    getData: vi.fn(),
    setData: vi.fn(),
  };
}

function dragOverWithClientX(element: Element, clientX: number) {
  const event = createEvent.dragOver(element, { dataTransfer: dataTransfer() });
  Object.defineProperty(event, "clientX", { value: clientX });
  fireEvent(element, event);
}

function mockRect(element: Element, left: number, width: number) {
  Object.defineProperty(element, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      bottom: 40,
      height: 30,
      left,
      right: left + width,
      top: 10,
      width,
      x: left,
      y: 10,
      toJSON: () => ({}),
    }),
  });
}

function openNewTabMenu() {
  fireEvent.click(screen.getByTitle("New tab"));
}

function isolatedWorktreeCheckbox(): HTMLInputElement {
  return screen.getByLabelText("Isolated git worktree") as HTMLInputElement;
}

describe("TabBar coordinator close controls", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  it("hides close buttons for managed coordinators while leaving unmanaged role-like tabs closable", () => {
    const { container, onCloseTab } = renderTabBar();

    expect(closeButton(container, 10)).toBeNull();
    expect(closeButton(container, 11)).toBeNull();
    expect(closeButton(container, 12)).toBeTruthy();
    expect(closeButton(container, 13)).toBeTruthy();

    fireEvent.click(closeButton(container, 12) as Element);

    expect(onCloseTab).toHaveBeenCalledWith(12);
  });

  it("keeps context-menu close hidden only for managed coordinators", () => {
    const { container, onCloseTab } = renderTabBar();

    fireEvent.contextMenu(tabButton(container, 10));
    expect(screen.queryByText("Close")).toBeNull();

    fireEvent.contextMenu(tabButton(container, 13));
    fireEvent.click(screen.getByText("Close"));

    expect(onCloseTab).toHaveBeenCalledWith(13);
  });

  it("offers all verified terminal panes for new tabs", () => {
    const onAddTab = vi.fn();
    renderTabBar({ onAddTab });

    fireEvent.click(screen.getByTitle("New tab"));

    expect(screen.getByText("Claude")).toBeTruthy();
    expect(screen.getByText("Codex")).toBeTruthy();
    expect(screen.getByText("OpenCode")).toBeTruthy();
    expect(screen.queryByText("Cursor")).toBeNull();
    expect(screen.queryByText("DeepSeek")).toBeNull();

    fireEvent.click(screen.getByText("Claude"));
    expect(onAddTab).toHaveBeenCalledWith("claude", undefined, undefined, "terminal");
  });

  it("keeps full-process reboot behind the desktop lifecycle menu", () => {
    const onRebootCli = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderTabBar({ onRebootCli, showRebootButton: true });

    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    fireEvent.click(screen.getByText("Reboot CLI"));

    expect(confirmSpy.mock.calls[0][0]).toContain("older project CLI");
    expect(onRebootCli).toHaveBeenCalledOnce();
    confirmSpy.mockRestore();
  });

  it("does not render Reboot CLI without both the full-process action and visibility flag", () => {
    const onRebootCli = vi.fn();

    renderTabBar({ onRebootCli, showRebootButton: false });
    expect(screen.queryByText("Reboot CLI")).toBeNull();

    renderTabBar({ showRebootButton: true });
    expect(screen.queryByText("Reboot CLI")).toBeNull();
  });

  it("does not reboot the CLI when confirmation is declined", () => {
    const onRebootCli = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderTabBar({ onRebootCli, showRebootButton: true });

    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    fireEvent.click(screen.getByText("Reboot CLI"));

    expect(confirmSpy.mock.calls[0][0]).toContain("older project CLI");
    expect(onRebootCli).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("offers recommended transport reconnect without pane-reboot language", () => {
    const onReconnectCli = vi.fn();
    const onRebootCli = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderTabBar({
      onReconnectCli,
      onRebootCli,
      showRebootButton: true,
      lifecycleInventory: {
        reconnect_transport: true,
        persistent_terminal_hosting: true,
        panes: [],
      },
    });

    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    expect(screen.getByText("Recommended")).toBeTruthy();
    fireEvent.click(screen.getByText("Reconnect Server"));

    expect(confirmSpy.mock.calls[0][0]).toContain("every pane will keep running");
    expect(confirmSpy.mock.calls[0][0]).not.toContain("restart");
    expect(onReconnectCli).toHaveBeenCalledOnce();
    expect(onRebootCli).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("enumerates preservation consequences before reboot", () => {
    const onRebootCli = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderTabBar({
      onRebootCli,
      showRebootButton: true,
      lifecycleInventory: {
        reconnect_transport: true,
        persistent_terminal_hosting: true,
        panes: [
          { pane_id: 1, mode: "live_adoptable" },
          { pane_id: 2, mode: "restart_required_on_cli_reboot" },
          { pane_id: 3, mode: "structured_pane_may_resume" },
        ],
      },
    });

    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    fireEvent.click(screen.getByText("Reboot CLI"));

    const warning = String(confirmSpy.mock.calls[0][0]);
    expect(warning).toContain("1 terminal pane will stay live");
    expect(warning).toContain("1 terminal pane will restart");
    expect(warning).toContain("1 structured pane may restart");
    expect(onRebootCli).toHaveBeenCalledOnce();
    confirmSpy.mockRestore();
  });

  it("shows correlated reconnect progress without reboot labels", () => {
    renderTabBar({
      onReconnectCli: vi.fn(),
      onRebootCli: vi.fn(),
      showRebootButton: true,
      lifecycleInventory: {
        reconnect_transport: true,
        persistent_terminal_hosting: true,
        panes: [],
      },
      lifecycleStatus: {
        sessionId: "session-a",
        requestId: "request-a",
        operation: "reconnect_transport",
        phase: "reconnecting",
        message: "Reconnecting server transport",
        startedAt: 1,
        updatedAt: 2,
      },
    });

    fireEvent.click(screen.getByTitle("Project CLI lifecycle actions"));
    expect(screen.getByRole("status").textContent).toContain("Reconnecting server transport");
    expect(screen.getByRole("status").textContent).not.toContain("reboot");
  });
});

describe("TabBar Overview visibility", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  it("omits Overview while preserving real pane tabs when team mode is unavailable", () => {
    const { container } = renderTabBar({
      showOverview: false,
      tabs: basicTabs(),
      activeTabId: 1,
    });

    expect(screen.queryByRole("button", { name: "Overview" })).toBeNull();
    expect(tabButton(container, 1).textContent).toContain("Alpha");
  });
});

describe("TabBar rename and reorder controls", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  it("renames tabs from the context menu with Enter and blur, and cancels with Escape", () => {
    const onRenameTab = vi.fn();
    const { container } = renderTabBar({
      activeTabId: 1,
      onRenameTab,
      tabs: basicTabs(),
    });

    fireEvent.contextMenu(tabButton(container, 1), { clientX: 10, clientY: 20 });
    fireEvent.click(screen.getByText("Rename"));
    const enterInput = screen.getByDisplayValue("Alpha");
    fireEvent.change(enterInput, { target: { value: "  Alpha Prime  " } });
    fireEvent.keyDown(enterInput, { key: "Enter" });

    expect(onRenameTab).toHaveBeenCalledWith(1, "Alpha Prime");

    fireEvent.contextMenu(tabButton(container, 2), { clientX: 10, clientY: 20 });
    fireEvent.click(screen.getByText("Rename"));
    const blurInput = screen.getByDisplayValue("Beta");
    fireEvent.change(blurInput, { target: { value: "Beta Prime" } });
    fireEvent.blur(blurInput);

    expect(onRenameTab).toHaveBeenCalledWith(2, "Beta Prime");

    onRenameTab.mockClear();
    fireEvent.contextMenu(tabButton(container, 3), { clientX: 10, clientY: 20 });
    fireEvent.click(screen.getByText("Rename"));
    const escapeInput = screen.getByDisplayValue("Gamma");
    fireEvent.change(escapeInput, { target: { value: "Gamma Prime" } });
    fireEvent.keyDown(escapeInput, { key: "Escape" });

    expect(onRenameTab).not.toHaveBeenCalled();
  });

  it("selects tabs only when the tab is not being renamed", () => {
    const onSelectTab = vi.fn();
    const { container } = renderTabBar({
      activeTabId: 1,
      onSelectTab,
      tabs: basicTabs(),
    });

    fireEvent.click(tabButton(container, 2));

    expect(onSelectTab).toHaveBeenCalledWith(2);

    fireEvent.contextMenu(tabButton(container, 1), { clientX: 10, clientY: 20 });
    fireEvent.click(screen.getByText("Rename"));
    fireEvent.click(screen.getByDisplayValue("Alpha"));

    expect(onSelectTab).toHaveBeenCalledTimes(1);
  });

  it("reorders tabs by dropping before or after a target tab", () => {
    const onReorderTabs = vi.fn();
    const { container } = renderTabBar({
      activeTabId: 1,
      onReorderTabs,
      tabs: basicTabs(),
    });

    const alpha = tabButton(container, 1);
    const gamma = tabButton(container, 3);
    mockRect(alpha, 100, 100);
    mockRect(gamma, -300, 100);

    fireEvent.dragStart(gamma, { dataTransfer: dataTransfer() });
    dragOverWithClientX(alpha, -1);
    fireEvent.drop(alpha, { dataTransfer: dataTransfer() });

    expect(onReorderTabs).toHaveBeenCalledWith([3, 1, 2]);

    fireEvent.dragStart(alpha, { dataTransfer: dataTransfer() });
    dragOverWithClientX(gamma, 380);
    fireEvent.drop(gamma, { dataTransfer: dataTransfer() });

    expect(onReorderTabs).toHaveBeenLastCalledWith([2, 3, 1]);
  });
});

describe("TabBar add-tab worktree controls", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  it("passes undefined isolated-worktree mode for an untouched default provider pick", () => {
    const onAddTab = vi.fn();
    renderTabBar({ onAddTab });

    openNewTabMenu();
    fireEvent.click(screen.getByText("Claude"));

    expect(onAddTab).toHaveBeenCalledWith("claude", undefined, undefined, "terminal");
  });

  it("passes true when isolated worktree is checked and resets it after picking", () => {
    const onAddTab = vi.fn();
    renderTabBar({ onAddTab });

    openNewTabMenu();
    fireEvent.click(isolatedWorktreeCheckbox());
    fireEvent.click(screen.getByText("Claude"));

    expect(onAddTab).toHaveBeenCalledWith("claude", undefined, true, "terminal");

    openNewTabMenu();
    expect(isolatedWorktreeCheckbox().checked).toBe(false);
    fireEvent.click(screen.getByText("Codex"));

    expect(onAddTab).toHaveBeenLastCalledWith("codex", undefined, undefined, "terminal");
  });

  it("offers verified terminal providers and passes kind=terminal", () => {
    const onAddTab = vi.fn();
    renderTabBar({ onAddTab });

    openNewTabMenu();
    fireEvent.click(screen.getByText("OpenCode"));

    expect(onAddTab).toHaveBeenCalledWith("opencode", undefined, undefined, "terminal");
  });

  it("does not offer a terminal tab for providers a pty can't host", () => {
    // Mirrors `terminal_binary_for` in the CLI.
    renderTabBar();
    openNewTabMenu();

    expect(screen.getByText("Claude")).toBeTruthy();
    expect(screen.getByText("Codex")).toBeTruthy();
    expect(screen.getByText("OpenCode")).toBeTruthy();
    for (const label of ["MiniMax", "GLM", "DeepSeek", "Cursor"]) {
      expect(screen.queryByText(label)).toBeNull();
    }
  });

  it("marks a historical retired tab unsupported without branding it", () => {
    const { container } = renderTabBar({
      tabs: [pane({
        pane_id: 81,
        label: "Historical",
        role: "developer",
        provider: "minimax",
        model: "MiniMax-M2.7",
      })],
      activeTabId: 81,
    });

    const tab = tabButton(container, 81);
    expect(tab.querySelector('[title="Unsupported provider"]')).toBeTruthy();
    expect(tab.textContent).toContain("!");
  });

  it("closes the menu from an outside click", () => {
    renderTabBar();

    openNewTabMenu();
    expect(screen.getByText("Claude")).toBeTruthy();

    fireEvent.mouseDown(document.body);

    expect(screen.queryByText("Claude")).toBeNull();
  });
});
