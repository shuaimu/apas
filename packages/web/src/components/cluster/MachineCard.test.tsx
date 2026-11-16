import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MachineProject, MachineWithProjects } from "@/lib/store";
import { MachineCard } from "./MachineCard";

afterEach(cleanup);

function entry(extra: Partial<MachineWithProjects> = {}, projects: MachineProject[] = []): MachineWithProjects {
  return {
    machine: { machineId: "m1", hostname: "zoo-005", os: "linux", arch: "x64", daemonVersion: "26.11.40", lastSeen: "2026-11-15T10:00:00Z" },
    projects,
    ...extra,
  };
}

function renderCard(machine: MachineWithProjects, { behind = false, stoppedOpen = false, openStateSupported = false } = {}) {
  const onToggleStopped = vi.fn();
  render(
    <MachineCard
      entry={machine}
      behind={behind}
      latestVersion="26.11.53"
      showReboot
      connected
      pending={[]}
      stoppedOpen={stoppedOpen}
      onToggleStopped={onToggleStopped}
      openStateSupported={openStateSupported}
      requestConfirm={vi.fn()}
      onReboot={vi.fn()}
      onOpen={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  return { onToggleStopped };
}

const stopped = (name: string): MachineProject => ({ projectId: name, name, path: `/srv/${name}`, isRunning: false });

describe("MachineCard", () => {
  it("shows one amber chip, Update available before Older APAS", () => {
    renderCard(entry({ daemonConnected: true, projectInstancesAvailable: false }), { behind: true });
    const chip = screen.getByText("Update available");
    expect(chip.getAttribute("title")).toBe("This machine runs 26.11.40. The newest version seen is 26.11.53.");
    expect(screen.queryByText("Older APAS")).toBeNull();
    cleanup();

    renderCard(entry({ daemonConnected: true, projectInstancesAvailable: false }));
    expect(screen.getByText("Older APAS").getAttribute("title")).toBe(
      "zoo-005 runs an older APAS that cannot run a project beside other machines. Reboot zoo-005 to update it.",
    );
    cleanup();

    renderCard(entry({ daemonConnected: true, projectInstancesAvailable: true }));
    expect(screen.queryByText("Older APAS")).toBeNull();
    expect(screen.queryByText("Update available")).toBeNull();
  });

  it("says Connected or Offline, and Last seen only when not connected", () => {
    renderCard(entry({ daemonConnected: true }));
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(screen.queryByText(/Last seen/)).toBeNull();
    cleanup();

    renderCard(entry({ daemonConnected: false }));
    expect(screen.getByText("Offline")).toBeTruthy();
    expect(screen.getByText(/Last seen/)).toBeTruthy();
    cleanup();

    renderCard(entry());
    expect(screen.queryByText("Connected")).toBeNull();
    expect(screen.queryByText("Offline")).toBeNull();
  });

  it("orders rows with errors, then running, then stopped, and counts what runs", () => {
    renderCard(entry({}, [
      stopped("b-stopped"),
      { projectId: "r", name: "a-running", path: "/srv/r", isRunning: true, memoryKb: 2 * 1024 * 1024 },
      { ...stopped("z-error"), lastError: "It broke" },
    ]));
    const names = Array.from(document.querySelectorAll("[id^=project-] > div:first-child > div:first-child")).map((node) => node.textContent);
    expect(names).toEqual(["z-error", "a-running", "b-stopped"]);
    expect(screen.getByText("1 of 3 projects running · 2.0 GiB")).toBeTruthy();
    expect(screen.getByText("Providers not reported")).toBeTruthy();
  });

  it("counts what this machine runs, whatever the projects' open states", () => {
    renderCard(entry({ daemonConnected: true }, [
      // Open, with its tabs on another machine: this machine does not run it.
      { ...stopped("elsewhere"), openState: "open", otherHosts: [{ hostname: "zoo-006", connected: true, running: true, ownsTabs: true }] },
      // Closed, and still running here.
      { projectId: "s", name: "still", path: "/srv/s", isRunning: true, openState: "closed", part: "still_running" },
      { projectId: "r", name: "running", path: "/srv/r", isRunning: true, openState: "open", part: "running" },
    ]), { openStateSupported: true });
    expect(screen.getByText("2 of 3 projects running")).toBeTruthy();
    const states = Array.from(document.querySelectorAll("[data-testid=project-open-state]")).map((node) => node.textContent);
    const parts = Array.from(document.querySelectorAll("[data-testid=machine-part]")).map((node) => node.textContent);
    // Running rows first, then the one this machine is not running.
    expect(states).toEqual(["Open", "Closed", "Open"]);
    expect(parts).toEqual(["Running", "Still running", "No tabs here"]);
  });

  it("keeps a row the server says did not start out of the fold, with its reason", () => {
    renderCard(entry({ daemonConnected: true }, [
      stopped("a"), stopped("b"), stopped("c"), stopped("d"),
      { ...stopped("refused"), openState: "open", part: "not_started", partReason: "zoo-005 is not allowed to run it." },
    ]), { openStateSupported: true });
    expect(screen.getByText("Not started: zoo-005 is not allowed to run it.")).toBeTruthy();
    expect(screen.queryByText("a")).toBeNull();
    expect(screen.getByRole("button", { name: "Show 4 projects not running here" })).toBeTruthy();
  });

  it("says when a machine has no projects", () => {
    renderCard(entry());
    expect(screen.getByText("No projects on this machine.")).toBeTruthy();
  });

  it("folds the projects this machine is not running only when there are more than three", () => {
    renderCard(entry({}, [stopped("a"), stopped("b"), stopped("c")]));
    expect(screen.queryByRole("button", { name: /projects not running here/ })).toBeNull();
    expect(screen.getByText("a")).toBeTruthy();
    cleanup();

    const { onToggleStopped } = renderCard(entry({}, [stopped("a"), stopped("b"), stopped("c"), stopped("d")]));
    expect(screen.queryByText("a")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show 4 projects not running here" }));
    expect(onToggleStopped).toHaveBeenCalledTimes(1);
    cleanup();

    renderCard(entry({}, [stopped("a"), stopped("b"), stopped("c"), stopped("d")]), { stoppedOpen: true });
    expect(screen.getByText("d")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Hide projects not running here" })).toBeTruthy();
  });
});
