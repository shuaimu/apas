export const PANE_STATUS_PENDING_ANSWER = "Pending answer";

export function paneIsAwaitingAnswerStatus(
  status: string | null | undefined,
): boolean {
  return status === PANE_STATUS_PENDING_ANSWER;
}

export function paneIsWorkingStatus(
  status: string | null | undefined,
): boolean {
  return Boolean(status) && !paneIsAwaitingAnswerStatus(status);
}
