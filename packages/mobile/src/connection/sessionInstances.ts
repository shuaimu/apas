import type { ServerToWeb } from "@apas/protocol";

import { useMobileStore } from "@/state/store";

/**
 * Keep the live instance set of an attached session. The server sends the
 * complete set right after `session_attached` and again whenever it changes,
 * but only to connections that advertised `project_instances_v1`: an older
 * native build would reject the frame and reconnect.
 */
export function handleSessionInstancesMessage(message: ServerToWeb): boolean {
  if (message.type !== "session_instances") return false;
  useMobileStore.getState().setSessionInstances(message.session_id, message.instances);
  return true;
}
