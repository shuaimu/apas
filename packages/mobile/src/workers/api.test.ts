import { workersApi } from "./api";
import { validAccessToken } from "@/api/client";

jest.mock("@/api/client", () => ({
  ApiError: class extends Error { readonly status: number; constructor(message: string, status: number) { super(message); this.status = status; } },
  validAccessToken: jest.fn(async () => "account-token"),
}));
jest.mock("@/config/endpoints", () => ({ endpoints: { apiUrl: "https://apas.example" } }));
jest.mock("@apas/protocol", () => ({ validateJobContract: jest.fn(() => ({ valid: true })), validateWorkerContract: jest.fn(() => ({ valid: true })) }));
const fetchMock = jest.fn(); const originalFetch = global.fetch;
beforeEach(() => { global.fetch = fetchMock; fetchMock.mockReset(); jest.mocked(validAccessToken).mockClear(); });
afterAll(() => { global.fetch = originalFetch; });

test("lost approval acknowledgement cannot automatically create another recurring grant", async () => {
  fetchMock.mockRejectedValueOnce(new Error("lost after commit"));
  await expect(workersApi.approve("project", "", { request_id: "original", preview_id: "reviewed", fingerprint: "digest" })).rejects.toThrow();
  expect(fetchMock).toHaveBeenCalledTimes(1);
  fetchMock.mockResolvedValueOnce({ ok: true, json: async () => ({ request_id: "original", worker_id: "same-worker" }) });
  await workersApi.receipt("original");
  expect(fetchMock.mock.calls[1][0]).toBe("https://apas.example/projects/worker-requests/original");
  expect(fetchMock.mock.calls[1][1].method).toBe("GET");
});

test("authentication expiry never replays worker lifecycle POST", async () => {
  fetchMock.mockResolvedValueOnce({ ok: false, status: 401, json: async () => ({ error: "expired" }) });
  await expect(workersApi.mutate("project", "worker", "pause", { request_id: "original", expected_version: 7 })).rejects.toMatchObject({ status: 401 });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(validAccessToken).not.toHaveBeenCalledWith(true);
});

test("project inventory filters before pagination and review retains requested historical run", async () => {
  fetchMock.mockResolvedValue({ ok: true, json: async () => ({}) });
  await workersApi.list("project/one", "next page", "paused");
  expect(fetchMock.mock.calls[0][0]).toBe("https://apas.example/projects/project%2Fone/workers?limit=40&cursor=next%20page&status=paused");
  await workersApi.review("project", "worker", "old-run", "files");
  expect(fetchMock.mock.calls[1][0]).toContain("/workers/worker/runs/old-run/review?limit=40&cursor=files");
  await workersApi.answer("project", "worker", "old-run", "question", { request_id: "answer", run_id: "old-run", epoch: 3, expected_version: 4, answers: [] });
  expect(fetchMock.mock.calls[2][0]).toContain("/workers/worker/runs/old-run/questions/question/answers");
  expect(JSON.parse(fetchMock.mock.calls[2][1].body)).toMatchObject({ request_id: "answer", run_id: "old-run", epoch: 3, expected_version: 4 });
});
