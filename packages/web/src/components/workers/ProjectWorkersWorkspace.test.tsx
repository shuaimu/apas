import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { WorkerConversionSource, WorkerSummary } from "@apas/protocol";
import { useStore } from "@/lib/store";
import * as cache from "@/lib/workerCache";
import { writeJobRecord } from "@/lib/jobCache";
import { ProjectWorkersWorkspace } from "./ProjectWorkersWorkspace";

const inventory = vi.hoisted(() => ({ workers: [] as WorkerSummary[], ready: true, loading: false, error: "", hasMore: true, refresh: vi.fn(), loadMore: vi.fn() }));
const useInventory = vi.hoisted(() => vi.fn());
const earlier = vi.hoisted(() => ({ jobs: [] as { id: string; title: string; state: string }[], ready: true, loading: false, error: "", hasMore: false, refresh: vi.fn(), loadMore: vi.fn() }));
const useEarlier = vi.hoisted(() => vi.fn());
vi.mock("@/lib/useProjectWorkers", () => ({ useProjectWorkers: (project: string | null) => { useInventory(project); return inventory; } }));
vi.mock("@/lib/useProjectJobs", () => ({ useProjectJobs: (project: string | null | undefined) => { useEarlier(project); return earlier; } }));
vi.mock("./WorkerComposer", () => ({ WorkerComposer: ({ source, onApproved }: { source?: WorkerConversionSource; onApproved: (id: string, project: string) => void }) => <><output data-testid="conversion-source">{JSON.stringify(source)}</output><button onClick={() => onApproved("worker", "project")}>Acknowledge approval receipt</button></> }));
vi.mock("./WorkerDetail", () => ({ WorkerDetailScreen: ({ id, runId, questionId, review, onNavigate }: { id: string; runId?: string; questionId?: string; review?: boolean; onNavigate: (destination: { runId: string; questionId?: string; review?: boolean }) => void }) => <div>
  <output data-testid="worker-destination">{JSON.stringify({ id, runId, questionId, review })}</output>
  <button onClick={() => onNavigate({ runId: "earlier-run", review: true })}>Earlier worker review</button>
  <button onClick={() => onNavigate({ runId: "question-run", questionId: "question" })}>Historical question</button>
</div> }));
vi.mock("../jobs/JobDetail", () => ({ JobDetailScreen: ({ id, runId, questionId, review }: { id: string; runId?: string; questionId?: string; review?: boolean }) => <output data-testid="earlier-destination">{JSON.stringify({ id, runId, questionId, review })}</output> }));

const initialStore = useStore.getInitialState();
const worker: WorkerSummary = {
  id: "worker", project_id: "project", title: "Repository caretaker", description: "Inspect the full repository responsibility without shortening the approved description.",
  state: "idle", reason: null, desired_state: "active", mandate_revision: 1, backlog_revision: 1, state_version: 3, high_watermark: 3,
  next_check_at: "2026-11-12T12:00:00Z", active_job_id: null, active_run_id: null, current_item_id: null,
  machine_id: "host", cluster_owner_user_id: "owner", hostname: "approved-host", attention: false,
  created_at: "2026-11-12T10:00:00Z", updated_at: "2026-11-12T11:00:00Z",
};
const project = { id: "project", name: "Project" };
const scope = ["account", "project", ""] as const;
function Workspace({ initialOverview = false, summary }: { initialOverview?: boolean; summary?: string }) {
  return <ProjectWorkersWorkspace project={project} paneSelectionRequested={false} initialOverview={initialOverview} overviewSummary={summary}
    renderOverviewPanels={(selectInteractive) => <section aria-label="Interactive project"><button onClick={selectInteractive}>Open a tab</button></section>}>
    {({ active, tabs, overviewTab, content, selectInteractive }) => <>
      {overviewTab}{tabs}<button onClick={selectInteractive}>Interactive pane</button>
      <div hidden={active}><input aria-label="Terminal draft" defaultValue="retained input" /></div>{content}
    </>}
  </ProjectWorkersWorkspace>;
}

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); Element.prototype.scrollIntoView = vi.fn(); });
beforeEach(async () => {
  await cache.purgeWorkerCache();
  useStore.setState({ userId: "account", sessionId: "selected-project" });
  inventory.workers = [worker]; inventory.ready = true; inventory.hasMore = true; inventory.loading = false; inventory.error = "";
  earlier.jobs = []; earlier.ready = true; earlier.hasMore = false;
  vi.clearAllMocks();
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); useStore.setState(initialStore, true); });
afterAll(() => vi.unstubAllGlobals());

it("shows project-filtered idle and hidden workers, full descriptions, next check and explicit pagination without manufacturing panes", async () => {
  await cache.writeWorkerRecord(scope, "draft", { hidden: [worker.id], opened: [], results: [], hiddenResults: [], view: { kind: "overview" } }, "project-workspace");
  render(<Workspace />);
  expect(await screen.findByText(worker.description)).toBeTruthy();
  expect(useInventory).toHaveBeenCalledWith("project");
  expect(screen.getByText(/View hidden/)).toBeTruthy();
  expect(screen.getByText("approved-host")).toBeTruthy();
  expect(screen.getByText("Next check")).toBeTruthy();
  expect(screen.getByText(/loaded subset/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: `Autonomous worker: ${worker.title}` })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Show full description" }));
  expect(screen.getByRole("button", { name: "Collapse description" }).getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: "Load more project workers" }));
  expect(inventory.loadMore).toHaveBeenCalledOnce();
  expect(useStore.getState().paneConfigs).toEqual(initialStore.paneConfigs);
  expect(useStore.getState().sessionId).toBe("selected-project");
});

it("preserves mounted terminal input and exact worker run/question navigation, while hiding only a view", async () => {
  render(<Workspace />);
  const terminal = screen.getByRole("textbox", { name: "Terminal draft" });
  fireEvent.change(terminal, { target: { value: "unsent terminal text" } });
  fireEvent.click(await screen.findByRole("button", { name: `Autonomous worker: ${worker.title}` }));
  fireEvent.click(screen.getByRole("button", { name: "Earlier worker review" }));
  expect(JSON.parse(screen.getByTestId("worker-destination").textContent!)).toEqual({ id: "worker", runId: "earlier-run", review: true });
  fireEvent.click(screen.getByRole("button", { name: "Historical question" }));
  expect(JSON.parse(screen.getByTestId("worker-destination").textContent!)).toEqual({ id: "worker", runId: "question-run", questionId: "question" });
  fireEvent.click(screen.getByRole("button", { name: `Hide autonomous worker: ${worker.title}` }));
  expect(await screen.findByText(/View hidden/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Interactive pane" }));
  expect(screen.getByRole("textbox", { name: "Terminal draft" })).toBe(terminal);
  expect((terminal as HTMLInputElement).value).toBe("unsent terminal text");
  expect(useStore.getState().sessionId).toBe("selected-project");
});

it("migrates an exact old job result rather than inventing a worker, including original run and question/review identity", async () => {
  await writeJobRecord(scope, "draft", { hidden: [], opened: ["old-job"], view: { kind: "job", id: "old-job", runId: "old-run", questionId: "old-question", review: true } }, "project-workspace");
  await writeJobRecord(["account", "project", "old-job"], "snapshot", { detail: { job: { id: "old-job", project_id: "project", title: "Retained result" } } });
  render(<Workspace />);
  expect(JSON.parse((await screen.findByTestId("earlier-destination")).textContent!)).toEqual({ id: "old-job", runId: "old-run", questionId: "old-question", review: true });
  expect(screen.getByText(/This job is not a persistent worker/)).toBeTruthy();
  expect(screen.getByRole("button", { name: /Earlier result: Retained result/ }).textContent).toContain("Cached");
  expect(screen.queryByTestId("worker-destination")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Interactive pane" }));
  fireEvent.click(screen.getByRole("button", { name: /Earlier result: Retained result/ }));
  expect(JSON.parse(screen.getByTestId("earlier-destination").textContent!)).toEqual({ id: "old-job", runId: "old-run", questionId: "old-question", review: true });
  await act(async () => { await cache.purgeWorkerCache("account", "project"); });
  expect(screen.queryByTestId("earlier-destination")).toBeNull();
  expect(screen.queryByText(/Retained result/)).toBeNull();
});

it("restores an opened worker label outside the inventory page as cached and purges it with hidden views", async () => {
  await cache.writeWorkerRecord(scope, "draft", { hidden: [], opened: ["older-worker"], results: [], hiddenResults: [], view: { kind: "interactive" } }, "project-workspace");
  await cache.writeWorkerRecord(["account", "project", "older-worker"], "snapshot", { detail: { worker: { ...worker, id: "older-worker", title: "Older responsibility" } } });
  render(<Workspace />);
  const tab = await screen.findByRole("button", { name: "Autonomous worker: Older responsibility" });
  expect(tab.textContent).toContain("Cached");
  await act(async () => { await cache.purgeWorkerCache("account", "project"); });
  expect(screen.queryByText("Older responsibility")).toBeNull();
  expect(useInventory).toHaveBeenLastCalledWith(null);
});

it("does not replace explicit pane selection with late Overview preference hydration", async () => {
  await cache.writeWorkerRecord(scope, "draft", { hidden: [], opened: [], results: [], hiddenResults: [], view: { kind: "overview" } }, "project-workspace");
  const original = cache.readWorkerRecord;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  vi.spyOn(cache, "readWorkerRecord").mockImplementation(async (...args) => { await gate; return original(...args); });
  const { rerender } = render(<Workspace />);
  fireEvent.click(screen.getByRole("button", { name: "Interactive pane" }));
  await act(async () => { release(); });
  await screen.findByRole("button", { name: `Autonomous worker: ${worker.title}` });
  rerender(<Workspace initialOverview />);
  expect(screen.getByRole("button", { name: "Project overview" }).getAttribute("aria-pressed")).toBe("false");
});

it("fences delayed preference and label restores after project purge", async () => {
  const original = cache.readWorkerRecord;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  vi.spyOn(cache, "readWorkerRecord").mockImplementation(async (...args) => {
    const result = await original(...args);
    await gate;
    return result;
  });
  await cache.writeWorkerRecord(scope, "draft", { hidden: [], opened: ["worker"], results: [], hiddenResults: [], view: { kind: "worker", id: "worker" } }, "project-workspace");
  render(<Workspace />);
  await act(async () => { await cache.purgeWorkerCache("account", "project"); release(); });
  expect(screen.queryByRole("button", { name: `Autonomous worker: ${worker.title}` })).toBeNull();
  expect(screen.queryByTestId("worker-destination")).toBeNull();
  await waitFor(() => expect(useInventory).toHaveBeenLastCalledWith(null));
});

it("opens a fresh independent worker only as an unapproved draft", async () => {
  render(<Workspace initialOverview />);
  await screen.findByRole("button", { name: `Autonomous worker: ${worker.title}` });
  fireEvent.click(screen.getByRole("button", { name: "New autonomous worker" }));
  expect(screen.getByTestId("conversion-source").textContent).toBe("");
  expect(screen.queryByTestId("worker-destination")).toBeNull();
  expect(useStore.getState().sessionId).toBe("selected-project");
  fireEvent.click(screen.getByRole("button", { name: "Acknowledge approval receipt" }));
  expect(JSON.parse(screen.getByTestId("worker-destination").textContent!)).toEqual({ id: "worker" });
});

it("drops protected opened labels synchronously when the account changes", async () => {
  await cache.writeWorkerRecord(scope, "draft", { hidden: [], opened: ["account-only"], results: [], hiddenResults: [], view: { kind: "interactive" } }, "project-workspace");
  await cache.writeWorkerRecord(["account", "project", "account-only"], "snapshot", { detail: { worker: { ...worker, id: "account-only", title: "Private responsibility" } } });
  render(<Workspace />);
  await screen.findByRole("button", { name: "Autonomous worker: Private responsibility" });
  inventory.workers = [];
  act(() => useStore.setState({ userId: "different-account" }));
  expect(screen.queryByText("Private responsibility")).toBeNull();
  expect(screen.queryByTestId("worker-destination")).toBeNull();
});

it("drops retired bot conversion provenance from saved composer preferences", async () => {
  await cache.writeWorkerRecord(scope, "draft", { hidden: [], opened: [], results: [], hiddenResults: [], view: { kind: "compose", source: { kind: "bot", session_id: "legacy-session", pane_id: 42 } } }, "project-workspace");
  render(<Workspace />);
  expect((await screen.findByTestId("conversion-source")).textContent).toBe("");
  expect(screen.queryByTestId("worker-destination")).toBeNull();
  expect(useStore.getState().sessionId).toBe("selected-project");
});

// The Overview on desktop and on the phone: the caller's panels come first and
// the workers section last, in the page's own small parts.
it("lays the Overview out as one page: the project's name, the caller's panels, then autonomous workers", async () => {
  render(<Workspace initialOverview />);
  const page = await screen.findByRole("region", { name: "Project overview" });
  // Not the Jobs page: its stylesheet's large buttons and titles do not apply.
  expect(page.classList.contains("jobs-app")).toBe(false);
  expect(within(page).getByRole("heading", { level: 1 }).textContent).toBe("Project");
  const panels = within(page).getByRole("region", { name: "Interactive project" });
  const workers = await within(page).findByRole("heading", { level: 2, name: "Autonomous workers" });
  expect(panels.compareDocumentPosition(workers) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(within(page).getByRole("heading", { level: 3, name: "Earlier results" })).toBeTruthy();
  expect(within(page).queryByRole("heading", { name: "Project overview" })).toBeNull();
});

it("keeps every worker function on the Overview", async () => {
  await cache.writeWorkerRecord(scope, "draft", { hidden: [worker.id], opened: [], results: [], hiddenResults: [], view: { kind: "overview" } }, "project-workspace");
  render(<Workspace />);
  expect(await screen.findByText(worker.description)).toBeTruthy();
  expect(useInventory).toHaveBeenCalledWith("project");
  expect(screen.getByText("idle")).toBeTruthy();
  expect(screen.getByText(/View hidden/)).toBeTruthy();
  expect(screen.getByText("approved-host")).toBeTruthy();
  expect(screen.getByText("Next check")).toBeTruthy();
  expect(screen.getByText("Last confirmed update")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Show full description" }));
  expect(screen.getByRole("button", { name: "Collapse description" }).getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByText(/loaded subset/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Load more project workers" }));
  expect(inventory.loadMore).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Refresh project workers" }));
  expect(inventory.refresh).toHaveBeenCalled();
  // Opening a hidden worker shows its view and its tab again.
  fireEvent.click(screen.getByRole("button", { name: `Open ${worker.title}` }));
  expect(JSON.parse(screen.getByTestId("worker-destination").textContent!)).toEqual({ id: "worker" });
  expect(screen.getByRole("button", { name: `Autonomous worker: ${worker.title}` }).getAttribute("aria-pressed")).toBe("true");
  // A worker's own screens keep the Jobs style and their way back.
  expect(screen.getByRole("region", { name: "Project autonomous work" }).classList.contains("jobs-app")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Project overview" , pressed: false }));
  expect(await screen.findByRole("heading", { level: 2, name: "Autonomous workers" })).toBeTruthy();
});

it("is where a worker is created, now that the tab bar's + no longer offers one", async () => {
  inventory.workers = []; inventory.hasMore = false;
  render(<Workspace initialOverview />);
  expect(await screen.findByText(/No autonomous workers in this project yet/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "New autonomous worker" }));
  expect(screen.getByTestId("conversion-source").textContent).toBe("");
  expect(screen.getByRole("button", { name: "Back to interactive agents" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Acknowledge approval receipt" }));
  expect(JSON.parse(screen.getByTestId("worker-destination").textContent!)).toEqual({ id: "worker" });
});

it("loads, lists, pages and opens earlier results from the Overview", async () => {
  earlier.jobs = [{ id: "old-job", title: "Retained result", state: "candidate_ready" }]; earlier.hasMore = true;
  render(<Workspace initialOverview />);
  const load = await screen.findByRole("button", { name: "Load earlier project results" });
  // Nothing is fetched or shown until it is asked for.
  expect(useEarlier).toHaveBeenLastCalledWith(null);
  expect(screen.queryByText("Retained result")).toBeNull();
  fireEvent.click(load);
  expect(useEarlier).toHaveBeenLastCalledWith("project");
  expect(earlier.refresh).toHaveBeenCalledOnce();
  expect(screen.getByText("Retained result")).toBeTruthy();
  expect(screen.getByText("candidate ready · Read-only")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Load more earlier results" }));
  expect(earlier.loadMore).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Open earlier result: Retained result" }));
  expect(JSON.parse(screen.getByTestId("earlier-destination").textContent!)).toEqual({ id: "old-job" });
  expect(screen.getByText(/This job is not a persistent worker/)).toBeTruthy();
});

it("says so when there are no earlier results or the worker list is stale", async () => {
  inventory.ready = false;
  render(<Workspace initialOverview />);
  expect((await screen.findByText(/Offline or reconnecting/)).getAttribute("role")).toBe("status");
  fireEvent.click(screen.getByRole("button", { name: "Load earlier project results" }));
  expect(screen.getByText("No retained earlier results.")).toBeTruthy();
});

it("has no Back to interactive agents on the Overview: the caller's panels and tab bar lead to the tabs", async () => {
  render(<Workspace initialOverview />);
  const page = await screen.findByRole("region", { name: "Project overview" });
  const terminal = screen.getByLabelText("Terminal draft");
  expect(terminal.closest("[hidden]")).not.toBeNull();
  // Not hidden, not kept for the keyboard: it is not on this page at all.
  expect(screen.queryByRole("button", { name: "Back to interactive agents" })).toBeNull();
  // The caller's panel opens a tab, and the mounted terminal is the same one.
  fireEvent.click(within(page).getByRole("button", { name: "Open a tab" }));
  expect(screen.queryByRole("region", { name: "Project overview" })).toBeNull();
  expect(screen.getByRole("button", { name: "Project overview" }).getAttribute("aria-pressed")).toBe("false");
  expect(screen.getByRole("textbox", { name: "Terminal draft" })).toBe(terminal);
  expect(terminal.closest("[hidden]")).toBeNull();
  // So does the caller's own tab bar.
  fireEvent.click(screen.getByRole("button", { name: "Project overview" }));
  fireEvent.click(screen.getByRole("button", { name: "Interactive pane" }));
  expect(screen.queryByRole("region", { name: "Project overview" })).toBeNull();
});

it("keeps both ways out on a worker's own screens, which are pages of their own", async () => {
  render(<Workspace />);
  fireEvent.click(await screen.findByRole("button", { name: `Autonomous worker: ${worker.title}` }));
  const screenOfWorker = screen.getByRole("region", { name: "Project autonomous work" });
  expect(within(screenOfWorker).getByRole("button", { name: "Project overview" })).toBeTruthy();
  fireEvent.click(within(screenOfWorker).getByRole("button", { name: "Back to interactive agents" }));
  expect(screen.queryByRole("region", { name: "Project autonomous work" })).toBeNull();
  expect(screen.getByLabelText("Terminal draft").closest("[hidden]")).toBeNull();
});

it("shows the caller's own summary line under the project's name", async () => {
  render(<Workspace initialOverview summary="This project's tabs and its autonomous workers." />);
  const page = await screen.findByRole("region", { name: "Project overview" });
  expect(within(page).getByText("This project's tabs and its autonomous workers.")).toBeTruthy();
  expect(within(page).queryByText(/what they have used/)).toBeNull();
});

it("removes the workers and the caller's panels from the Overview when access changes, and rechecks on request", async () => {
  render(<Workspace initialOverview />);
  expect(await screen.findByText(worker.description)).toBeTruthy();
  await act(async () => { await cache.purgeWorkerCache("account", "project"); });
  expect(screen.getByRole("alert").textContent).toMatch(/Worker access changed/);
  expect(screen.queryByText(worker.description)).toBeNull();
  expect(screen.queryByRole("region", { name: "Interactive project" })).toBeNull();
  expect(screen.queryByRole("button", { name: "New autonomous worker" })).toBeNull();
  expect(useInventory).toHaveBeenLastCalledWith(null);
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Check current access" })); });
  expect(await screen.findByText(worker.description)).toBeTruthy();
  expect(screen.getByRole("region", { name: "Interactive project" })).toBeTruthy();
});
