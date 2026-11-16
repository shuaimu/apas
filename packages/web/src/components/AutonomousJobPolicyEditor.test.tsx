import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import type { JobPolicy, JobPolicyManagement } from "@apas/protocol";
import { purgeWorkerCache } from "@/lib/workerCache";
import { AutonomousJobPolicyEditor } from "./AutonomousJobPolicyEditor";

const account = vi.hoisted(() => ({ userId: "account" }));
vi.mock("@/lib/store", () => ({ useStore: Object.assign((selector: (state: typeof account) => unknown) => selector(account), { getState: () => account }) }));
const policy: JobPolicy = {
  enabled: true, publish_enabled: false, worker_profiles: ["omp"], model_profiles: ["model"], verification_profiles: ["checks"],
  limits: { elapsed_seconds: 60, spend_micro_usd: 1000, attempts: 1, cpu_millis: 1000, memory_bytes: 1024, workspace_bytes: 2048, output_bytes: 4096, pids: 16, model_concurrency: 1, model_output_tokens: 1000 },
  max_active_project: 1, max_active_machine: 1, max_active_cluster: 1, max_queued_project: 4,
  recurring: { enabled: false, max_elapsed_seconds_24h: 600, max_spend_micro_usd_24h: 10000, max_runs_24h: 4, min_interval_seconds: 900 },
};
const document: JobPolicyManagement = { requested: null, inherited: { policy, sources: [], digest: "inherited" }, effective: { policy, sources: [], digest: "effective" }, unavailable_reasons: ["Host still requires qualified OMP readiness"], can_manage: true };
beforeAll(() => { vi.stubGlobal("indexedDB", new IDBFactory()); });
beforeEach(async () => { account.userId = "account"; await purgeWorkerCache(); });
afterEach(cleanup);
afterAll(() => { vi.unstubAllGlobals(); });
type PolicyRequest = <T>(path: string, init?: RequestInit) => Promise<T>;

it("keeps recurring permission independently denied and omits an inherited override", async () => {
  const request = vi.fn<PolicyRequest>().mockResolvedValue(document);
  render(<AutonomousJobPolicyEditor title="Project policy" endpoint="/policy" request={request as PolicyRequest} />);
  const recurring = await screen.findByLabelText("Allow recurring autonomous workers") as HTMLInputElement;
  expect(recurring.checked).toBe(false);
  expect(recurring.closest("fieldset")?.disabled).toBe(true);
  expect((screen.getByLabelText("Allow autonomous job admission") as HTMLInputElement).checked).toBe(true);
  expect(screen.getByText("Host still requires qualified OMP readiness")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Save autonomous job policy" }));
  await waitFor(() => expect(request).toHaveBeenCalledWith("/policy", expect.objectContaining({ method: "PUT" })));
  const write = request.mock.calls.find(([, init]) => init?.method === "PUT")!;
  expect(JSON.parse(String(write[1]?.body))).not.toHaveProperty("recurring");
});

it("saves explicit recurring permission with all finite bounds and cadence", async () => {
  const request = vi.fn<PolicyRequest>().mockResolvedValue(document);
  render(<AutonomousJobPolicyEditor title="Project policy" endpoint="/policy" request={request as PolicyRequest} />);
  fireEvent.click(await screen.findByLabelText("Set explicit recurring override (otherwise inherit)"));
  fireEvent.click(screen.getByLabelText("Allow recurring autonomous workers"));
  fireEvent.change(screen.getByLabelText("Minimum inspection interval (seconds)"), { target: { value: "1800" } });
  fireEvent.click(screen.getByRole("button", { name: "Save autonomous job policy" }));
  await waitFor(() => expect(request).toHaveBeenCalledWith("/policy", expect.objectContaining({ method: "PUT" })));
  const write = request.mock.calls.find(([, init]) => init?.method === "PUT")!;
  expect(JSON.parse(String(write[1]?.body)).recurring).toEqual({ ...policy.recurring, enabled: true, min_interval_seconds: 1800 });
});

it("shows requested recurring enablement separately from effective denial", async () => {
  const requested = { ...document, requested: { ...policy, recurring: { ...policy.recurring, enabled: true } } };
  const request = vi.fn<PolicyRequest>().mockResolvedValue(requested);
  render(<AutonomousJobPolicyEditor title="Project policy" endpoint="/policy" request={request as PolicyRequest} />);
  const recurring = await screen.findByLabelText("Allow recurring autonomous workers") as HTMLInputElement;
  expect(recurring.checked).toBe(true);
  expect(screen.getByText(/Requested recurring permission: Enabled. Effective recurring permission: Disabled/)).toBeTruthy();
});

it("does not repopulate policy after a late response crosses the purge barrier", async () => {
  let release!: (value: JobPolicyManagement) => void;
  const request = vi.fn<PolicyRequest>().mockImplementation(() => new Promise<JobPolicyManagement>((resolve) => { release = resolve; }));
  render(<AutonomousJobPolicyEditor title="Project policy" endpoint="/policy" request={request as PolicyRequest} />);
  await act(async () => { await purgeWorkerCache("account", "project"); release(document); });
  expect(screen.queryByLabelText("Allow recurring autonomous workers")).toBeNull();
  expect(screen.getByRole("alert").textContent).toContain("Access changed");
});

it("names the unavailable reasons it has words for and keeps the rest as given", async () => {
  const withCodes = { ...document, unavailable_reasons: ["job_policy_disabled", "something_new"] };
  const request = vi.fn<PolicyRequest>().mockResolvedValue(withCodes);
  render(<AutonomousJobPolicyEditor
    title="Project policy"
    endpoint="/policy"
    request={request as PolicyRequest}
    reasonLabels={{ job_policy_disabled: "Worker jobs are turned off for this deployment." }}
  />);
  expect(await screen.findByText("Workers cannot run here yet:")).toBeTruthy();
  expect(screen.getByText("Worker jobs are turned off for this deployment.")).toBeTruthy();
  expect(screen.queryByText("job_policy_disabled")).toBeNull();
  expect(screen.getByText("something_new")).toBeTruthy();
});

it("keeps its heading for screen readers only when embedded", async () => {
  const request = vi.fn<PolicyRequest>().mockResolvedValue(document);
  const { container } = render(<AutonomousJobPolicyEditor title="Project policy" endpoint="/policy" request={request as PolicyRequest} embedded />);
  const heading = screen.getByRole("heading", { name: "Project policy" });
  expect(heading.className).toContain("sr-only");
  expect((container.firstElementChild as HTMLElement).className).not.toContain("border");
  // The form is the same: the nested recurring fieldset still governs its fields.
  const recurring = await screen.findByLabelText("Allow recurring autonomous workers") as HTMLInputElement;
  expect(recurring.closest("fieldset")?.disabled).toBe(true);
});
