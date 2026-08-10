import type { MobileBootstrapResponse, MobileSessionSummary } from "@apas/protocol";

import { handleAuthorizedLink } from "@/notifications";

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const mockBootstrap = jest.fn<Promise<MobileBootstrapResponse>, []>();
const mockReplaceSessions = jest.fn<Promise<void>, [MobileSessionSummary[]]>();
const mockRouterPush = jest.fn();

jest.mock("@/api/client", () => ({
  bootstrap: () => mockBootstrap(),
  registerPushToken: jest.fn(),
}));
jest.mock("@/storage/cache", () => ({
  replaceSessionSummaries: (sessions: MobileSessionSummary[]) => mockReplaceSessions(sessions),
}));
jest.mock("@/security/credentials", () => ({ getInstallationId: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: (...args: unknown[]) => mockRouterPush(...args) } }));
jest.mock("expo-constants", () => ({
  __esModule: true,
  default: { isDevice: false, expoConfig: { extra: {} } },
}));
jest.mock("expo-linking", () => ({
  addEventListener: jest.fn(() => ({ remove: jest.fn() })),
  getInitialURL: jest.fn(() => Promise.resolve(null)),
}));
jest.mock("expo-notifications", () => ({
  setNotificationHandler: jest.fn(),
  addNotificationResponseReceivedListener: jest.fn(() => ({ remove: jest.fn() })),
  addPushTokenListener: jest.fn(() => ({ remove: jest.fn() })),
  getLastNotificationResponse: jest.fn(() => null),
}));

function bootstrap(sessions: MobileSessionSummary[]): MobileBootstrapResponse {
  return {
    account_status: "active",
    cluster_role: "user",
    features: { notifications: true, deep_links: true },
    launch_targets: [],
    machines: [],
    protocol_max_version: 1,
    protocol_min_version: 1,
    sessions,
    user_email: "mobile@example.test",
    user_id: "15e3ea16-f83c-4338-a421-c0ae5e8dbf02",
  };
}

const session: MobileSessionSummary = {
  id: sessionId,
  project_id: "9cd95c53-90d1-472a-89c9-a3a008fc15a4",
  project_name: "notification-test",
  status: "active",
};

describe("notification and link authorization refresh", () => {
  beforeEach(() => {
    mockBootstrap.mockReset();
    mockReplaceSessions.mockReset().mockResolvedValue();
    mockRouterPush.mockReset();
  });

  it("refreshes bootstrap and cache before navigating to an accessible target", async () => {
    mockBootstrap.mockResolvedValue(bootstrap([session]));

    await expect(handleAuthorizedLink(`apas://code/session/${sessionId}`)).resolves.toBe(true);

    expect(mockReplaceSessions).toHaveBeenCalledWith([session]);
    expect(mockRouterPush).toHaveBeenCalledWith({
      pathname: "/(code)/session/[sessionId]",
      params: { sessionId },
    });
    expect(mockReplaceSessions.mock.invocationCallOrder[0]).toBeLessThan(mockRouterPush.mock.invocationCallOrder[0]);
  });

  it("treats stale, unauthorized, and missing notification targets as a safe no-op", async () => {
    mockBootstrap.mockResolvedValue(bootstrap([]));

    await expect(handleAuthorizedLink(`apas://code/session/${sessionId}`)).resolves.toBe(false);
    await expect(handleAuthorizedLink("apas://code/session/not-a-uuid")).resolves.toBe(false);
    await expect(handleAuthorizedLink("https://evil.example/code/session/missing")).resolves.toBe(false);
    expect(mockRouterPush).not.toHaveBeenCalled();
  });

  it("does not navigate when current authorization cannot be refreshed", async () => {
    mockBootstrap.mockRejectedValue(new Error("device session revoked"));

    await expect(handleAuthorizedLink("apas://code")).rejects.toThrow("device session revoked");
    expect(mockReplaceSessions).not.toHaveBeenCalled();
    expect(mockRouterPush).not.toHaveBeenCalled();
  });
});
