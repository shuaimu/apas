import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import MachinesPage from "./page";
import { useStore, type MachineWithProjects } from "@/lib/store";

const routerPush = vi.hoisted(() => vi.fn());

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: routerPush }),
}));

const initialStore = useStore.getState();

function machineEntry(): MachineWithProjects {
  return {
    machine: {
      machineId: "machine-1",
      hostname: "build-host",
      os: "linux",
      arch: "x64",
      lastSeen: "2026-06-17T05:00:00Z",
      deepseekBackend: {
        apiBaseUrl: "https://api.deepseek.com/anthropic",
        apiKey: "sk-deepseek",
        apiKeyConfigured: true,
      },
    },
    projects: [
      {
        projectId: "project-running",
        name: "Active API",
        path: "/srv/active-api",
        isRunning: true,
        pid: 4242,
        memoryKb: 2 * 1024 * 1024,
      },
      {
        projectId: "project-stopped",
        name: "Stopped API",
        path: "/srv/stopped-api",
        isRunning: false,
      },
    ],
  };
}

function seedMachines(machines: MachineWithProjects[] = [machineEntry()]) {
  const actions = {
    connect: vi.fn(),
    listMachines: vi.fn(),
    startMachineProjectCli: vi.fn(),
    stopMachineProjectCli: vi.fn(),
    setMachineDeepseekConfig: vi.fn(),
  };

  window.localStorage.setItem("apas_token", "test-token");
  act(() => {
    useStore.setState({
      token: "test-token",
      connected: true,
      machines,
      usageLimits: new Map(),
      ...actions,
    });
  });

  return actions;
}

afterEach(() => {
  routerPush.mockReset();
  window.localStorage.clear();
  act(() => {
    useStore.setState(initialStore, true);
  });
});

describe("MachinesPage", () => {
  it("renders machine identity, supported backend state, projects, and the empty state", () => {
    const actions = seedMachines();

    const { unmount } = render(<MachinesPage />);

    expect(actions.listMachines).toHaveBeenCalledTimes(1);
    expect(screen.getByText("build-host")).toBeTruthy();
    expect(screen.getByText(/linux\/x64/)).toBeTruthy();
    expect(screen.getByText("DeepSeek Backend (Claude Runtime)")).toBeTruthy();
    expect(screen.getByText("API key configured")).toBeTruthy();
    expect(screen.queryByText(/MiniMax/i)).toBeNull();
    expect(screen.queryByText(/GLM/i)).toBeNull();
    expect(screen.getByText("Active API")).toBeTruthy();
    expect(screen.getByText("Stopped API")).toBeTruthy();
    expect(screen.getByText(/Running.*pid 4242/)).toBeTruthy();
    expect(screen.getByText("Stopped")).toBeTruthy();
    expect(screen.queryByText(/No machines reported yet/)).toBeNull();

    unmount();
    seedMachines([]);
    render(<MachinesPage />);

    expect(screen.getByText(/No machines reported yet/)).toBeTruthy();
  });

  it("wires DeepSeek Save and Clear controls to the retained store action", () => {
    const actions = seedMachines();

    render(<MachinesPage />);

    fireEvent.change(screen.getByLabelText("DeepSeek API key for build-host"), {
      target: { value: "  sk-deepseek-new  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save DeepSeek API key for build-host" }));
    expect(actions.setMachineDeepseekConfig).toHaveBeenCalledWith(
      "machine-1",
      "sk-deepseek-new",
      false,
    );
    fireEvent.click(screen.getByRole("button", { name: "Clear DeepSeek API key for build-host" }));
    expect(actions.setMachineDeepseekConfig).toHaveBeenLastCalledWith("machine-1", undefined, true);
  });

  it("wires Refresh and project Start/Stop controls to store actions", () => {
    const actions = seedMachines();

    render(<MachinesPage />);
    actions.listMachines.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "Refresh machines" }));
    expect(actions.listMachines).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Start Stopped API on build-host" }));
    expect(actions.startMachineProjectCli).toHaveBeenCalledWith("machine-1", "project-stopped");

    fireEvent.click(screen.getByRole("button", { name: "Stop Active API on build-host" }));
    expect(actions.stopMachineProjectCli).toHaveBeenCalledWith("machine-1", "project-running");
  });
});
