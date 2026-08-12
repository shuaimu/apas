import NetInfo from "@react-native-community/netinfo";
import { AppState } from "react-native";

import { ApiError, bootstrap, validAccessToken } from "@/api/client";
import { applyCodeEvents, normalizeServerMessage } from "@apas/protocol";
import { endpoints } from "@/config/endpoints";
import { ConnectionSupervisor, type SocketLike } from "@/connection/supervisor";
import { handlePaneWorkSummaryMessage, reconcileVisiblePaneWorkSummaries } from "@/connection/workSummaries";
import {
  acceptEvents,
  removeInaccessibleSessions,
  replaceSessionSummaries,
  wipeCache,
} from "@/storage/cache";
import { useMobileStore } from "@/state/store";
import { clearCredentials } from "@/security/credentials";
import { publishTerminalMessage } from "@/terminal/events";
import { startNotificationAndLinkRuntime, stopNotificationAndLinkRuntime } from "@/notifications";

let supervisor: ConnectionSupervisor | null = null;
let cleanup: (() => void) | null = null;

export function startConnectionRuntime(): ConnectionSupervisor {
  if (supervisor) return supervisor;
  let sequence = 0;
  supervisor = new ConnectionSupervisor({
    createSocket: () => new WebSocket(`${endpoints.wsUrl}/ws/web`) as unknown as SocketLike,
    accessToken: validAccessToken,
    bootstrap,
    persistBootstrap: async (value) => {
      const allowed = new Set(value.sessions.map((session) => session.id));
      await removeInaccessibleSessions(allowed);
      await replaceSessionSummaries(value.sessions);
    },
    setPhase: useMobileStore.getState().setConnection,
    setMutationsAllowed: useMobileStore.getState().setServerMutationsAllowed,
    setNegotiatedCapabilities: useMobileStore.getState().setNegotiatedCapabilities,
    applyBootstrap: (value) => {
      useMobileStore.getState().applyBootstrap(value);
      if (value.features.notifications || value.features.deep_links) {
        startNotificationAndLinkRuntime();
      }
    },
    onMessage: (message) => {
      if (handlePaneWorkSummaryMessage(message)) return;
      if (message.type === "session_attached") {
        useMobileStore.getState().setSessionActive(message.session_id, message.has_active_cli);
      }
      if (message.type === "pane_list") {
        useMobileStore.getState().setPanes(message.session_id, message.panes);
      }
      if (message.type === "pane_status") {
        const paneId = message.pane_id
          ?? (message.pane_type === "deadloop" ? 1 : message.pane_type === "interactive" ? 2 : null);
        if (paneId !== null) {
          useMobileStore.getState().setPaneStatus(message.session_id, paneId, message.status);
        }
      }
      publishTerminalMessage(message);
      const events = normalizeServerMessage(message, {
        receivedAt: new Date().toISOString(),
        sequence: sequence++,
      });
      if (events.length === 0) return;
      const sessionId = events[0].session_id;
      const current = useMobileStore.getState().eventsBySession[sessionId] ?? [];
      const accepted = applyCodeEvents(current, events);
      useMobileStore.getState().setEvents(sessionId, accepted);
      void acceptEvents(events).catch(() => undefined);
    },
    onSynchronized: () => {
      reconcileVisiblePaneWorkSummaries((message) => supervisor?.send(message) ?? false);
    },
    isAuthenticationLoss: (error) => error instanceof ApiError && [401, 403].includes(error.status),
    onAuthenticationLost: () => {
      void (async () => {
        await clearCredentials({ preserveInstallation: true });
        await wipeCache();
        useMobileStore.getState().reset();
      })();
    },
  });
  const networkSubscription = NetInfo.addEventListener((state) => {
    supervisor?.setNetworkOnline(Boolean(state.isConnected && state.isInternetReachable !== false));
  });
  const appSubscription = AppState.addEventListener("change", (state) => {
    supervisor?.setForeground(state === "active");
  });
  cleanup = () => {
    networkSubscription();
    appSubscription.remove();
  };
  supervisor.start();
  return supervisor;
}

export function stopConnectionRuntime(): void {
  supervisor?.stop();
  supervisor = null;
  cleanup?.();
  stopNotificationAndLinkRuntime();
  cleanup = null;
}

export function connectionSupervisor(): ConnectionSupervisor | null {
  return supervisor;
}
