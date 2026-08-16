/**
 * The same machine must read the same way wherever it is listed.
 *
 * Each surface renders its own markup, so nothing structural stops them from
 * drifting — one could keep saying "Reboot daemon" while the other learns to
 * say a machine is behind. This renders both against identical machine data and
 * compares what they actually say.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MachinesPage from "@/app/machines/page";
import { MobileCodeHome } from "@/components/mobile/MobileCodeHome";
import { useStore, type MachineWithProjects } from "@/lib/store";

vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));

const initialStore = useStore.getState();
const originalFetch = globalThis.fetch;

/** One host a version behind another, so both wordings appear at once. */
const FLEET = [
  { machineId: "machine-a", hostname: "zoo-005", daemonVersion: "26.08.74" },
  { machineId: "machine-b", hostname: "zoo-006", daemonVersion: "26.08.70" },
];

function rebootControlNames(): string[] {
  return screen
    .getAllByRole("button")
    .map((button) => button.getAttribute("aria-label") ?? "")
    .filter((name) => name.toLowerCase().includes("daemon"))
    .sort();
}

beforeEach(() => {
  globalThis.fetch = vi.fn().mockResolvedValue({
    ok: true,
    status: 200,
    json: async () => ({
      items: [],
      limit: 200,
      offset: 0,
      sessions: [],
      machines: FLEET.map((machine) => ({
        machine: {
          machine_id: machine.machineId,
          hostname: machine.hostname,
          daemon_version: machine.daemonVersion,
        },
        projects: [],
      })),
      cluster: null,
      deployment: null,
    }),
  }) as unknown as typeof fetch;
  window.localStorage.setItem("apas_token", "test-token");
});

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
  act(() => {
    useStore.setState(initialStore, true);
  });
});

describe("restart wording across surfaces", () => {
  it("describes the same machines the same way on desktop and mobile", async () => {
    act(() => {
      useStore.setState({
        token: "test-token",
        connected: true,
        serverVersion: null,
        machines: FLEET.map((machine) => ({
          machine: { ...machine, os: "linux", arch: "x64" },
          projects: [],
        })) as MachineWithProjects[],
        usageLimits: new Map(),
        connect: vi.fn(),
        listMachines: vi.fn(),
        rebootDaemon: vi.fn(),
      });
    });

    const desktop = render(<MachinesPage />);
    const desktopNames = rebootControlNames();
    desktop.unmount();

    render(
      <MobileCodeHome
        active
        connected
        legacySessions={[]}
        token="test-token"
        onAccount={vi.fn()}
        onManageMachines={vi.fn()}
        onOpenSession={vi.fn()}
        onRebootDaemon={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Machines" }));
    await screen.findByText("zoo-005");
    const mobileNames = rebootControlNames();

    expect(desktopNames).toEqual(mobileNames);
    // And they are saying the thing this change is for, not merely agreeing.
    expect(desktopNames).toEqual([
      "Reboot and update the daemon on zoo-006",
      "Reboot the daemon on zoo-005",
    ]);
  });
});
