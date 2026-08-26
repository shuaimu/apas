export const PANE_STATUS_PENDING_ANSWER = "Pending answer";

export function paneStatusIsPendingAnswer(status: string | null | undefined): boolean {
  return status === PANE_STATUS_PENDING_ANSWER;
}

export function paneStatusIsWorking(status: string | null | undefined): boolean {
  return Boolean(status) && !paneStatusIsPendingAnswer(status);
}
