import { createEvent, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";
import type { PaneConfig } from "@/lib/store";
import {
  DEEPSEEK_DEFAULT_MODEL,
  PROVIDER_MODEL_GROUPS,
} from "@/lib/providerOptions";

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label" | "role">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: "claude",
    mode: "deadloop",
    session_id: `pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
    role: overrides.role,
    managed: overrides.managed,
  };
}

function renderTabBar(
  overrides: {
    activeTabId?: number;
    onSelectTab?: ReturnType<typeof vi.fn>;
    onAddTab?: ReturnType<typeof vi.fn>;
    onRebootCli?: ReturnType<typeof vi.fn>;
    onRenameTab?: ReturnType<typeof vi.fn>;
    onReorderTabs?: ReturnType<typeof vi.fn>;
    showRebootButton?: boolean;
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
      onSelectTab={onSelectTab}
      onCloseTab={onCloseTab}
      onAddTab={onAddTab}
      onRenameTab={onRenameTab}
      onReorderTabs={onReorderTabs}
      onRebootCli={overrides.onRebootCli}
      showRebootButton={overrides.showRebootButton}
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

  it("renders the add-tab provider menu from shared provider/model groups", () => {
    const onAddTab = vi.fn();
    renderTabBar({ onAddTab });

    fireEvent.click(screen.getByTitle("New tab"));

    for (const group of PROVIDER_MODEL_GROUPS) {
      expect(screen.getByText(group.label)).toBeTruthy();
    }

    const claudeGroup = PROVIDER_MODEL_GROUPS.find((group) => group.id === "claude");
    expect(claudeGroup).toBeTruthy();
    fireEvent.click(screen.getByText("Claude"));

    for (const option of claudeGroup?.options ?? []) {
      expect(screen.getByText(option.label)).toBeTruthy();
    }

    fireEvent.click(screen.getByText("DeepSeek"));

    expect(onAddTab).toHaveBeenCalledWith(
      "claude",
      DEEPSEEK_DEFAULT_MODEL,
      undefined,
    );
  });

  it("keeps full-process reboot behind the explicit Reboot CLI control", () => {
    const onRebootCli = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderTabBar({ onRebootCli, showRebootButton: true });

    fireEvent.click(screen.getByText("Reboot CLI"));

    expect(confirmSpy).toHaveBeenCalledWith("Are you sure you want to reboot the CLI?");
    expect(onRebootCli).toHaveBeenCalledOnce();
    confirmSpy.mockRestore();
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
