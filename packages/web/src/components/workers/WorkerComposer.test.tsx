import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { JobTarget, WorkerDetail, WorkerMandateSpec, WorkerPreview, WorkerTargetsResponse } from "@apas/protocol";
import type { SavedWorkerRequest } from "@/lib/workerCache";
import type * as WorkerHooks from "@/lib/useWorkers";
import { purgeWorkerCache, readWorkerRecord, writeWorkerRecord } from "@/lib/workerCache";
import { writeJobRecord } from "@/lib/jobCache";
import { WorkerComposer, eligibleWorkerTarget } from "./WorkerComposer";

const session = vi.hoisted(() => ({ userId: "account", sessions: [], cliClients: [], machines: [] }));
const transport = vi.hoisted(() => ({ fetch: vi.fn(), replace: vi.fn() }));
const mutation = vi.hoisted(() => ({ request: undefined as SavedWorkerRequest | undefined, busy: false, error: "", unresolved: false, retryAllowed: false, resolve: vi.fn(), submit: vi.fn() }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ replace: transport.replace }) }));
vi.mock("@/lib/store", () => ({ useStore: Object.assign((selector: (state: typeof session) => unknown) => selector(session), { getState: () => session }) }));
vi.mock("@/lib/workers", () => ({ workerFetch: transport.fetch, workerPath: (project: string, worker?: string) => `/projects/${project}/workers${worker ? `/${worker}` : ""}`, workerHref: (project: string, worker: string) => `/code/workers/${worker}?project=${project}` }));
vi.mock("@/lib/useWorkers", async (original) => ({ ...await original<typeof WorkerHooks>(), useWorkerOnline: () => true, useWorkerMutation: () => mutation }));

const perRun = { elapsed_seconds: 60, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000, memory_bytes: 1024, workspace_bytes: 2048, output_bytes: 4096, pids: 16, model_concurrency: 1, model_output_tokens: 1000 };
const spec: WorkerMandateSpec = { title: "Maintain accessibility", description: "Find and repair inaccessible project controls", checklist: ["Inspect keyboard navigation", "Preserve screen reader labels"], machine_id: "machine", worker_profile: "omp", model_profile: "model", verification_profile: "checks", base_ref: "main", interval_seconds: 900, limits: { per_run: perRun, elapsed_seconds_24h: 600, spend_micro_usd_24h: 10000, runs_24h: 4, attempts_per_item: 1 } };
const target: JobTarget = {
  project_id: "project", machine_id: "machine", cluster_owner_user_id: "owner", hostname: "Qualified host", available: true, unavailable_reasons: [],
  allowed_actions: { admit: true, answer: false, cancel: false, retry: false, retry_publication: false, manage_policy: false },
  policy: { digest: "policy", sources: [], policy: { enabled: true, publish_enabled: true, worker_profiles: ["omp"], model_profiles: ["model"], verification_profiles: ["checks"], limits: perRun, max_active_project: 1, max_active_machine: 1, max_active_cluster: 1, max_queued_project: 4, recurring: { enabled: true, max_elapsed_seconds_24h: 600, max_spend_micro_usd_24h: 10000, max_runs_24h: 4, min_interval_seconds: 900 } } },
  profiles: [{ key: "omp", label: "Qualified OMP", digest: "omp-digest", image: "worker-image", omp_version: "version", ready: true, unavailable_reasons: [], limits: perRun, model_profiles: [{ key: "model", label: "Explicit model", model: "model-name", api: "openai_responses" }], verification_profiles: [{ key: "checks", label: "Independent checks", digest: "checks-digest", image: "checks-image" }] }],
};
const targets: WorkerTargetsResponse = { project_id: "project", targets: [target], unavailable_reasons: [] };
const preview: WorkerPreview = { id: "preview", project_id: "project", worker_id: null, expected_version: null, fingerprint: "fingerprint", expires_at: "2099-01-01T00:00:00Z", mandate: { spec, revision: 1, repository: "example/project", worker_profile_digest: "omp-digest", verification_profile_digest: "checks-digest", policy_digest: "policy", approved_by: "account", approved_at: "2026-01-01T00:00:00Z" }, cluster_owner_user_id: "owner", hostname: "Qualified host", base_sha: "a".repeat(40), policy: target.policy, source: null };

beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(async () => {
  await purgeWorkerCache();
  transport.fetch.mockReset(); transport.replace.mockReset(); mutation.submit.mockReset();
  mutation.request = undefined; mutation.unresolved = false; session.userId = "account";
  transport.fetch.mockImplementation(async (_path: string, kind: string) => kind === "targets" ? targets : preview);
});
afterEach(cleanup);
afterAll(() => { vi.unstubAllGlobals(); });

it("retains a routed project before authorized inventory arrives", async () => {
  render(<WorkerComposer initialProject="project" />);
  expect((screen.getByLabelText("Project") as HTMLSelectElement).value).toBe("project");
  expect(screen.getByRole("option", { name: "project" })).toBeTruthy();
  await waitFor(() => expect(transport.fetch).toHaveBeenCalledWith("/projects/project/worker-targets", "targets"));
});

it("requires independent recurring permission and intersected worker/model/verifier readiness", () => {
  expect(eligibleWorkerTarget(target)).toBe(true);
  expect(eligibleWorkerTarget({ ...target, allowed_actions: { ...target.allowed_actions, admit: false } })).toBe(false);
  expect(eligibleWorkerTarget({ ...target, policy: { ...target.policy, policy: { ...target.policy.policy, recurring: { enabled: false, max_elapsed_seconds_24h: 600, max_spend_micro_usd_24h: 10000, max_runs_24h: 4, min_interval_seconds: 900 } } } })).toBe(false);
  expect(eligibleWorkerTarget({ ...target, policy: { ...target.policy, policy: { ...target.policy.policy, recurring: undefined } } })).toBe(false);
  expect(eligibleWorkerTarget({ ...target, policy: { ...target.policy, policy: { ...target.policy.policy, worker_profiles: [] } } })).toBe(false);
  expect(eligibleWorkerTarget({ ...target, profiles: [{ ...target.profiles[0], ready: false }] })).toBe(false);
});

it("preserves an unavailable saved host instead of following the project hint", async () => {
  await writeWorkerRecord(["account", "project", ""], "draft", { spec: { ...spec, machine_id: "missing-host" } }, "composer:new");
  render(<WorkerComposer project={{ id: "project", name: "Project", machineId: "machine" }} />);
  await waitFor(() => expect((screen.getByLabelText("Execution machine") as HTMLSelectElement).value).toBe("missing-host"));
  expect(screen.getByText(/saved execution machine is unavailable/)).toBeTruthy();
  expect((screen.getByRole("button", { name: "Review exact recurring mandate" }) as HTMLButtonElement).disabled).toBe(true);
  expect(mutation.submit).not.toHaveBeenCalled();
});

it("uses only an eligible host hint and never defaults the model", async () => {
  render(<WorkerComposer project={{ id: "project", name: "Project", machineId: "machine" }} />);
  await waitFor(() => expect((screen.getByLabelText("Execution machine") as HTMLSelectElement).value).toBe("machine"));
  expect((screen.getByLabelText("Model profile") as HTMLSelectElement).value).toBe("");
  expect((screen.getByLabelText("OMP worker profile") as HTMLSelectElement).value).toBe("");
});

it("offers legacy text without copying finite approval, provider or recurring allowance", async () => {
  await writeJobRecord(["account", "project", ""], "draft", { instruction: "Earlier task", acceptance: "Earlier criterion", machine: "legacy", model: "legacy-model", preview: { id: "old-approval" }, limits: perRun }, "composer");
  render(<WorkerComposer project={{ id: "project", name: "Project" }} />);
  fireEvent.click(await screen.findByRole("button", { name: "Copy text into unapproved mandate" }));
  expect((screen.getByLabelText("Job description") as HTMLTextAreaElement).value).toBe("Earlier task");
  expect((screen.getByLabelText("Standing checklist (one item per line)") as HTMLTextAreaElement).value).toBe("Earlier criterion");
  expect((screen.getByLabelText("Execution machine") as HTMLSelectElement).value).toBe("");
  expect((screen.getByLabelText("Inspection interval (seconds)") as HTMLInputElement).value).toBe("");
  expect(screen.queryByRole("button", { name: "Approve recurring worker" })).toBeNull();
});

it("previews complete reviewed inputs and navigates only after durable approval", async () => {
  await writeWorkerRecord(["account", "project", ""], "draft", { spec, pristine: false }, "composer:new");
  const onApproved = vi.fn();
  const { rerender } = render(<WorkerComposer project={{ id: "project", name: "Project" }} onApproved={onApproved} />);
  await waitFor(() => expect((screen.getByRole("button", { name: "Review exact recurring mandate" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Review exact recurring mandate" }));
  const approve = await screen.findByRole("button", { name: "Approve recurring worker" });
  expect(transport.fetch).toHaveBeenCalledWith("/projects/project/worker-previews", "preview", { worker_id: null, expected_version: null, spec, source: null });
  expect(screen.getByText(/rolling 24-hour allowance renews automatically/i)).toBeTruthy();
  expect(onApproved).not.toHaveBeenCalled();
  fireEvent.click(approve);
  await waitFor(() => expect(mutation.submit).toHaveBeenCalledTimes(1));
  const request = mutation.submit.mock.calls[0][0] as SavedWorkerRequest;
  expect(request.body).toEqual({ request_id: request.id, preview_id: "preview", fingerprint: "fingerprint" });
  expect(onApproved).not.toHaveBeenCalled();
  mutation.request = { ...request, receipt: { request_id: request.id, operation: "approve", status: "accepted", worker_id: "acknowledged", project_id: "project", state_version: 1, error_code: null } };
  rerender(<WorkerComposer project={{ id: "project", name: "Project" }} onApproved={onApproved} />);
  await waitFor(() => expect(onApproved).toHaveBeenCalledWith("acknowledged", "project"));
});

it("fences late target responses and draft writes behind project revocation", async () => {
  let release!: (value: WorkerTargetsResponse) => void;
  transport.fetch.mockImplementation(() => new Promise<WorkerTargetsResponse>((resolve) => { release = resolve; }));
  render(<WorkerComposer project={{ id: "project", name: "Project", machineId: "machine" }} />);
  await waitFor(() => expect(transport.fetch).toHaveBeenCalled());
  await act(async () => { await purgeWorkerCache("account", "project"); release(targets); });
  expect(screen.queryByLabelText("Execution machine")).toBeNull();
  expect(await readWorkerRecord(["account", "project", ""], "targets")).toBeUndefined();
  expect(screen.getByText(/Access changed. Reopen the project/)).toBeTruthy();
});

it("discards a preview that arrives after protected project data was purged", async () => {
  await writeWorkerRecord(["account", "project", ""], "draft", { spec, pristine: false }, "composer:new");
  let release!: (value: WorkerPreview) => void;
  transport.fetch.mockImplementation((_path: string, kind: string) => kind === "targets" ? Promise.resolve(targets) : new Promise<WorkerPreview>((resolve) => { release = resolve; }));
  render(<WorkerComposer project={{ id: "project", name: "Project" }} />);
  await waitFor(() => expect((screen.getByRole("button", { name: "Review exact recurring mandate" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Review exact recurring mandate" }));
  await waitFor(() => expect(transport.fetch).toHaveBeenCalledWith("/projects/project/worker-previews", "preview", expect.any(Object)));
  await act(async () => { await purgeWorkerCache("account", "project"); release(preview); });
  expect(screen.queryByRole("button", { name: "Approve recurring worker" })).toBeNull();
  expect(await readWorkerRecord(["account", "project", ""], "draft", "composer:new")).toBeUndefined();
  expect(mutation.submit).not.toHaveBeenCalled();
});

it("reapproves existing worker identity with an optimistic version rather than creating another", async () => {
  const worker: WorkerDetail = { worker: { id: "existing", project_id: "project", title: spec.title, description: spec.description, state: "paused", reason: null, desired_state: "paused", mandate_revision: 2, backlog_revision: 3, state_version: 7, high_watermark: 8, next_check_at: null, active_job_id: null, active_run_id: null, current_item_id: null, machine_id: "machine", cluster_owner_user_id: "owner", hostname: "Qualified host", attention: false, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" }, mandate: preview.mandate, checkpoint: null, budget: { usage: { elapsed_seconds: 0, input_tokens: 0, output_tokens: 0, spend_micro_usd: 0, estimated: false }, runs: 0, reserved_elapsed_seconds: 0, reserved_spend_micro_usd: 0, reserved_runs: 0, accounting_uncertain: false, eligible_at: null, window_started_at: "2026-01-01T00:00:00Z" }, allowed_actions: { edit_mandate: true, edit_backlog: false, pause: false, stop: false, resume: true, wake: false, retry: false, answer: false, retry_publication: false }, current_job: null };
  transport.fetch.mockImplementation(async (_path: string, kind: string) => kind === "targets" ? targets : { ...preview, worker_id: "existing", expected_version: 7 });
  render(<WorkerComposer worker={worker} />);
  await waitFor(() => expect((screen.getByRole("button", { name: "Review exact recurring mandate" }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Review exact recurring mandate" }));
  fireEvent.click(await screen.findByRole("button", { name: "Approve replacement mandate" }));
  await waitFor(() => expect(mutation.submit).toHaveBeenCalledTimes(1));
  expect(transport.fetch).toHaveBeenCalledWith("/projects/project/worker-previews", "preview", { worker_id: "existing", expected_version: 7, spec, source: null });
  expect(mutation.submit.mock.calls[0][0].path).toBe("/projects/project/workers/existing/mandate");
});
