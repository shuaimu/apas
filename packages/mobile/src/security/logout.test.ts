import { runLogoutAndWipe, type LogoutDependencies } from "./logoutOrchestrator";

describe("logoutAndWipe", () => {
  it("wipes credentials, encrypted cache, and state after successful revocation", async () => {
    const dependencies: LogoutDependencies = {
      stopConnection: jest.fn(),
      revokeRemote: jest.fn(async () => undefined),
      clearProtectedCredentials: jest.fn(async () => undefined),
      wipeEncryptedCache: jest.fn(async () => undefined),
      resetState: jest.fn(),
    };
    await runLogoutAndWipe(dependencies);
    expect(dependencies.stopConnection).toHaveBeenCalled();
    expect(dependencies.clearProtectedCredentials).toHaveBeenCalled();
    expect(dependencies.wipeEncryptedCache).toHaveBeenCalled();
    expect(dependencies.resetState).toHaveBeenCalled();
  });

  it("still wipes local protected state when the server is unreachable", async () => {
    const dependencies: LogoutDependencies = {
      stopConnection: jest.fn(),
      revokeRemote: jest.fn(async () => { throw new Error("offline"); }),
      clearProtectedCredentials: jest.fn(async () => undefined),
      wipeEncryptedCache: jest.fn(async () => undefined),
      resetState: jest.fn(),
    };
    await expect(runLogoutAndWipe(dependencies)).rejects.toThrow("offline");
    expect(dependencies.clearProtectedCredentials).toHaveBeenCalled();
    expect(dependencies.wipeEncryptedCache).toHaveBeenCalled();
    expect(dependencies.resetState).toHaveBeenCalled();
  });
});
