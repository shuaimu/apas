import { render, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";
import type { MachineWithProjects, PaneConfig } from "@/lib/store";
import type { AutopilotListStatus } from "@/lib/autopilot";
import { describeTabHosts } from "@/lib/tabHosts";

const PROJECT = "30000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000005";
const ZOO6 = "50000000-0000-4000-8000-000000000006";

function tab(pane_id: number, label: string, machine_id: string, hostname: string): PaneConfig {
  return {
    pane_id,
    provider: "omp",
    mode: "interactive",
    kind: "terminal",
    session_id: `pane-${pane_id}`,
    is_paused: false,
    label,
    machine_id,
    hostname,
  };
}

function machine(machineId: string, hostname: string): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x86_64" },
    projects: [{ projectId: PROJECT, path: "/work/project", isRunning: true }],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
  };
}

const TABS = [tab(1, "OMP 1", ZOO5, "zoo-005"), tab(2, "OMP 2", ZOO6, "zoo-006")];

function renderBar(autopilotTabs: Map<number, AutopilotListStatus> | undefined, multiMachine: boolean) {
  const tabHosts = multiMachine
    ? describeTabHosts({
        panes: TABS,
        instances: [
          { machineId: ZOO5, hostname: "zoo-005", primary: true },
          { machineId: ZOO6, hostname: "zoo-006", primary: false },
        ],
        machines: [machine(ZOO5, "zoo-005"), machine(ZOO6, "zoo-006")],
        instancesSupported: true,
        projectIds: [PROJECT],
        isOwner: true,
      }).tabs
    : undefined;
  return render(
    <TabBar
      tabs={TABS}
      activeTabId={1}
      paneStatuses={{}}
      pausedPanes={[]}
      onSelectTab={vi.fn()}
      onCloseTab={vi.fn()}
      onAddTab={vi.fn()}
      tabHosts={tabHosts}
      autopilotTabs={autopilotTabs}
    />,
  );
}

function tabElement(container: HTMLElement, paneId: number): HTMLElement {
  return container.querySelector(`[data-tab-id="${paneId}"]`) as HTMLElement;
}

beforeEach(() => {
  // jsdom has no layout engine; TabBar scrolls the active tab into view.
  Element.prototype.scrollIntoView = vi.fn();
});

describe("TabBar autopilot marker", () => {
  const ON = new Map<number, AutopilotListStatus>([[1, { kind: "on", text: "Running an iteration" }]]);

  it("gives a tab on autopilot a marker and a spelled-out name", () => {
    const { container } = renderBar(ON, false);
    const marked = tabElement(container, 1);
    expect(marked.getAttribute("aria-label")).toBe("OMP 1, on autopilot");
    const marker = within(marked).getByTestId("tab-autopilot-marker");
    // A glyph only: the name is on the tab, the detail in the tooltip.
    expect(marker.getAttribute("aria-hidden")).toBe("true");
    expect(marker.getAttribute("title")).toBe("Autopilot · Running an iteration");
  });

  it("keeps the host in the name of a tab on autopilot once hosts are shown", () => {
    const { container } = renderBar(ON, true);
    expect(tabElement(container, 1).getAttribute("aria-label")).toBe("OMP 1, on autopilot — Runs on zoo-005");
    expect(tabElement(container, 2).getAttribute("aria-label")).toBe("OMP 2 — Runs on zoo-006");
  });

  it("names tabs that are not on autopilot exactly as a bar without autopilot does", () => {
    for (const multiMachine of [false, true]) {
      const plain = renderBar(undefined, multiMachine);
      const expected = TABS.map((item) => ({
        label: tabElement(plain.container, item.pane_id).getAttribute("aria-label"),
        html: tabElement(plain.container, item.pane_id).outerHTML,
      }));
      plain.unmount();

      const withAutopilot = renderBar(ON, multiMachine);
      // Tab 2 is not on autopilot: nothing about it changed, markup included.
      expect(tabElement(withAutopilot.container, 2).getAttribute("aria-label")).toBe(expected[1].label);
      expect(tabElement(withAutopilot.container, 2).outerHTML).toBe(expected[1].html);
      withAutopilot.unmount();

      // An ended autopilot leaves its tab unmarked and its name unchanged.
      const ended = renderBar(new Map([[1, { kind: "ended", text: "Stopped" }]]), multiMachine);
      expect(tabElement(ended.container, 1).outerHTML).toBe(expected[0].html);
      ended.unmount();
    }
  });

  it("keeps the marker, dimmed, while autopilot is paused", () => {
    const { container } = renderBar(new Map([[1, { kind: "paused", text: "Paused: the turn was interrupted" }]]), false);
    const marked = tabElement(container, 1);
    expect(marked.getAttribute("aria-label")).toBe("OMP 1, autopilot paused");
    const marker = within(marked).getByTestId("tab-autopilot-marker");
    expect(marker.getAttribute("data-autopilot")).toBe("paused");
    expect(marker.className).toContain("opacity-60");
    expect(marker.getAttribute("title")).toBe("Autopilot · Paused: the turn was interrupted");
  });
});
