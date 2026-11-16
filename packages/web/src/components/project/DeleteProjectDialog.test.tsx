import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { useStore } from "@/lib/store";
import { DeleteProjectDialog, type DeleteProjectDialogProps } from "./DeleteProjectDialog";

const PROJECT = { id: "30000000-0000-4000-8000-000000000001", name: "mako" };
const DELETE_URL = `https://apas.mpaxos.com/projects/${PROJECT.id}/delete`;
const STATUS_URL = `https://apas.mpaxos.com/projects/${PROJECT.id}/deletion`;

let forgetProject: Mock;
let listSessions: Mock;
let fetchMock: Mock;

function response(status: number, body: unknown = {}) {
  return { status, ok: status >= 200 && status < 300, json: async () => body };
}

function open(props: Partial<DeleteProjectDialogProps> = {}) {
  const onClose = vi.fn();
  const onDeleted = vi.fn();
  render(
    <DeleteProjectDialog
      project={PROJECT}
      onClose={onClose}
      onDeleted={onDeleted}
      deletionWait={{ intervalMs: 1, attempts: 3 }}
      {...props}
    />,
  );
  return { onClose, onDeleted };
}

function typeName(value: string) {
  fireEvent.change(screen.getByLabelText("Project name"), { target: { value } });
}

function deleteButton() {
  return screen.getByRole("button", { name: /^(Delete project|Deleting…)$/ }) as HTMLButtonElement;
}

function calls(url: string) {
  return fetchMock.mock.calls.filter(([called]) => called === url);
}

beforeEach(() => {
  forgetProject = vi.fn();
  listSessions = vi.fn();
  fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
  useStore.setState({
    token: "test-token",
    forgetProject: forgetProject as never,
    listSessions: listSessions as never,
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("DeleteProjectDialog", () => {
  it("says what is erased, what stays, and which machines are stopped first", () => {
    open({ stoppedFirstHosts: ["zoo-005", "zoo-006"] });
    const dialog = screen.getByRole("dialog", { name: "Delete project" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(screen.getByRole("heading", { name: "Delete mako?" })).toBeTruthy();
    expect(dialog.textContent).toContain(
      "This erases the project from the APAS server: its tabs, the conversations shown in APAS, its history, usage, sharing, invitations and policy. This cannot be undone.",
    );
    expect(dialog.textContent).toContain(
      "The folder and its files stay on every machine. Each of those machines keeps the folder registered, so it stays in the lists as a registered folder, and opening it there creates a new, empty project.",
    );
    expect(screen.getByText("Agents running in the project are stopped first, on zoo-005 and zoo-006.")).toBeTruthy();
    // No offline tab owner: nothing is said about one.
    expect(dialog.textContent).not.toContain("offline");
    expect(document.activeElement).toBe(screen.getByLabelText("Project name"));
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("names no machine when none runs the project", () => {
    open();
    expect(screen.getByText("Agents running in the project are stopped first.")).toBeTruthy();
  });

  it("says an offline tab owner cannot be stopped and that the project comes back from it", () => {
    open({ stoppedFirstHosts: ["zoo-005"], offlineTabOwnerHosts: ["laptop"] });
    expect(screen.getByText(
      "laptop is offline and owns tabs of this project, so it cannot be stopped now. If laptop reconnects while it still runs the project, the project comes back from it.",
    )).toBeTruthy();
    cleanup();
    open({ offlineTabOwnerHosts: ["laptop", "mini"] });
    expect(screen.getByText(
      "laptop and mini are offline and own tabs of this project, so they cannot be stopped now. If one of them reconnects while it still runs the project, the project comes back from it.",
    )).toBeTruthy();
  });

  it("keeps Delete project disabled until the typed text is the project's name", () => {
    open();
    expect(deleteButton().disabled).toBe(true);
    typeName("mak");
    expect(deleteButton().disabled).toBe(true);
    // The id is what the server checks, not what the user types.
    typeName(PROJECT.id);
    expect(deleteButton().disabled).toBe(true);
    typeName("Mako");
    expect(deleteButton().disabled).toBe(true);
    fireEvent.keyDown(screen.getByLabelText("Project name"), { key: "Enter" });
    fireEvent.click(deleteButton());
    expect(fetchMock).not.toHaveBeenCalled();
    typeName("mako");
    expect(deleteButton().disabled).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("sends one request with the project id, and says Deleting… until the server reports it done", async () => {
    let finish: (value: unknown) => void = () => {};
    fetchMock
      .mockResolvedValueOnce(response(202))
      .mockResolvedValueOnce(response(200, { status: "deleting" }))
      .mockResolvedValueOnce(response(409))
      .mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const { onClose, onDeleted } = open();
    typeName("mako");
    fireEvent.click(deleteButton());
    // A second press while the first request is out sends nothing.
    fireEvent.click(deleteButton());

    await waitFor(() => expect(calls(STATUS_URL)).toHaveLength(3));
    expect(calls(DELETE_URL)).toHaveLength(1);
    expect(fetchMock.mock.calls[0]).toEqual([DELETE_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: "Bearer test-token" },
      body: JSON.stringify({ confirmation: PROJECT.id }),
    }]);
    expect(fetchMock.mock.calls[1]).toEqual([STATUS_URL, { headers: { Authorization: "Bearer test-token" } }]);
    expect(deleteButton().textContent).toBe("Deleting…");
    expect(deleteButton().disabled).toBe(true);
    expect(screen.getByRole("status").textContent).toContain("Deleting…");
    expect((screen.getByLabelText("Project name") as HTMLInputElement).disabled).toBe(true);
    expect(forgetProject).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => { finish(response(404)); });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(forgetProject).toHaveBeenCalledWith(PROJECT.id);
    expect(listSessions).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith(PROJECT.id);
    expect(calls(DELETE_URL)).toHaveLength(1);
  });

  it("shows a refusal, claims no deletion, and sends nothing again by itself", async () => {
    fetchMock.mockResolvedValueOnce(response(403, { error: "Only the project owner can delete it" }));
    const { onClose } = open();
    typeName("mako");
    fireEvent.click(deleteButton());
    expect((await screen.findByRole("alert")).textContent).toBe("Only the project owner can delete it");
    await new Promise((done) => setTimeout(done, 20));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("status")).toBeNull();
    expect(forgetProject).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    // The user may ask again; that is their request, not a retry.
    expect(deleteButton().textContent).toBe("Delete project");
    expect(deleteButton().disabled).toBe(false);
  });

  it("shows a request that got no answer as an error and sends nothing again by itself", async () => {
    fetchMock.mockRejectedValueOnce(new Error("Failed to fetch"));
    open();
    typeName("mako");
    fireEvent.click(deleteButton());
    expect((await screen.findByRole("alert")).textContent).toBe("Failed to fetch");
    await new Promise((done) => setTimeout(done, 20));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(forgetProject).not.toHaveBeenCalled();
  });

  it("never repeats an accepted request when the wait for it fails", async () => {
    fetchMock
      .mockResolvedValueOnce(response(202))
      .mockResolvedValueOnce(response(500, { error: "Deletion status is unavailable" }));
    const { onClose } = open();
    typeName("mako");
    fireEvent.click(deleteButton());
    expect((await screen.findByRole("alert")).textContent).toBe("Deletion status is unavailable");
    expect(deleteButton().disabled).toBe(true);
    expect(deleteButton().textContent).toBe("Deleting…");
    fireEvent.click(deleteButton());
    fireEvent.keyDown(screen.getByLabelText("Project name"), { key: "Enter" });
    await new Promise((done) => setTimeout(done, 20));
    expect(calls(DELETE_URL)).toHaveLength(1);
    expect(forgetProject).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("says the deletion is still in progress when the server has not finished in time", async () => {
    fetchMock.mockResolvedValueOnce(response(202)).mockResolvedValue(response(200));
    open();
    typeName("mako");
    fireEvent.click(deleteButton());
    expect((await screen.findByRole("alert")).textContent)
      .toBe("Deletion is still in progress. You can safely close this dialog.");
    expect(calls(STATUS_URL)).toHaveLength(3);
    expect(calls(DELETE_URL)).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Close" })).toBeTruthy();
  });

  it("claims nothing was sent when there is no sign-in to send it with", () => {
    useStore.setState({ token: null });
    open();
    typeName("mako");
    fireEvent.click(deleteButton());
    expect(screen.getByRole("alert").textContent).toBe("You are not signed in. Nothing was sent.");
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("closes without sending on Cancel, Escape and a press on the backdrop", () => {
    const { onClose } = open();
    typeName("mako");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    fireEvent.mouseDown(screen.getByRole("dialog"));
    expect(onClose).toHaveBeenCalledTimes(2);
    fireEvent.mouseDown(screen.getByRole("dialog").parentElement!);
    expect(onClose).toHaveBeenCalledTimes(3);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("returns focus to what opened it", () => {
    function Harness() {
      return <button type="button">Delete project…</button>;
    }
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Delete project…" });
    opener.focus();
    const view = render(<DeleteProjectDialog project={PROJECT} onClose={() => {}} />);
    expect(document.activeElement).toBe(screen.getByLabelText("Project name"));
    view.unmount();
    expect(document.activeElement).toBe(opener);
  });

  it("returns focus where it is told to when what led to it is gone", () => {
    // Another dialog led here and closed itself in the same step: its
    // button is gone, and focus belongs to what opened that dialog.
    render(<button type="button">Manage project access</button>);
    const shareButton = screen.getByRole("button", { name: "Manage project access" });
    expect(document.activeElement).toBe(document.body);
    const view = render(<DeleteProjectDialog project={PROJECT} onClose={() => {}} returnFocusTo={shareButton} />);
    expect(document.activeElement).toBe(screen.getByLabelText("Project name"));
    view.unmount();
    expect(document.activeElement).toBe(shareButton);
  });

  it("still drops the project from the lists, and tells its caller, when the dialog was closed while deleting", async () => {
    let finish: (value: unknown) => void = () => {};
    fetchMock
      .mockResolvedValueOnce(response(202))
      .mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const { onClose, onDeleted } = open();
    typeName("mako");
    fireEvent.click(deleteButton());
    await waitFor(() => expect(calls(STATUS_URL)).toHaveLength(1));
    cleanup();
    expect(onDeleted).not.toHaveBeenCalled();
    await act(async () => { finish(response(404)); });
    await waitFor(() => expect(forgetProject).toHaveBeenCalledWith(PROJECT.id));
    expect(listSessions).toHaveBeenCalledTimes(1);
    // A caller with a list of its own must drop the project from it too.
    // Only closing the dialog is skipped: it is closed already.
    expect(onDeleted).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith(PROJECT.id);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("calls the handler of its last render when the deletion ends after it was closed", async () => {
    let finish: (value: unknown) => void = () => {};
    fetchMock
      .mockResolvedValueOnce(response(202))
      .mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const first = vi.fn();
    const last = vi.fn();
    const view = render(
      <DeleteProjectDialog project={PROJECT} onClose={() => {}} onDeleted={first} deletionWait={{ intervalMs: 1, attempts: 3 }} />,
    );
    typeName("mako");
    fireEvent.click(deleteButton());
    await waitFor(() => expect(calls(STATUS_URL)).toHaveLength(1));
    view.rerender(
      <DeleteProjectDialog project={PROJECT} onClose={() => {}} onDeleted={last} deletionWait={{ intervalMs: 1, attempts: 3 }} />,
    );
    view.unmount();
    await act(async () => { finish(response(404)); });
    await waitFor(() => expect(last).toHaveBeenCalledWith(PROJECT.id));
    expect(first).not.toHaveBeenCalled();
  });

  it("tells its caller when it closes after a wait that ended without an answer", async () => {
    fetchMock.mockResolvedValueOnce(response(202)).mockResolvedValue(response(200));
    const onUnconfirmedClose = vi.fn();
    const { onDeleted } = open({ onUnconfirmedClose });
    typeName("mako");
    fireEvent.click(deleteButton());
    expect((await screen.findByRole("alert")).textContent)
      .toBe("Deletion is still in progress. You can safely close this dialog.");
    // Nothing yet: the dialog is still open and says how things stand.
    expect(onUnconfirmedClose).not.toHaveBeenCalled();
    cleanup();
    expect(onUnconfirmedClose).toHaveBeenCalledTimes(1);
    expect(onUnconfirmedClose).toHaveBeenCalledWith(PROJECT.id);
    expect(onDeleted).not.toHaveBeenCalled();
    expect(forgetProject).not.toHaveBeenCalled();
    expect(calls(DELETE_URL)).toHaveLength(1);
  });

  it("tells its caller at once when the wait ends without an answer after it was closed", async () => {
    let finish: (value: unknown) => void = () => {};
    fetchMock
      .mockResolvedValueOnce(response(202))
      .mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const onUnconfirmedClose = vi.fn();
    const { onDeleted } = open({ onUnconfirmedClose });
    typeName("mako");
    fireEvent.click(deleteButton());
    await waitFor(() => expect(calls(STATUS_URL)).toHaveLength(1));
    cleanup();
    expect(onUnconfirmedClose).not.toHaveBeenCalled();
    await act(async () => { finish(response(500, { error: "Deletion status is unavailable" })); });
    await waitFor(() => expect(onUnconfirmedClose).toHaveBeenCalledTimes(1));
    expect(onDeleted).not.toHaveBeenCalled();
    expect(forgetProject).not.toHaveBeenCalled();
  });

  it("tells its caller nothing when it closes without a deletion, or after one that finished", async () => {
    const onUnconfirmedClose = vi.fn();
    open({ onUnconfirmedClose });
    cleanup();
    fetchMock.mockResolvedValueOnce(response(202)).mockResolvedValueOnce(response(404));
    const { onClose } = open({ onUnconfirmedClose });
    typeName("mako");
    fireEvent.click(deleteButton());
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    cleanup();
    expect(onUnconfirmedClose).not.toHaveBeenCalled();
  });

  it("asks for the id of a project that has no name", () => {
    open({ project: { id: PROJECT.id, name: " " } });
    typeName("");
    expect(deleteButton().disabled).toBe(true);
    typeName(PROJECT.id);
    expect(deleteButton().disabled).toBe(false);
  });
});
