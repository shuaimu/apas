import Constants from "expo-constants";
import * as Linking from "expo-linking";
import * as Notifications from "expo-notifications";
import { Platform } from "react-native";
import { router } from "expo-router";

import { bootstrap, registerPushToken } from "@/api/client";
import { resolveAuthorizedDeepLink } from "@/deepLinks";
import { getInstallationId } from "@/security/credentials";
import { replaceSessionSummaries } from "@/storage/cache";
import { useMobileStore } from "@/state/store";

let cleanup: (() => void) | null = null;

Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldPlaySound: false,
    shouldSetBadge: true,
    shouldShowBanner: true,
    shouldShowList: true,
  }),
});

export async function handleAuthorizedLink(rawUrl: string): Promise<boolean> {
  const current = await bootstrap();
  useMobileStore.getState().applyBootstrap(current);
  await replaceSessionSummaries(current.sessions);
  const target = resolveAuthorizedDeepLink(rawUrl, new Set(current.sessions.map((session) => session.id)));
  if (!target) return false;
  if (target.kind === "home") router.push("/(code)/(tabs)");
  else if (target.kind === "session") router.push({ pathname: "/(code)/session/[sessionId]", params: { sessionId: target.sessionId } });
  else router.push({ pathname: "/(code)/new", params: { instruction: target.instruction } });
  return true;
}

async function registerCurrentExpoToken(): Promise<void> {
  if (!Constants.isDevice) return;
  const projectId = (Constants.expoConfig?.extra as { pushProjectId?: string } | undefined)?.pushProjectId;
  if (!projectId) throw new Error("EAS project ID is not configured");
  const token = await Notifications.getExpoPushTokenAsync({ projectId });
  await registerPushToken({
    installation_id: await getInstallationId(),
    platform: Platform.OS === "ios" ? "ios" : "android",
    token: token.data,
  });
}

export async function requestAndRegisterNotifications(): Promise<void> {
  const permission = await Notifications.getPermissionsAsync();
  const finalPermission = permission.status === "undetermined"
    ? await Notifications.requestPermissionsAsync()
    : permission;
  if (finalPermission.status !== "granted") throw new Error("Notification permission was not granted");
  await registerCurrentExpoToken();
}

export function startNotificationAndLinkRuntime(): void {
  if (cleanup) return;
  const notificationTap = Notifications.addNotificationResponseReceivedListener((response) => {
    const data = response.notification.request.content.data ?? {};
    if (typeof data.url === "string") void handleAuthorizedLink(data.url).catch(() => undefined);
  });
  const tokenRotation = Notifications.addPushTokenListener(() => {
    void registerCurrentExpoToken().catch(() => undefined);
  });
  const link = Linking.addEventListener("url", ({ url }) => {
    void handleAuthorizedLink(url).catch(() => undefined);
  });
  void Linking.getInitialURL().then((url) => url ? handleAuthorizedLink(url) : false).catch(() => undefined);
  const lastResponse = Notifications.getLastNotificationResponse();
  const notificationUrl = lastResponse?.notification.request.content.data?.url;
  if (typeof notificationUrl === "string") void handleAuthorizedLink(notificationUrl).catch(() => undefined);
  cleanup = () => {
    notificationTap.remove();
    tokenRotation.remove();
    link.remove();
  };
}

export function stopNotificationAndLinkRuntime(): void {
  cleanup?.();
  cleanup = null;
}
