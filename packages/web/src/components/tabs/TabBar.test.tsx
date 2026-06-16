import { fireEvent, render, screen } from "@testing-library/react";
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
    onAddTab?: ReturnType<typeof vi.fn>;
    onRebootCli?: ReturnType<typeof vi.fn>;
    showRebootButton?: boolean;
  } = {},
) {
  const onCloseTab = vi.fn();
  const onAddTab = overrides.onAddTab ?? vi.fn();
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
      onAddTab={onAddTab}
      onRebootCli={overrides.onRebootCli}
      showRebootButton={overrides.showRebootButton}
      paneStatuses={{}}
      pausedPanes={[]}
    />,
  );

  return { ...result, onAddTab, onCloseTab };
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
