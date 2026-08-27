import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore, type MachineWithProjects } from "@/lib/store";
import { CreateInstanceModal } from "./CreateInstanceModal";

const initialStore = useStore.getState();

function machine(machineId: string, hostname: string): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x64" },
    projects: [],
  };
}

function seed(
  machines: MachineWithProjects[],
  createProjectInstance = vi.fn().mockReturnValue(true),
): ReturnType<typeof vi.fn> {
  act(() => {
    useStore.setState({ machines, createProjectInstance });
  });
  return createProjectInstance;
}

describe("CreateInstanceModal", () => {
  afterEach(() => {
    vi.clearAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("prefills the captured clone URL and submits with the chosen machine", () => {
    const create = seed([machine("m1", "alpha"), machine("m2", "beta")]);

    render(
      <CreateInstanceModal
        open
        onClose={vi.fn()}
        gitRemote="github.com/shuaimu/apas"
        cloneUrl="git@github.com:shuaimu/apas.git"
      />,
    );

    // Raw URL prefilled; instance name + branch defaulted from the repo.
    expect(screen.getByDisplayValue("git@github.com:shuaimu/apas.git")).toBeTruthy();
    expect(screen.getByDisplayValue("apas")).toBeTruthy();
    expect(screen.getByDisplayValue("apas/apas")).toBeTruthy();

    fireEvent.click(screen.getByText("Create & start"));

    expect(create).toHaveBeenCalledWith(
      "m1", // first machine is preselected
      "github.com/shuaimu/apas",
      "apas",
      "apas/apas",
      "git@github.com:shuaimu/apas.git",
      undefined,
    );
  });

  it("reconstructs an https clone URL when none was captured", () => {
    seed([machine("m1", "alpha")]);

    render(<CreateInstanceModal open onClose={vi.fn()} gitRemote="github.com/foo/bar" />);

    expect(screen.getByDisplayValue("https://github.com/foo/bar.git")).toBeTruthy();
  });

  it("shows an empty state and disables create when no daemons are running", () => {
    seed([]);

    render(<CreateInstanceModal open onClose={vi.fn()} gitRemote="github.com/foo/bar" />);

    expect(screen.getByText(/No machines are running the apas daemon/i)).toBeTruthy();
    expect((screen.getByText("Create & start") as HTMLButtonElement).disabled).toBe(true);
  });

  it("uses the credential-isolated shared-cluster request shape", () => {
    const shared = machine("shared-1", "owner-host");
    shared.clusterOwnerUserId = "cluster-owner";
    shared.clusterAccess = "member";
    shared.sharedProvisioningAvailable = true;
    const create = seed([shared]);

    render(
      <CreateInstanceModal
        open
        onClose={vi.fn()}
        gitRemote="github.com/openai/codex"
        cloneUrl="https://github.com/openai/codex"
        clusterOwnerUserId="cluster-owner"
      />,
    );

    expect(screen.getByText(/Shared machines accept only public/)).toBeTruthy();
    expect(screen.queryByText("Projects root (optional)")).toBeNull();
    fireEvent.click(screen.getByText("Create & start"));
    expect(create).toHaveBeenCalledWith(
      "shared-1",
      "github.com/openai/codex",
      "codex",
      "apas/codex",
      "https://github.com/openai/codex",
      undefined,
      "cluster-owner",
    );
  });
});
