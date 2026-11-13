"use client";

import { useMemo } from "react";
import { useStore } from "./store";
import {
  autopilotListStatus,
  paneAutopilotKey,
  paneAutopilotSummary,
  paneCanHaveAutopilot,
  TAB_AUTOPILOT_CAPABILITY,
  type AutopilotListStatus,
} from "./autopilot";

/** Whether the server negotiated autopilot. Without it no frame arrives and
 *  no request is accepted, so nothing about autopilot is shown at all. */
export function useAutopilotNegotiated(): boolean {
  return useStore((state) => state.negotiatedCapabilities.has(TAB_AUTOPILOT_CAPABILITY));
}

/**
 * The autopilot of each of the attached session's tabs that has one to show:
 * on, paused, or ended with a reason lists still show. From the state the
 * page holds for a tab, else from the session list's block. Tabs that cannot
 * have an autopilot, and tabs with nothing to show, have no entry.
 */
export function useSessionAutopilotStatuses(
  sessionId: string | null,
  panes: readonly { pane_id: number; kind?: string | null; provider?: string | null; mode?: string | null }[],
): Map<number, AutopilotListStatus> {
  const negotiated = useAutopilotNegotiated();
  const paneAutopilot = useStore((state) => state.paneAutopilot);
  const listedPanes = useStore((state) =>
    sessionId ? state.sessions.find((session) => session.id === sessionId)?.panes : undefined);
  return useMemo(() => {
    const statuses = new Map<number, AutopilotListStatus>();
    if (!negotiated || !sessionId) return statuses;
    for (const pane of panes) {
      if (!paneCanHaveAutopilot(pane)) continue;
      const status = autopilotListStatus(paneAutopilotSummary(
        paneAutopilot[paneAutopilotKey(sessionId, pane.pane_id)],
        listedPanes?.find((listed) => listed.pane_id === pane.pane_id)?.autopilot,
      ));
      if (status) statuses.set(pane.pane_id, status);
    }
    return statuses;
  }, [listedPanes, negotiated, paneAutopilot, panes, sessionId]);
}
