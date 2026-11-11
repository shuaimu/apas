import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { ProjectJobListResponse } from "@apas/protocol";
import { ProjectJobsWorkspace } from "./ProjectJobsWorkspace";
import { purgeJobCache, writeJobRecord } from "@/lib/jobCache";
import type { SavedJob } from "@/lib/jobCache";

type JobSummary = ProjectJobListResponse["jobs"][number];
const session = vi.hoisted(() => ({ userId: "account" }));
const inventory = vi.hoisted(() => ({
  jobs: [] as JobSummary[], ready: true, loading: false, error: "", hasMore: true,
  refresh: vi.fn(), loadMore: vi.fn(),
}));
vi.mock("@/lib/store", () => ({ useStore: (selector: (state: typeof session) => unknown) => selector(session) }));
vi.mock("@/lib/useProjectJobs", () => ({ useProjectJobs: () => inventory }));

const retained: SavedJob = {
  detail: {
    job: {
      id: "older", project_id: "project", project_name: "Project", title: "Retained result",
      state: "failed", state_version: 3, current_run_id: "run", cluster_owner_user_id: "owner",
      machine_id: "machine", hostname: "host", attention_count: 0, reason: null,
      created_at: "2026-11-08T00:00:00Z", updated_at: "2026-11-08T01:00:00Z",
      usage: { elapsed_seconds: 1, input_tokens: 1, output_tokens: 1, spend_micro_usd: 1, estimated: false },
      limits: { elapsed_seconds: 120, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000,
        memory_bytes: 536_870_912, workspace_bytes: 67_108_864, output_bytes: 4096,
        pids: 16, model_concurrency: 2, model_output_tokens: 1024 },
      allowed_actions: { admit: false, answer: false, cancel: false, retry: false, retry_publication: false, manage_policy: false },
    },
    task: {
      instruction: "Retain the result for review", acceptance_criteria: ["Review remains accessible"],
      source_url: null, repository: "example/project", base_ref: "main", base_sha: "a".repeat(40),
      worker_profile: "worker", worker_profile_digest: "worker-digest", model_profile: "model",
      verification_profile: "verifier", verification_profile_digest: "verifier-digest",
    },
    runs: [], questions: [], high_watermark: 0, policy_digest: "policy-digest", publication: null,
  },
  events: [], cursor: 0,
};

const originalScrollIntoView = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView");
beforeAll(() => {
  vi.stubGlobal("indexedDB", new IDBFactory());
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
});
beforeEach(async () => {
  await purgeJobCache();
  inventory.jobs = [{ ...retained.detail.job, description: retained.detail.task.instruction, id: "newer", title: "Newer result", created_at: "2026-11-09T00:00:00Z" }];
  await writeJobRecord(["account", "project", "older"], "snapshot", retained);
  await writeJobRecord(["account", "project", ""], "draft", {
    opened: ["older"], hidden: [], view: { kind: "interactive" },
  }, "project-workspace");
});
afterEach(() => { cleanup(); });
afterAll(() => {
  vi.unstubAllGlobals();
  if (originalScrollIntoView) Object.defineProperty(HTMLElement.prototype, "scrollIntoView", originalScrollIntoView);
  else Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
});

it("restores an opened older result outside the loaded page as cached, then removes it on revocation", async () => {
  render(<ProjectJobsWorkspace project={{ id: "project", name: "Project" }} paneSelectionRequested={false}>
    {(workspace) => <>{workspace.tabs}{workspace.overviewTab}{workspace.content}</>}
  </ProjectJobsWorkspace>);

  const olderTab = await screen.findByRole("button", { name: "Autonomous job: Retained result" });
  expect(olderTab.textContent).toMatch(/cached/i);
  expect(olderTab.textContent).toContain("failed");
  expect(screen.queryByRole("button", { name: "Autonomous job: Newer result" })).toBeNull();

  await act(async () => { await purgeJobCache("account", "project"); });
  expect(screen.queryByRole("button", { name: "Autonomous job: Retained result" })).toBeNull();
});

it("migrates a late hydrated Overview preference without overriding later explicit pane navigation", async () => {
  const workspace = (initialOverview: boolean) => <ProjectJobsWorkspace
    project={{ id: "project", name: "Project" }} paneSelectionRequested={false} initialOverview={initialOverview}
  >{({ overviewTab, tabs, selectInteractive }) => <>{overviewTab}{tabs}<button onClick={selectInteractive}>Interactive pane</button></>}</ProjectJobsWorkspace>;
  const { rerender } = render(workspace(false));
  const overview = screen.getByRole("button", { name: "Project overview" });
  await screen.findByRole("button", { name: "Autonomous job: Retained result" });

  rerender(workspace(true));
  expect(overview.getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(screen.getByRole("button", { name: "Interactive pane" }));
  rerender(workspace(false));
  rerender(workspace(true));
  expect(overview.getAttribute("aria-pressed")).toBe("false");
});
