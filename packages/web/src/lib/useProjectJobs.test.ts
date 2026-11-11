import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { ProjectJobListResponse } from "@apas/protocol";
import type * as JobApi from "./jobs";
import { useProjectJobs } from "./useProjectJobs";
import { JobHttpError, jobFetch, receiveJobMessage } from "./jobs";
import * as cache from "./jobCache";

const session = vi.hoisted(() => ({ userId: "account-a", ws: {} as object, online: true }));
vi.mock("./store", () => ({
  useStore: Object.assign((selector: (state: typeof session) => unknown) => selector(session), {
    getState: () => session,
  }),
}));
vi.mock("./useJobs", () => ({ useJobOnline: () => session.online }));
vi.mock("./jobs", async (original) => ({ ...await original<typeof JobApi>(), jobFetch: vi.fn() }));

interface Pending<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}
function pending<T>(): Pending<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const requests: { path: string; response: Pending<ProjectJobListResponse> }[] = [];
const stamp = "2026-11-09T00:00:00Z";
function job(id: string, version = 1, project = "project-a"): ProjectJobListResponse["jobs"][number] {
  return {
    id, project_id: project, project_name: project, title: id, state: "running", state_version: version,
    description: `Review ${id}: ${"保留完整任务说明。".repeat(25)}\nVerify Unicode and multiline output.`,
    current_run_id: "run", cluster_owner_user_id: "owner", machine_id: "machine", hostname: "host",
    created_at: stamp, updated_at: stamp, attention_count: 0, reason: null,
    usage: { elapsed_seconds: 1, input_tokens: 1, output_tokens: 1, spend_micro_usd: 1, estimated: false },
    limits: { elapsed_seconds: 120, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000,
      memory_bytes: 536_870_912, workspace_bytes: 67_108_864, output_bytes: 4096,
      pids: 16, model_concurrency: 2, model_output_tokens: 1024 },
    allowed_actions: { admit: false, answer: false, cancel: true, retry: false, retry_publication: false, manage_policy: false },
  };
}

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(async () => {
  session.userId = "account-a";
  session.ws = {};
  session.online = true;
  requests.length = 0;
  await cache.purgeJobCache();
  vi.mocked(jobFetch).mockImplementation(<T,>(path: string) => {
    const response = pending<ProjectJobListResponse>();
    requests.push({ path, response });
    return response.promise as Promise<T>;
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.mocked(jobFetch).mockReset(); });
afterAll(() => { vi.unstubAllGlobals(); });

describe("project job inventory isolation", () => {
  it("hides old rows immediately on project/account switches and ignores their delayed replies", async () => {
    const { result, rerender } = renderHook(({ project }) => useProjectJobs(project), { initialProps: { project: "project-a" } });
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].response.resolve({ jobs: [job("a")], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    act(() => result.current.refresh());
    expect(requests).toHaveLength(2);
    rerender({ project: "project-b" });
    expect(result.current.jobs).toEqual([]);
    expect(result.current.ready).toBe(false);
    await waitFor(() => expect(requests).toHaveLength(3));
    await act(async () => { requests[1].response.resolve({ jobs: [job("stale-a")], next_cursor: null }); });
    expect(result.current.jobs).toEqual([]);
    await act(async () => { requests[2].response.resolve({ jobs: [job("b", 1, "project-b")], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    act(() => result.current.refresh());
    session.userId = "account-b";
    rerender({ project: "project-b" });
    expect(result.current.jobs).toEqual([]);
    await waitFor(() => expect(requests).toHaveLength(5));
    await act(async () => { requests[3].response.resolve({ jobs: [job("old-account", 2, "project-b")], next_cursor: null }); });
    expect(result.current.jobs).toEqual([]);
    await act(async () => { requests[4].response.resolve({ jobs: [], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(await cache.readJobRecord(["account-b", "project-b", ""], "list", "project")).toEqual({ jobs: [], next_cursor: null });
  });

  it("invalidates cached and in-flight content on purge without resurrecting it", async () => {
    const { result } = renderHook(() => useProjectJobs("project-a"));
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].response.resolve({ jobs: [job("private")], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    act(() => result.current.loadMore());
    expect(requests).toHaveLength(2);
    await act(async () => { await cache.purgeJobCache("account-a", "project-a"); });
    expect(result.current.jobs).toEqual([]);
    expect(result.current.ready).toBe(false);
    await act(async () => { requests[1].response.resolve({ jobs: [job("revoked")], next_cursor: null }); });
    expect(result.current.jobs).toEqual([]);
    expect(await cache.readJobRecord(["account-a", "project-a", ""], "list", "project")).toBeUndefined();
    expect(requests).toHaveLength(2);
    act(() => result.current.refresh());
    expect(requests).toHaveLength(3);
    await act(async () => { requests[2].response.resolve({ jobs: [], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
  });

  it("does not let delayed cache restoration overwrite a refresh requested during hydration", async () => {
    const restored = pending<ProjectJobListResponse>();
    vi.spyOn(cache, "readJobRecord").mockImplementationOnce(<T,>() => restored.promise as Promise<T | undefined>);
    const { result } = renderHook(() => useProjectJobs("project-a"));
    act(() => result.current.refresh());
    act(() => receiveJobMessage({ type: "job_changed", project_id: "project-a", job_id: "new", state_version: 3, last_event_seq: 3 }));
    expect(requests).toHaveLength(0);
    await act(async () => { restored.resolve({ jobs: [job("old")], next_cursor: null }); });
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].response.resolve({ jobs: [job("new", 3)], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.jobs.map((value) => value.id)).toEqual(["new"]);
  });
});

describe("project job inventory refresh", () => {
  it("serializes overlapping pages and hints, deduplicates rows and preserves newer versions without prefetching history", async () => {
    const { result } = renderHook(() => useProjectJobs("project-a"));
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].response.resolve({ jobs: [job("first", 3)], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.jobs[0].description).toBe(job("first").description);
    expect(jobFetch).toHaveBeenCalledWith("/projects/project-a/jobs", "project_list");
    expect(requests).toHaveLength(1);
    act(() => { result.current.loadMore(); result.current.loadMore(); });
    expect(requests).toHaveLength(2);
    expect(requests[1].path).toBe("/projects/project-a/jobs?cursor=older");
    act(() => {
      receiveJobMessage({ type: "job_changed", project_id: "project-b", job_id: "foreign", state_version: 1, last_event_seq: 1 });
      receiveJobMessage({ type: "job_changed", project_id: "project-a", job_id: "second", state_version: 3, last_event_seq: 3 });
      receiveJobMessage({ type: "job_changed", project_id: "project-a", job_id: "first", state_version: 3, last_event_seq: 3 });
    });
    expect(requests).toHaveLength(2);
    await act(async () => { requests[1].response.resolve({ jobs: [job("first", 1), job("second", 2)], next_cursor: "still-older" }); });
    await waitFor(() => expect(requests).toHaveLength(3));
    expect(result.current.ready).toBe(false);
    expect(result.current.jobs.map((value) => [value.id, value.state_version])).toEqual([["first", 3], ["second", 2]]);
    await act(async () => { requests[2].response.resolve({ jobs: [job("first", 2)], next_cursor: "new-boundary" }); });
    await waitFor(() => expect(requests).toHaveLength(4));
    expect(requests[3].path).toBe("/projects/project-a/jobs?cursor=new-boundary");
    await act(async () => { requests[3].response.resolve({ jobs: [job("first", 1), job("second", 3)], next_cursor: "more-history" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.jobs.map((value) => [value.id, value.state_version])).toEqual([["first", 3], ["second", 3]]);
    expect(result.current.jobs.map((value) => value.description)).toEqual([job("first").description, job("second").description]);
    expect(result.current.jobs[0].allowed_actions.cancel).toBe(false);
    expect(result.current.jobs[1].allowed_actions.cancel).toBe(true);
    expect(result.current.hasMore).toBe(true);
    expect(requests).toHaveLength(4);
  });

  it("ignores obsolete summary-only cache without fabricating descriptions or purging detail", async () => {
    const { description, ...legacy } = job("legacy");
    expect(description.length).toBeGreaterThan(100);
    await cache.writeJobRecord(["account-a", "project-a", ""], "list", { jobs: [legacy], next_cursor: "older" }, "project");
    await cache.writeJobRecord(["account-a", "project-a", "legacy"], "detail", { marker: "retained" });
    session.online = false;
    const { result, rerender } = renderHook(() => useProjectJobs("project-a"));
    await act(async () => { await cache.readJobRecord(["account-a", "project-a", ""], "list", "project"); });
    expect(result.current.jobs).toEqual([]);
    expect(await cache.readJobRecord(["account-a", "project-a", "legacy"], "detail")).toEqual({ marker: "retained" });
    session.online = true;
    rerender();
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].response.resolve({ jobs: [job("legacy")], next_cursor: null }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.jobs[0].description).toBe(job("legacy").description);
  });

  it("keeps offline cache read-only and reauthorizes on reconnect before exposing controls", async () => {
    await cache.writeJobRecord(["account-a", "project-a", ""], "list", { jobs: [job("cached")], next_cursor: "older" }, "project");
    session.online = false;
    const { result, rerender } = renderHook(() => useProjectJobs("project-a"));
    await waitFor(() => expect(result.current.jobs).toHaveLength(1));
    expect(result.current.ready).toBe(false);
    expect(result.current.jobs[0].allowed_actions.cancel).toBe(false);
    act(() => result.current.loadMore());
    expect(requests).toHaveLength(0);
    session.online = true;
    rerender();
    expect(result.current.ready).toBe(false);
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => { requests[0].response.resolve({ jobs: [job("cached", 2)], next_cursor: "older" }); });
    await waitFor(() => expect(result.current.ready).toBe(true));
    expect(result.current.jobs[0].allowed_actions.cancel).toBe(true);
    session.ws = {};
    rerender();
    expect(result.current.ready).toBe(false);
    await waitFor(() => expect(requests).toHaveLength(2));
    await act(async () => { requests[1].response.reject(new JobHttpError(403, "job_access_denied", "Access denied")); });
    await waitFor(() => expect(result.current.jobs).toEqual([]));
    expect(result.current.ready).toBe(false);
    await waitFor(async () => expect(await cache.readJobRecord(["account-a", "project-a", ""], "list", "project")).toBeUndefined());
    expect(requests).toHaveLength(2);
  });
});
