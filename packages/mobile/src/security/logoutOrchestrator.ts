export interface LogoutDependencies {
  stopConnection: () => void;
  revokeRemote: () => Promise<void>;
  clearProtectedCredentials: () => Promise<void>;
  wipeEncryptedCache: () => Promise<void>;
  resetState: () => void;
}

export async function runLogoutAndWipe(dependencies: LogoutDependencies): Promise<void> {
  dependencies.stopConnection();
  try {
    await dependencies.revokeRemote();
  } finally {
    await dependencies.clearProtectedCredentials();
    await dependencies.wipeEncryptedCache();
    dependencies.resetState();
  }
}
