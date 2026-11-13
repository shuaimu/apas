import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { WorkerConversionSource, WorkerSummary } from "@apas/protocol";
import { useStore } from "@/lib/store";
import * as cache from "@/lib/workerCache";
import { writeJobRecord } from "@/lib/jobCache";
import { ProjectWorkersWorkspace } from "./ProjectWorkersWorkspace";

const inventory = vi.hoisted(() => ({ workers: [] as WorkerSummary[], ready: true, loading: false, error: "", hasMore: true, refresh: vi.fn(), loadMore: vi.fn() }));
const useInventory = vi.hoisted(() => vi.fn());
vi.mock("@/lib/useProjectWorkers", () => ({ useProjectWorkers: (project: string | null) => { useInventory(project); return inventory; } }));
vi.mock("@/lib/useProjectJobs", () => ({ useProjectJobs: () => ({ jobs: [], ready: true, loading: false, error: "", hasMore: false, refresh: vi.fn(), loadMore: vi.fn() }) }));
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
function Workspace({ initialOverview = false }: { initialOverview?: boolean }) {
  return <ProjectWorkersWorkspace project={project} paneSelectionRequested={false} initialOverview={initialOverview}>
    {({ active, tabs, overviewTab, content, selectInteractive, addAutonomous }) => <>
      {overviewTab}{tabs}<button onClick={selectInteractive}>Interactive pane</button><button disabled={!addAutonomous} onClick={() => addAutonomous?.()}>Add worker</button>
      <div hidden={active}><input aria-label="Terminal draft" defaultValue="retained input" /></div>{content}
    </>}
  </ProjectWorkersWorkspace>;
}

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); Element.prototype.scrollIntoView = vi.fn(); });
beforeEach(async () => {
  await cache.purgeWorkerCache();
  useStore.setState({ userId: "account", sessionId: "selected-project" });
  inventory.workers = [worker]; inventory.ready = true; inventory.hasMore = true;
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
  render(<Workspace />);
  await screen.findByRole("button", { name: `Autonomous worker: ${worker.title}` });
  fireEvent.click(screen.getByRole("button", { name: "Add worker" }));
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
