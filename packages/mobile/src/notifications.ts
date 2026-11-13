import Constants from "expo-constants";
import * as Linking from "expo-linking";
import * as Notifications from "expo-notifications";
import { Platform } from "react-native";
import { router } from "expo-router";

import { ApiError, bootstrap, registerPushToken } from "@/api/client";
import { resolveAuthorizedDeepLink } from "@/deepLinks";
import { getInstallationId } from "@/security/credentials";
import { replaceSessionSummaries } from "@/storage/cache";
import { useMobileStore } from "@/state/store";
import { jobsApi } from "@/jobs/api";
import { acceptJobsBootstrap, purgeJobAccess, useJobs } from "@/jobs/runtime";
import { consumeJobLink, parseJobLink, retainJobLink } from "@/jobs/links";
import { parseWorkerLink } from "@/workers/links";
import { workersApi } from "@/workers/api";
import { exactWorkerRun } from "@/workers/continuity";
import { authorityCurrent, handleWorkerError, workerAuthority } from "@/workers/runtime";

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
  const destination = parseJobLink(rawUrl);
  const workerDestination = parseWorkerLink(rawUrl);
  let current;
  try { current = await bootstrap(); }
  catch (error) {
    if (destination || workerDestination) retainJobLink(rawUrl);
    throw error;
  }
  useMobileStore.getState().applyBootstrap(current);
  await replaceSessionSummaries(current.sessions);
  await acceptJobsBootstrap(current);
  if (workerDestination) {
    const stamp = workerAuthority();
    let project: string | undefined;
    try {
      const detail = await workersApi.resolve(workerDestination.worker);
      project = detail.worker.project_id;
      if (detail.worker.id !== workerDestination.worker) throw new Error("Worker destination mismatch");
      if (workerDestination.run) {
        const execution = exactWorkerRun(await workersApi.run(project, workerDestination.worker, workerDestination.run), workerDestination.worker, workerDestination.run);
        if (workerDestination.question && !execution.questions.some((question) => question.id === workerDestination.question && question.run_id === workerDestination.run)) throw new Error("Requested historical question is unavailable");
        if (workerDestination.review) {
          const review = await workersApi.review(project, workerDestination.worker, workerDestination.run);
          if (review.run_id !== workerDestination.run) throw new Error("Requested historical review is unavailable");
        }
      }
      if (!authorityCurrent(stamp)) return false;
      const params = { workerId: workerDestination.worker, project };
      if (workerDestination.question && workerDestination.run) router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]/questions/[questionId]", params: { ...params, runId: workerDestination.run, questionId: workerDestination.question } });
      else if (workerDestination.review && workerDestination.run) router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]/review", params: { ...params, runId: workerDestination.run } });
      else if (workerDestination.run) router.push({ pathname: "/(code)/workers/[workerId]/runs/[runId]", params: { ...params, runId: workerDestination.run } });
      else router.push({ pathname: "/(code)/workers/[workerId]", params });
      return true;
    } catch (error) {
      if (authorityCurrent(stamp)) await handleWorkerError(error, project);
      return false;
    }
  }
  if (destination) {
    try {
      const detail = await jobsApi.resolve(destination.job);
      const params = { jobId: destination.job, project: detail.job.project_id, ...(destination.run ? { run: destination.run } : {}) };
      if (destination.question) router.push({ pathname: "/(code)/jobs/[jobId]/questions/[questionId]", params: { ...params, questionId: destination.question } });
      else if (destination.review) router.push({ pathname: "/(code)/jobs/[jobId]/review", params });
      else router.push({ pathname: "/(code)/jobs/[jobId]", params });
      return true;
    } catch (error) {
      if (error instanceof ApiError && [401, 403, 404, 410].includes(error.status)) {
        const project = useJobs.getState().entries[destination.job]?.detail.job.project_id
          ?? useJobs.getState().inbox.jobs.find((job) => job.id === destination.job)?.project_id;
        if (error.status === 401) await purgeJobAccess();
        else if (project) await purgeJobAccess(project);
      }
      // The exact destination renders a safe unavailable explanation; never attach a session.
      router.push({ pathname: "/(code)/jobs/[jobId]", params: { jobId: destination.job } });
      return false;
    }
  }
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
  const retained = consumeJobLink();
  if (retained) void handleAuthorizedLink(retained).catch(() => undefined);
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
