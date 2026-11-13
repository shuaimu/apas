import type { MobilePaneSummary, PaneConfig } from "@apas/protocol";

export const PANE_STATUS_PENDING_ANSWER = "Pending answer";

export function paneStatusIsPendingAnswer(status: string | null | undefined): boolean {
  return status === PANE_STATUS_PENDING_ANSWER;
}

export function paneStatusIsWorking(status: string | null | undefined): boolean {
  return Boolean(status) && !paneStatusIsPendingAnswer(status);
}

export function paneIsRetiredBot(pane: PaneConfig | MobilePaneSummary | undefined): boolean {
  return pane?.mode === "deadloop";
}
