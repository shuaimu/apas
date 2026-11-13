import { jobsApi } from "./api";
import { validAccessToken } from "@/api/client";

jest.mock("@/api/client", () => ({
  ApiError: class extends Error {
    readonly status: number;
    constructor(message: string, status: number) { super(message); this.status = status; }
  },
  validAccessToken: jest.fn(async () => "current-account-token"),
}));
jest.mock("@/config/endpoints", () => ({ endpoints: { apiUrl: "https://apas.example" } }));
jest.mock("@apas/protocol", () => ({ validateJobContract: jest.fn(() => ({ valid: true })) }));
const fetchMock = jest.fn();
const originalFetch = global.fetch;
beforeEach(() => { global.fetch = fetchMock; fetchMock.mockReset(); jest.mocked(validAccessToken).mockClear(); });
afterAll(() => { global.fetch = originalFetch; });

test("earlier job review preserves its requested run and uses read-only HTTP", async () => {
  fetchMock.mockResolvedValueOnce({ ok: true, json: async () => ({}) });
  await jobsApi.review("project", "job", null, "historical-run");
  expect(fetchMock).toHaveBeenCalledWith("https://apas.example/projects/project/jobs/job/review?limit=40&run_id=historical-run", expect.objectContaining({ method: "GET" }));
});

test("earlier admission receipts remain resolvable without replaying retired mutations", async () => {
  fetchMock.mockResolvedValueOnce({ ok: true, json: async () => ({ request_id: "original", status: "accepted" }) });
  await expect(jobsApi.receipt("original")).resolves.toMatchObject({ request_id: "original" });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(fetchMock.mock.calls[0][1].method).toBe("GET");
});

