import { logoutRemote } from "@/api/client";
import { stopConnectionRuntime } from "@/connection/runtime";
import { clearCredentials } from "@/security/credentials";
import { wipeCache } from "@/storage/cache";
import { useMobileStore } from "@/state/store";
import { runLogoutAndWipe, type LogoutDependencies } from "@/security/logoutOrchestrator";

const defaultDependencies: LogoutDependencies = {
  stopConnection: stopConnectionRuntime,
  revokeRemote: logoutRemote,
  clearProtectedCredentials: () => clearCredentials({ preserveInstallation: true }),
  wipeEncryptedCache: wipeCache,
  resetState: useMobileStore.getState().reset,
};

export async function logoutAndWipe(
  dependencies: LogoutDependencies = defaultDependencies,
): Promise<void> {
  return runLogoutAndWipe(dependencies);
}
