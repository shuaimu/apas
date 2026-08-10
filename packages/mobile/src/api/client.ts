import Constants from "expo-constants";
import { Platform } from "react-native";
import type {
  MobileAuthResponse,
  MobileBootstrapResponse,
  MobileDeviceSession,
  MobileLoginRequest,
  MobileNotificationPreferences,
  MobilePushTokenRequest,
  MobileTaskLaunchRequest,
  MobileTaskLaunchResponse,
} from "@apas/protocol";

import { endpoints } from "@/config/endpoints";
import {
  clearCredentials,
  getDeviceName,
  getInstallationId,
  loadCredentials,
  saveCredentials,
  type StoredCredentials,
} from "@/security/credentials";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function responseError(response: Response): Promise<ApiError> {
  const body = (await response.json().catch(() => null)) as { error?: string; message?: string } | null;
  return new ApiError(body?.error ?? body?.message ?? `Request failed (${response.status})`, response.status);
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`${endpoints.apiUrl}${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", ...init.headers },
  });
  if (!response.ok) throw await responseError(response);
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

function stored(response: MobileAuthResponse): StoredCredentials {
  return {
    accessToken: response.access_token,
    accessExpiresAt: response.access_expires_at,
    refreshToken: response.refresh_token,
    refreshExpiresAt: response.refresh_expires_at,
    deviceSessionId: response.device_session_id,
  };
}

export async function login(email: string, password: string): Promise<MobileAuthResponse> {
  const body: MobileLoginRequest = {
    email: email.trim(),
    password,
    installation_id: await getInstallationId(),
    platform: Platform.OS === "ios" ? "ios" : "android",
    device_name: await getDeviceName(),
    app_version: Constants.expoConfig?.version ?? "0.0.0",
  };
  const response = await request<MobileAuthResponse>("/auth/mobile/login", {
    method: "POST",
    body: JSON.stringify(body),
  });
  await saveCredentials(stored(response));
  return response;
}

export async function refresh(): Promise<MobileAuthResponse> {
  const current = await loadCredentials();
  if (!current) throw new ApiError("This device is signed out", 401);
  const response = await request<MobileAuthResponse>("/auth/mobile/refresh", {
    method: "POST",
    body: JSON.stringify({
      refresh_token: current.refreshToken,
      installation_id: await getInstallationId(),
    }),
  });
  await saveCredentials(stored(response));
  return response;
}

export async function validAccessToken(forceRefresh = false): Promise<string> {
  const current = await loadCredentials();
  if (!current) throw new ApiError("This device is signed out", 401);
  const expiresSoon = Date.parse(current.accessExpiresAt) - Date.now() < 60_000;
  if (forceRefresh || expiresSoon) return (await refresh()).access_token;
  return current.accessToken;
}

export async function bootstrap(): Promise<MobileBootstrapResponse> {
  const token = await validAccessToken();
  try {
    return await request<MobileBootstrapResponse>("/mobile/v1/bootstrap", {
      headers: { Authorization: `Bearer ${token}` },
    });
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 401) throw error;
    const retryToken = await validAccessToken(true);
    return request<MobileBootstrapResponse>("/mobile/v1/bootstrap", {
      headers: { Authorization: `Bearer ${retryToken}` },
    });
  }
}

export async function listDevices(): Promise<MobileDeviceSession[]> {
  const token = await validAccessToken();
  return request<MobileDeviceSession[]>("/mobile/v1/devices", {
    headers: { Authorization: `Bearer ${token}` },
  });
}

export async function revokeDevice(deviceSessionId: string): Promise<void> {
  const token = await validAccessToken();
  await request(`/mobile/v1/devices/${encodeURIComponent(deviceSessionId)}/revoke`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
  });
}

export async function logoutRemote(): Promise<void> {
  const current = await loadCredentials();
  if (!current) return;
  try {
    await request("/auth/mobile/logout", {
      method: "POST",
      headers: { Authorization: `Bearer ${current.accessToken}` },
      body: JSON.stringify({ refresh_token: current.refreshToken }),
    });
  } finally {
    await clearCredentials({ preserveInstallation: true });
  }
}

export async function registerPushToken(value: MobilePushTokenRequest): Promise<void> {
  const token = await validAccessToken();
  await request("/mobile/v1/push-token", {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
    body: JSON.stringify(value),
  });
}

export async function getNotificationPreferences(): Promise<MobileNotificationPreferences> {
  const token = await validAccessToken();
  return request<MobileNotificationPreferences>("/mobile/v1/notification-preferences", {
    headers: { Authorization: `Bearer ${token}` },
  });
}

export async function updateNotificationPreferences(
  preferences: MobileNotificationPreferences,
): Promise<MobileNotificationPreferences> {
  const token = await validAccessToken();
  return request<MobileNotificationPreferences>("/mobile/v1/notification-preferences", {
    method: "PUT",
    headers: { Authorization: `Bearer ${token}` },
    body: JSON.stringify(preferences),
  });
}

export async function launchTask(
  task: MobileTaskLaunchRequest,
): Promise<MobileTaskLaunchResponse> {
  const token = await validAccessToken();
  try {
    return await request<MobileTaskLaunchResponse>("/mobile/v1/task-launches", {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
      body: JSON.stringify(task),
    });
  } catch (error) {
    if (!(error instanceof ApiError) || error.status !== 401) throw error;
    const retryToken = await validAccessToken(true);
    return request<MobileTaskLaunchResponse>("/mobile/v1/task-launches", {
      method: "POST",
      headers: { Authorization: `Bearer ${retryToken}` },
      body: JSON.stringify(task),
    });
  }
}
