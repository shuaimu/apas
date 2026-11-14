/**
 * How long ago something happened, in the short form lists use: `just now`,
 * `5m ago`, `3h ago`, `12d ago`.
 *
 * One formatter for every place that shows the same moment, so a tab's card
 * and its row in a table never disagree ("44s ago" beside "just now").
 *
 * Anything under a minute is `just now`, and so is a moment in the future:
 * server timestamps are compared with this browser's clock, and a browser
 * that runs a few seconds behind must not read "-2s ago".
 */
export function formatRelativeTime(when: Date | number, now: number = Date.now()): string {
  const at = typeof when === "number" ? when : when.getTime();
  if (Number.isNaN(at)) return "—";
  const minutes = Math.floor((now - at) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}
