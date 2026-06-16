import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";
import type { PaneConfig } from "@/lib/store";

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

function renderTabBar() {
  const onCloseTab = vi.fn();
  const tabs = [
    pane({ pane_id: 10, label: "Managed Manager", role: "team manager", managed: true }),
    pane({ pane_id: 11, label: "Managed Tech Lead", role: "tech lead", managed: true }),
    pane({ pane_id: 12, label: "Side Manager", role: "manager", managed: false }),
    pane({ pane_id: 13, label: "Side Tech Lead", role: "tech lead", managed: false }),
  ];

  const result = render(
    <TabBar
      tabs={tabs}
      activeTabId={12}
      onSelectTab={vi.fn()}
      onCloseTab={onCloseTab}
      onAddTab={vi.fn()}
      paneStatuses={{}}
      pausedPanes={[]}
    />,
  );

  return { ...result, onCloseTab };
}

function closeButton(container: HTMLElement, paneId: number): Element | null {
  return container.querySelector(`[data-tab-id="${paneId}"] [title="Close tab"]`);
}

function tabButton(container: HTMLElement, paneId: number): Element {
  const tab = container.querySelector(`[data-tab-id="${paneId}"]`);
  expect(tab).toBeTruthy();
  return tab as Element;
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
});
