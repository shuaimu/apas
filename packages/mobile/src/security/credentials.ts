import * as Application from "expo-application";
import * as Crypto from "expo-crypto";
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

const KEYS = {
  accessToken: "apas.mobile.access-token",
  accessExpiresAt: "apas.mobile.access-expires-at",
  refreshToken: "apas.mobile.refresh-token",
  refreshExpiresAt: "apas.mobile.refresh-expires-at",
  deviceSessionId: "apas.mobile.device-session-id",
  installationId: "apas.mobile.installation-id",
  cacheKey: "apas.mobile.cache-key",
} as const;

export interface StoredCredentials {
  accessToken: string;
  accessExpiresAt: string;
  refreshToken: string;
  refreshExpiresAt: string;
  deviceSessionId: string;
}

const secureOptions: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
};

export async function getInstallationId(): Promise<string> {
  const current = await SecureStore.getItemAsync(KEYS.installationId);
  if (current) return current;
  const created = Crypto.randomUUID();
  await SecureStore.setItemAsync(KEYS.installationId, created, secureOptions);
  return created;
}

export async function getDeviceName(): Promise<string> {
  const nativeName = Application.applicationName?.trim();
  return nativeName ? `${nativeName} on ${Platform.OS}` : `APAS Code on ${Platform.OS}`;
}

export async function saveCredentials(credentials: StoredCredentials): Promise<void> {
  await Promise.all([
    SecureStore.setItemAsync(KEYS.accessToken, credentials.accessToken, secureOptions),
    SecureStore.setItemAsync(KEYS.accessExpiresAt, credentials.accessExpiresAt, secureOptions),
    SecureStore.setItemAsync(KEYS.refreshToken, credentials.refreshToken, secureOptions),
    SecureStore.setItemAsync(KEYS.refreshExpiresAt, credentials.refreshExpiresAt, secureOptions),
    SecureStore.setItemAsync(KEYS.deviceSessionId, credentials.deviceSessionId, secureOptions),
  ]);
}

export async function loadCredentials(): Promise<StoredCredentials | null> {
  const [accessToken, accessExpiresAt, refreshToken, refreshExpiresAt, deviceSessionId] =
    await Promise.all([
      SecureStore.getItemAsync(KEYS.accessToken),
      SecureStore.getItemAsync(KEYS.accessExpiresAt),
      SecureStore.getItemAsync(KEYS.refreshToken),
      SecureStore.getItemAsync(KEYS.refreshExpiresAt),
      SecureStore.getItemAsync(KEYS.deviceSessionId),
    ]);
  if (!accessToken || !accessExpiresAt || !refreshToken || !refreshExpiresAt || !deviceSessionId) {
    return null;
  }
  return { accessToken, accessExpiresAt, refreshToken, refreshExpiresAt, deviceSessionId };
}

export async function getOrCreateCacheKey(): Promise<string> {
  const current = await SecureStore.getItemAsync(KEYS.cacheKey);
  if (current) return current;
  const bytes = await Crypto.getRandomBytesAsync(32);
  const key = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  await SecureStore.setItemAsync(KEYS.cacheKey, key, secureOptions);
  return key;
}

export async function clearCredentials(options: { preserveInstallation?: boolean } = {}): Promise<void> {
  const keys: string[] = [
    KEYS.accessToken,
    KEYS.accessExpiresAt,
    KEYS.refreshToken,
    KEYS.refreshExpiresAt,
    KEYS.deviceSessionId,
    KEYS.cacheKey,
  ];
  if (!options.preserveInstallation) keys.push(KEYS.installationId);
  await Promise.all(keys.map((key) => SecureStore.deleteItemAsync(key)));
}

export const credentialStorageBackend = "expo-secure-store" as const;
