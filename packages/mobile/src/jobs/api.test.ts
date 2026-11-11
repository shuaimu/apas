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

test("a lost admission reply never causes automatic mutation replay", async () => {
  fetchMock.mockRejectedValueOnce(new Error("connection lost after commit"));
  await expect(jobsApi.admit("project", { request_id: "original", preview_id: "preview", fingerprint: "digest" })).rejects.toBeInstanceOf(Error);
  expect(fetchMock).toHaveBeenCalledTimes(1);
});

test("even an authentication failure does not automatically replay a mutation", async () => {
  fetchMock.mockResolvedValueOnce({ ok: false, status: 401, json: async () => ({ error: "expired" }) });
  await expect(jobsApi.mutate("project", "job", "cancel", { request_id: "original", expected_version: 2 })).rejects.toMatchObject({ status: 401 });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(validAccessToken).not.toHaveBeenCalledWith(true);
});

