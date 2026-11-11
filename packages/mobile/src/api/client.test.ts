import type { MobileTaskLaunchRequest } from "@apas/protocol";

import { ApiError, launchTask } from "./client";

jest.mock("@/config/endpoints", () => ({ endpoints: { apiUrl: "https://apas.example" } }));
jest.mock("@/security/credentials", () => ({
  loadCredentials: jest.fn(async () => ({
    accessToken: "current-access",
    accessExpiresAt: "2999-01-01T00:00:00Z",
    refreshToken: "current-refresh",
    refreshExpiresAt: "2999-01-01T00:00:00Z",
    deviceSessionId: "device-session",
  })),
  saveCredentials: jest.fn(async () => undefined),
  clearCredentials: jest.fn(async () => undefined),
  getDeviceName: jest.fn(async () => "test device"),
  getInstallationId: jest.fn(async () => "installation"),
}));

const fetchMock = jest.fn();
const originalFetch = global.fetch;
beforeEach(() => { global.fetch = fetchMock; fetchMock.mockReset(); });
afterAll(() => { global.fetch = originalFetch; });

const request: MobileTaskLaunchRequest = {
  request_id: "04cbf715-81d0-42ca-86c5-87913e77c2c9",
  machine_id: "50000000-0000-4000-8000-000000000002",
  project_id: "30000000-0000-4000-8000-000000000001",
  instruction: "Run the slow suite on zoo-006",
  profile_key: "terminal:codex:official:default",
};

test("a refused launch target surfaces the server's reason and is never resent", async () => {
  const reason = "zoo-006 cannot run this project beside the instance on zoo-005; reboot zoo-006 to update it";
  fetchMock.mockResolvedValueOnce({ ok: false, status: 409, json: async () => ({ error: reason }) });

  const launch = launchTask(request);
  await expect(launch).rejects.toBeInstanceOf(ApiError);
  await expect(launch).rejects.toMatchObject({ message: reason, status: 409 });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
  expect(url).toBe("https://apas.example/mobile/v1/task-launches");
  expect(JSON.parse(String(init.body))).toEqual(request);
});
