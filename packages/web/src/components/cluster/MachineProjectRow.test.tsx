import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MachineProject, MachineWithProjects } from "@/lib/store";
import { CLOSE_DELETES_NOTHING, NOTHING_SENT_TEXT } from "@/lib/projectOpenState";
import { useConfirmDialog } from "./ConfirmDialog";
import { MachineProjectRow } from "./MachineProjectRow";

/**
 * One project in one machine's list, per scenario of
 * `machine-lifecycle-control` in the change `make-open-a-project-state`.
 * Everything the row shows comes from the machine list's own entries.
 */

const PROJECT = "30000000-0000-4000-8000-000000000001";
const ZOO5 = "50000000-0000-4000-8000-000000000001";
const ZOO6 = "50000000-0000-4000-8000-000000000002";
const LAPTOP = "50000000-0000-4000-8000-000000000007";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function machine(
  machineId: string,
  hostname: string,
  project: Partial<MachineProject> = {},
  overrides: Partial<MachineWithProjects> = {},
): MachineWithProjects {
  return {
    machine: { machineId, hostname, os: "linux", arch: "x64" },
    projects: [{ projectId: PROJECT, name: "mako", path: "/work/mako", isRunning: false, ...project }],
    clusterAccess: "owner",
    daemonConnected: true,
    projectInstancesAvailable: true,
    ...overrides,
  };
}

interface RowOptions {
  machines: MachineWithProjects[];
  /** The machine whose entry the row is. */
  from: string;
  supported?: boolean;
  connected?: boolean;
  onOpen?: () => boolean;
  onClose?: () => boolean;
}

function Row({ machines, from, supported = true, connected = true, onOpen, onClose }: RowOptions) {
  const { requestConfirm, confirmDialog } = useConfirmDialog();
  const entry = machines.find((candidate) => candidate.machine.machineId === from)!;
  return (
    <>
      <MachineProjectRow
        machineId={from}
        hostname={entry.machine.hostname}
        project={entry.projects[0]}
        highlighted={false}
        connected={connected}
        openStateSupported={supported}
        projectMachines={machines}
        requestConfirm={requestConfirm}
        onOpen={onOpen ?? (() => true)}
        onClose={onClose ?? (() => true)}
      />
      {confirmDialog}
    </>
  );
}

function renderRow(options: RowOptions) {
  const onOpen = vi.fn(options.onOpen ?? (() => true));
  const onClose = vi.fn(options.onClose ?? (() => true));
  const view = render(<Row {...options} onOpen={onOpen} onClose={onClose} />);
  return {
    onOpen,
    onClose,
    update: (machines: MachineWithProjects[]) =>
      view.rerender(<Row {...options} machines={machines} onOpen={onOpen} onClose={onClose} />),
  };
}

const state = () => screen.getByTestId("project-open-state").textContent;
const part = () => screen.queryByTestId("machine-part")?.textContent ?? null;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const dialogLines = (name: string) =>
  Array.from(screen.getByRole("dialog", { name }).querySelectorAll("p")).map((line) => line.textContent);
const confirm = (name: string) => fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name }));

const other = (hostname: string, running: boolean, connected = true) =>
  ({ hostname, connected, running, ownsTabs: true });

describe("MachineProjectRow", () => {
  it("opens a closed project from one machine's entry, naming the other machine first", () => {
    const closed = [
      machine(ZOO5, "zoo-005", { openState: "closed", part: "not_started", otherHosts: [other("zoo-006", false)] }),
      machine(ZOO6, "zoo-006", { openState: "closed", part: "not_started", otherHosts: [other("zoo-005", false)] }),
    ];
    const { onOpen, onClose, update } = renderRow({ machines: closed, from: ZOO5 });

    expect(state()).toBe("Closed");
    // "Closed" already says this machine is not running it, and the other
    // machine is in line with it.
    expect(part()).toBeNull();
    expect(screen.queryByTestId("other-machines")).toBeNull();
    expect(screen.queryByRole("button", { name: /^Close/ })).toBeNull();
    // Nothing starts or stops the project on this machine alone.
    expect(screen.queryByRole("button", { name: /^(Start|Stop)/ })).toBeNull();

    fireEvent.click(button("Open mako (listed on zoo-005)"));
    expect(onOpen).not.toHaveBeenCalled();
    expect(dialogLines("Open mako?")).toEqual(["Its tabs start on zoo-005 and zoo-006."]);
    confirm("Open");
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();

    // The request went out: the entry says so until the server's answer
    // changes it.
    const opening = button("Open mako (listed on zoo-005)");
    expect(opening.textContent).toContain("Opening…");
    expect(opening.disabled).toBe(true);

    update([
      machine(ZOO5, "zoo-005", { openState: "open", part: "starting", otherHosts: [other("zoo-006", false)] }),
      machine(ZOO6, "zoo-006", { openState: "open", part: "starting", otherHosts: [other("zoo-005", false)] }),
    ]);
    expect(state()).toBe("Open");
    expect(part()).toBe("Starting…");
    expect(screen.queryByText("Opening…")).toBeNull();
    expect(button("Close mako (listed on zoo-005)").disabled).toBe(false);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("closes a project from one machine's entry, naming both machines and that nothing is deleted", () => {
    const open = [
      machine(ZOO5, "zoo-005", { isRunning: true, openState: "open", part: "running", otherHosts: [other("zoo-006", true)] }),
      machine(ZOO6, "zoo-006", { isRunning: true, openState: "open", part: "running", memoryKb: 2 * 1024 * 1024, pid: 4242, otherHosts: [other("zoo-005", true)] }),
    ];
    const { onOpen, onClose } = renderRow({ machines: open, from: ZOO6 });

    expect(state()).toBe("Open");
    expect(part()).toBe("Running · 2.0 GiB");
    expect(screen.getByTestId("machine-part").getAttribute("title")).toBe("pid 4242");
    expect(screen.getByTestId("other-machines").textContent).toBe("zoo-005: Running");
    // It runs everywhere: there is nothing an Open would do.
    expect(screen.queryByRole("button", { name: /^(Open|Keep)/ })).toBeNull();

    fireEvent.click(button("Close mako (listed on zoo-006)"));
    expect(dialogLines("Close mako?")).toEqual([
      "This stops its agents on zoo-005 and zoo-006.",
      CLOSE_DELETES_NOTHING,
    ]);
    // Declining stops nothing.
    confirm("Cancel");
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(button("Close mako (listed on zoo-006)"));
    confirm("Close");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
    expect(button("Close mako (listed on zoo-006)").textContent).toContain("Closing…");
  });

  it("opens a registered project that never ran on the machine whose entry is used", () => {
    const { onOpen } = renderRow({ machines: [machine(ZOO5, "zoo-005")], from: ZOO5 });

    expect(state()).toBe("Closed");
    expect(screen.getByTestId("project-open-state").getAttribute("title"))
      .toBe("Nobody has opened or closed this project yet. It is shown from what runs.");
    expect(part()).toBe("No tabs here");
    fireEvent.click(button("Open mako (listed on zoo-005)"));
    expect(dialogLines("Open mako?")).toEqual(["It starts on zoo-005."]);
    confirm("Open");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("shows an open project with no tabs on this machine and offers nothing that starts it here", () => {
    renderRow({
      machines: [
        machine(ZOO5, "zoo-005", { isRunning: true, openState: "open", part: "running" }),
        machine(ZOO6, "zoo-006", { openState: "open", otherHosts: [other("zoo-005", true)] }),
      ],
      from: ZOO6,
    });

    expect(state()).toBe("Open");
    expect(part()).toBe("No tabs here");
    expect(screen.getByTestId("other-machines").textContent).toBe("zoo-005: Running");
    expect(screen.queryByRole("button", { name: /^(Open|Keep|Start)/ })).toBeNull();
    // Closing is for the whole project, so it is offered from here too.
    fireEvent.click(button("Close mako (listed on zoo-006)"));
    expect(dialogLines("Close mako?")).toEqual(["This stops its agents on zoo-005.", CLOSE_DELETES_NOTHING]);
  });

  it("offers to keep open and to close a project nobody has recorded that this machine runs", () => {
    const { onOpen } = renderRow({
      machines: [machine(ZOO5, "zoo-005", { isRunning: true, part: "running" })],
      from: ZOO5,
    });

    expect(state()).toBe("Open");
    expect(part()).toBe("Running");
    const keep = button("Keep mako open (listed on zoo-005)");
    expect(keep.textContent).toContain("Keep open");
    expect(keep.title).toBe("Its tabs start again on zoo-005 after those machines have been away.");
    expect(button("Close mako (listed on zoo-005)")).toBeTruthy();

    fireEvent.click(keep);
    expect(dialogLines("Keep mako open?")).toEqual([
      "This records the project as open. Nothing starts now.",
      "Its tabs start again on zoo-005 after those machines have been away.",
    ]);
    confirm("Keep open");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("says nothing was sent when the page has no connection, and sends nothing later", () => {
    vi.useFakeTimers();
    const closed = [machine(ZOO5, "zoo-005", { openState: "closed", part: "not_started" })];
    const { onOpen, update } = renderRow({ machines: closed, from: ZOO5, onOpen: () => false });

    fireEvent.click(button("Open mako (listed on zoo-005)"));
    confirm("Open");
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("alert").textContent).toBe(NOTHING_SENT_TEXT);
    const open = button("Open mako (listed on zoo-005)");
    expect(open.textContent).not.toContain("Opening…");
    expect(open.disabled).toBe(false);

    // Neither time nor a new machine list sends it for the user.
    act(() => { vi.advanceTimersByTime(120_000); });
    update([machine(ZOO5, "zoo-005", { openState: "closed", part: "not_started", lastError: "later" })]);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("disables Open and Close while the page is not connected to the server", () => {
    renderRow({
      machines: [machine(ZOO5, "zoo-005", { isRunning: true, part: "running" })],
      from: ZOO5,
      connected: false,
    });

    const keep = button("Keep mako open (listed on zoo-005)");
    expect(keep.disabled).toBe(true);
    expect(keep.title).toBe("Not connected to the server");
    const close = button("Close mako (listed on zoo-005)");
    expect(close.disabled).toBe(true);
    expect(close.title).toBe("Not connected to the server");
  });

  it("says an Open from a machine that owns none of the tabs starts the others and not this one", () => {
    const { onOpen } = renderRow({
      machines: [machine(LAPTOP, "laptop", { openState: "closed", otherHosts: [other("zoo-005", false)] })],
      from: LAPTOP,
    });

    expect(state()).toBe("Closed");
    expect(part()).toBe("No tabs here");
    fireEvent.click(button("Open mako (listed on laptop)"));
    expect(onOpen).not.toHaveBeenCalled();
    expect(dialogLines("Open mako?"))
      .toEqual(["It starts on zoo-005 and not on laptop, which has no tabs of it."]);
    confirm("Open");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("says of an offline tab owner that its tabs start when it reconnects", () => {
    renderRow({
      machines: [machine(ZOO5, "zoo-005", {
        openState: "closed",
        part: "not_started",
        otherHosts: [other("laptop", false, false)],
      })],
      from: ZOO5,
    });

    fireEvent.click(button("Open mako (listed on zoo-005)"));
    expect(dialogLines("Open mako?")).toEqual([
      "Its tabs start on zoo-005.",
      "laptop is offline. Its tabs start when laptop reconnects.",
    ]);
  });

  it("shows why this machine did not start an open project, and offers to open it again", () => {
    renderRow({
      machines: [machine(ZOO6, "zoo-006", {
        openState: "open",
        part: "not_started",
        partReason: "The project's policy does not allow it on zoo-006.",
        // The daemon's own error, when it is the same sentence, is not said twice.
        lastError: "The project's policy does not allow it on zoo-006.",
        otherHosts: [other("zoo-005", true), other("laptop", false, false)],
      })],
      from: ZOO6,
    });

    expect(state()).toBe("Open");
    expect(part()).toBe("Not started");
    expect(screen.getByTestId("part-reason").textContent)
      .toBe("Not started: The project's policy does not allow it on zoo-006.");
    expect(screen.getAllByText(/The project's policy does not allow it/)).toHaveLength(1);
    expect(screen.getByTestId("other-machines").textContent)
      .toBe("laptop: Waiting for laptop (offline) · zoo-005: Running");
    expect(button("Open mako again (listed on zoo-006)").textContent).toContain("Open again");
    expect(button("Close mako (listed on zoo-006)")).toBeTruthy();
  });

  it("keeps the daemon's error when it differs from the server's reason", () => {
    renderRow({
      machines: [machine(ZOO6, "zoo-006", { openState: "open", part: "not_started", lastError: "The checkout is missing." })],
      from: ZOO6,
    });

    expect(part()).toBe("Not started");
    expect(screen.queryByTestId("part-reason")).toBeNull();
    expect(screen.getByText("The checkout is missing.")).toBeTruthy();
  });

  it("offers Close again while this machine still runs a closed project", () => {
    const { onClose } = renderRow({
      machines: [machine(ZOO6, "zoo-006", {
        isRunning: true,
        openState: "closed",
        part: "still_running",
        partReason: "The account that closed it may not use zoo-006.",
      })],
      from: ZOO6,
    });

    expect(state()).toBe("Closed");
    expect(part()).toBe("Still running");
    expect(screen.getByTestId("part-reason").textContent).toBe("The account that closed it may not use zoo-006.");
    fireEvent.click(button("Close mako again (listed on zoo-006)"));
    expect(dialogLines("Close mako again?")).toEqual([
      "zoo-006 still runs this project.",
      "This stops its agents on zoo-006.",
      CLOSE_DELETES_NOTHING,
    ]);
    confirm("Close again");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("names another machine that still runs a closed project", () => {
    renderRow({
      machines: [machine(ZOO5, "zoo-005", {
        openState: "closed",
        part: "not_started",
        otherHosts: [other("zoo-006", true), other("laptop", false, false)],
      })],
      from: ZOO5,
    });

    expect(state()).toBe("Closed");
    expect(screen.getByTestId("other-machines").textContent).toBe("zoo-006: Still running");
    expect(button("Close mako again (listed on zoo-005)")).toBeTruthy();
    expect(button("Open mako (listed on zoo-005)")).toBeTruthy();
  });

  it("says this machine waits while it is offline", () => {
    renderRow({
      machines: [machine(ZOO6, "zoo-006", { openState: "open", part: "waiting_offline" }, { daemonConnected: false })],
      from: ZOO6,
    });

    expect(part()).toBe("Waiting for zoo-006 (offline)");
    // Opening again cannot start an offline machine.
    expect(screen.queryByRole("button", { name: /^Open/ })).toBeNull();
    expect(button("Close mako (listed on zoo-006)")).toBeTruthy();
  });

  it("gives the button back after 30 seconds without an answer, and sends nothing again", () => {
    vi.useFakeTimers();
    const { onOpen } = renderRow({
      machines: [machine(ZOO5, "zoo-005", { openState: "closed", part: "not_started" })],
      from: ZOO5,
    });

    fireEvent.click(button("Open mako (listed on zoo-005)"));
    confirm("Open");
    expect(button("Open mako (listed on zoo-005)").disabled).toBe(true);
    act(() => { vi.advanceTimersByTime(30_000); });
    const open = button("Open mako (listed on zoo-005)");
    expect(open.disabled).toBe(false);
    expect(open.textContent).not.toContain("Opening…");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  describe("on a server that records no open state", () => {
    it("shows the project from what runs and never offers Keep open", () => {
      const { onClose } = renderRow({
        machines: [machine(ZOO5, "zoo-005", { isRunning: true, memoryKb: 2048 })],
        from: ZOO5,
        supported: false,
      });

      expect(state()).toBe("Open");
      expect(screen.getByTestId("project-open-state").getAttribute("title")).toBeNull();
      expect(part()).toBe("Running · 2 MiB");
      expect(screen.queryByRole("button", { name: /^(Open|Keep)/ })).toBeNull();
      fireEvent.click(button("Close mako (listed on zoo-005)"));
      expect(dialogLines("Close mako?"))
        .toEqual(["This stops its agents on every machine that runs it.", CLOSE_DELETES_NOTHING]);
      confirm("Close");
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("offers Open for a project that runs nowhere, saying what the request does", () => {
      const { onOpen } = renderRow({ machines: [machine(ZOO5, "zoo-005")], from: ZOO5, supported: false });

      expect(state()).toBe("Closed");
      expect(part()).toBeNull();
      fireEvent.click(button("Open mako (listed on zoo-005)"));
      expect(dialogLines("Open mako?"))
        .toEqual(["It starts on each online machine that has its tabs, or on zoo-005 when none has."]);
      confirm("Open");
      expect(onOpen).toHaveBeenCalledTimes(1);
    });
  });
});
