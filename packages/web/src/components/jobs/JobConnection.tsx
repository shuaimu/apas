"use client";

import { useEffect } from "react";
import { validateServerMessage } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { purgeJobCache } from "@/lib/jobCache";
import { receiveJobMessage } from "@/lib/jobs";
import type { JobWireMessage } from "@/lib/jobs";

/** Observes the existing authenticated socket; never owns a second connection. */
export function JobConnection() {
  const socket = useStore((state) => state.ws);
  useEffect(() => useStore.subscribe((state, previous) => {
    if (previous.userId && (state.userId !== previous.userId || !state.token || state.accountStatus === "suspended" && previous.accountStatus !== "suspended")) {
      void purgeJobCache(previous.userId).catch(() => useStore.getState().showToast("Job cache could not be cleared. Clear this site's browser data before sharing this device.", "error"));
    }
  }), []);
  useEffect(() => {
    const changed = (event: StorageEvent) => {
      if (event.key === "apas_token" || event.key === "apas_user_id") {
        void purgeJobCache().finally(() => window.location.reload());
      }
    };
    window.addEventListener("storage", changed);
    return () => window.removeEventListener("storage", changed);
  }, []);
  useEffect(() => {
    if (!socket) return;
    const receive = (event: MessageEvent) => {
      try {
        const message = JSON.parse(String(event.data));
        if (message.type === "project_access_changed" && message.project_id) {
          void purgeJobCache(useStore.getState().userId ?? undefined, message.project_id).catch(() => useStore.getState().showToast("Job cache purge failed. Clear browser site data.", "error"));
        }
        if (typeof message.type === "string" && message.type.startsWith("job_") && validateServerMessage(message).valid) {
          receiveJobMessage(message as JobWireMessage);
        }
      } catch { /* The shared transport reports malformed frames. */ }
    };
    socket.addEventListener("message", receive);
    return () => socket.removeEventListener("message", receive);
  }, [socket]);
  return null;
}
