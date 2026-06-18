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
      minimaxBackend: {
        apiBaseUrl: "https://api.minimax.io/anthropic",
        apiKey: "sk-minimax",
        apiKeyConfigured: true,
      },
      glmBackend: {
        apiBaseUrl: "https://api.z.ai/api/anthropic",
        apiKeyConfigured: false,
      },
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
    setMachineMiniMaxConfig: vi.fn(),
    setMachineGlmConfig: vi.fn(),
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
  it("renders machine identity, provider backend states, projects, and the empty state", () => {
    const actions = seedMachines();

    const { unmount } = render(<MachinesPage />);

    expect(actions.listMachines).toHaveBeenCalledTimes(1);
    expect(screen.getByText("build-host")).toBeTruthy();
    expect(screen.getByText(/linux\/x64/)).toBeTruthy();
    expect(screen.getByText("MiniMax Backend (Claude Runtime)")).toBeTruthy();
    expect(screen.getByText("GLM Backend (Claude Runtime)")).toBeTruthy();
    expect(screen.getByText("DeepSeek Backend (Claude Runtime)")).toBeTruthy();
    expect(screen.getAllByText("API key configured")).toHaveLength(2);
    expect(screen.getByText("API key not configured")).toBeTruthy();
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

  it("wires MiniMax, GLM, and DeepSeek Save and Clear controls to store actions", () => {
    const actions = seedMachines();

    render(<MachinesPage />);

    fireEvent.change(screen.getByLabelText("MiniMax API key for build-host"), {
      target: { value: "  sk-minimax-new  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save MiniMax API key for build-host" }));
    expect(actions.setMachineMiniMaxConfig).toHaveBeenCalledWith(
      "machine-1",
      "sk-minimax-new",
      false,
    );
    fireEvent.click(screen.getByRole("button", { name: "Clear MiniMax API key for build-host" }));
    expect(actions.setMachineMiniMaxConfig).toHaveBeenLastCalledWith("machine-1", undefined, true);

    fireEvent.change(screen.getByLabelText("GLM API key for build-host"), {
      target: { value: "  sk-glm-new  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save GLM API key for build-host" }));
    expect(actions.setMachineGlmConfig).toHaveBeenCalledWith(
      "machine-1",
      "sk-glm-new",
      false,
    );
    fireEvent.click(screen.getByRole("button", { name: "Clear GLM API key for build-host" }));
    expect(actions.setMachineGlmConfig).toHaveBeenLastCalledWith("machine-1", undefined, true);

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
