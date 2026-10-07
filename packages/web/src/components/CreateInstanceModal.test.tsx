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

function localMachine(): MachineWithProjects {
  return { ...machine("m1", "alpha"), clusterOwnerUserId: "owner", clusterAccess: "owner", localProjectRegistrationAvailable: true };
}

function localReady(machines = [localMachine()]) {
  const send = vi.fn();
  act(() => {
    useStore.setState({
      machines,
      pendingInstances: {},
      ws: { readyState: 1, send } as unknown as WebSocket,
      registerLocalProject: initialStore.registerLocalProject,
      userId: "owner",
    });
  });
  return send;
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

    // The raw URL is the only project metadata the user can edit.
    expect(screen.getByDisplayValue("git@github.com:shuaimu/apas.git")).toBeTruthy();
    expect(screen.queryByLabelText("Instance name")).toBeNull();
    expect(screen.queryByLabelText("New branch")).toBeNull();
    expect(screen.queryByLabelText("Projects root (optional)")).toBeNull();
    expect(screen.getByText("~/apas_projects/apas")).toBeTruthy();
    expect(screen.getByText("apas/apas")).toBeTruthy();

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

  it.each([
    ["https://github.com/openai/codex.git", "https://github.com/openai/codex.git"],
    ["github.com/openai/codex", "https://github.com/openai/codex"],
    ["  github.com/openai/codex.git  ", "https://github.com/openai/codex.git"],
    ["GITHUB.COM/openai/codex", "https://github.com/openai/codex"],
  ])("creates a brand-new project from %s", (enteredUrl, cloneUrl) => {
    const create = seed([machine("m1", "alpha")]);

    render(<CreateInstanceModal open onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Clone URL"), {
      target: { value: enteredUrl },
    });

    expect(screen.getByText("~/apas_projects/codex")).toBeTruthy();
    expect(screen.getByText("apas/codex")).toBeTruthy();
    fireEvent.click(screen.getByText("Create & start"));
    expect(create).toHaveBeenCalledWith(
      "m1",
      "github.com/openai/codex",
      "codex",
      "apas/codex",
      cloneUrl,
      undefined,
    );
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

describe("existing-folder creation", () => {
  afterEach(() => act(() => { useStore.setState(initialStore, true); }));

  it("switches validation and payloads without losing either draft, then waits for confirmation", () => {
    const send = localReady();
    const onClose = vi.fn();
    render(<CreateInstanceModal open onClose={onClose} />);
    fireEvent.change(screen.getByLabelText("Clone URL"), { target: { value: "not a github URL" } });
    fireEvent.click(screen.getByRole("button", { name: "Existing folder" }));
    expect(screen.getByRole("button", { name: "Existing folder" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.queryByLabelText("Clone URL")).toBeNull();
    const path = screen.getByLabelText("Folder path on alpha");
    fireEvent.change(path, { target: { value: "relative/path" } });
    expect((screen.getByRole("button", { name: "Register folder" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(path, { target: { value: "~/work/My Project" } });
    fireEvent.click(screen.getByRole("button", { name: "Clone from GitHub" }));
    expect(screen.getByDisplayValue("not a github URL")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Existing folder" }));
    expect(screen.getByDisplayValue("~/work/My Project")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Register folder" }));
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("status").textContent).toContain("Waiting for confirmed registration");
    fireEvent.click(screen.getByRole("button", { name: "Registering…" }));
    expect(send).toHaveBeenCalledTimes(1);
    const message = JSON.parse(send.mock.calls[0][0]);
    expect(message).toEqual({
      type: "register_local_project", machine_id: "m1", cluster_owner_user_id: "owner",
      path: "~/work/My Project", request_id: expect.any(String),
    });
  });

  it("retains the local draft after a send failure and a daemon failure, allowing correction", () => {
    const send = localReady();
    send.mockImplementationOnce(() => { throw new Error("Socket disconnected"); });
    render(<CreateInstanceModal open onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Existing folder" }));
    fireEvent.change(screen.getByLabelText("Folder path on alpha"), { target: { value: "/work/project" } });
    fireEvent.click(screen.getByRole("button", { name: "Register folder" }));
    expect(screen.getByRole("alert").textContent).toContain("Socket disconnected");
    expect(screen.getByDisplayValue("/work/project")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Register folder" }));
    const operation = Object.values(useStore.getState().pendingInstances)[0];
    act(() => useStore.setState({ pendingInstances: {
      [operation.requestId]: { ...operation, source: "local", path: "/work/project", status: "failed", error: "Folder does not exist" },
    } }));
    expect(screen.getByRole("alert").textContent).toContain("Folder does not exist");
    fireEvent.change(screen.getByLabelText("Folder path on alpha"), { target: { value: "/work/corrected" } });
    expect((screen.getByRole("button", { name: "Register folder" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Register folder" }));
    expect(JSON.parse(send.mock.calls[2][0]).path).toBe("/work/corrected");
  });

  it("shows only eligible owned machines and preserves an explicitly shared cluster context", () => {
    const owned = localMachine();
    const shared = { ...machine("shared", "shared-host"), clusterOwnerUserId: "other", clusterAccess: "member" as const, localProjectRegistrationAvailable: true, sharedProvisioningAvailable: true };
    localReady([owned, shared, machine("older", "older-host")]);
    const view = render(<CreateInstanceModal open onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Existing folder" }));
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(screen.getByRole("option").textContent).toContain("alpha");
    view.unmount();
    render(<CreateInstanceModal open onClose={vi.fn()} clusterOwnerUserId="other" />);
    fireEvent.click(screen.getByRole("button", { name: "Existing folder" }));
    expect(screen.queryByLabelText("Machine")).toBeNull();
    expect(screen.getByText(/Shared-cluster members cannot register host folders/)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Register folder" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Clone from GitHub" }));
    expect(screen.getByRole("option").textContent).toContain("shared-host");
    expect(screen.getByText(/Shared machines accept only public/)).toBeTruthy();
  });

  it("keeps repository-group creation clone-only", () => {
    localReady();
    render(<CreateInstanceModal open onClose={vi.fn()} gitRemote="github.com/a/b" />);
    expect(screen.queryByRole("group", { name: "Project source" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Existing folder" })).toBeNull();
  });

  it("shows the confirmed canonical folder and links to its owner inventory without starting", () => {
    const send = localReady();
    localStorage.setItem("apas_mobile_cluster_owner", "shared-owner");
    render(<CreateInstanceModal open onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Existing folder" }));
    fireEvent.change(screen.getByLabelText("Folder path on alpha"), { target: { value: "~/alias" } });
    fireEvent.click(screen.getByRole("button", { name: "Register folder" }));
    const operation = Object.values(useStore.getState().pendingInstances)[0];
    act(() => useStore.setState({ pendingInstances: {
      [operation.requestId]: { ...operation, source: "local", path: "~/alias", status: "registered", clusterOwnerUserId: "owner", project: { projectId: "project-1", path: "/canonical/project", isRunning: false } },
    } }));
    expect(screen.getByText("/canonical/project")).toBeTruthy();
    expect(screen.getByText(/The project is stopped/)).toBeTruthy();
    expect(screen.getByRole("link", { name: "View on Machines" }).getAttribute("href")).toBe("/machines?cluster_owner=owner&machine=m1&project=project-1#project-m1-project-1");
    expect(send).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: "Register folder" })).toBeNull();
    localStorage.removeItem("apas_mobile_cluster_owner");
  });
});
