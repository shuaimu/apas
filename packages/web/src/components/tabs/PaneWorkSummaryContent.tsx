"use client";

import React from "react";
import type {
  PaneWorkSummary,
  PaneWorkSummaryAvailability,
  PaneWorkSummaryCache,
} from "@/lib/store";

export function formatSummaryWindow(
  start: string,
  end: string,
  locale?: string,
  timeZone?: string,
): string {
  const formatter = new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    ...(timeZone ? { timeZone } : {}),
  });
  return `${formatter.format(new Date(start))} – ${formatter.format(new Date(end))}`;
}

export function summaryAvailabilityMessage(
  availability: PaneWorkSummaryAvailability,
): string | null {
  switch (availability) {
    case "cli_update_required":
      return "The project CLI needs an update before it can generate new summaries. Cached summaries remain available.";
    case "summarizer_disabled":
      return "Summary generation is disabled on the project host.";
    case "summarizer_unavailable":
      return "The isolated summarizer is currently unavailable. Normal agent work is unaffected.";
    case "unknown":
      return "Summary generation support has not been confirmed yet.";
    default:
      return null;
  }
}

export function summaryStatusLabel(summary: PaneWorkSummary): string {
  switch (summary.status) {
    case "complete": return "Complete";
    case "partial": return "In progress";
    case "queued": return "Queued";
    case "generating": return "Generating";
    case "stale": return "Updating";
    case "failed": return "Failed";
    case "source_expired": return "Source expired";
  }
}

export function canRetrySummary(summary: PaneWorkSummary): boolean {
  return summary.status === "failed";
}

export function PaneWorkSummaryCard({
  summary,
  onRetry,
  retryDisabled = false,
}: {
  summary: PaneWorkSummary;
  onRetry: (windowStart: string) => void;
  retryDisabled?: boolean;
}) {
  const pending = summary.status === "queued" || summary.status === "generating";
  return (
    <article className="rounded-lg border border-gray-200 bg-white p-3 shadow-sm dark:border-gray-700 dark:bg-gray-900">
      <div className="flex items-start justify-between gap-3">
        <time className="text-xs font-medium text-gray-700 dark:text-gray-200">
          {formatSummaryWindow(summary.windowStart, summary.windowEnd)}
        </time>
        <span
          className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
            summary.status === "failed" || summary.status === "source_expired"
              ? "bg-red-100 text-red-700 dark:bg-red-950 dark:text-red-300"
              : summary.status === "complete"
                ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300"
                : "bg-amber-100 text-amber-700 dark:bg-amber-950 dark:text-amber-300"
          }`}
        >
          {summaryStatusLabel(summary)}
        </span>
      </div>

      {summary.summary && (
        <p className="mt-2 whitespace-pre-wrap text-sm leading-5 text-gray-700 dark:text-gray-200">
          {summary.summary}
        </p>
      )}

      {pending && !summary.summary && (
        <div className="mt-3 flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400" role="status">
          <span className="h-2 w-2 animate-pulse rounded-full bg-blue-500" />
          {summary.status === "queued" ? "Waiting for the project summarizer…" : "Summarizing retained activity…"}
        </div>
      )}

      {summary.status === "stale" && (
        <p className="mt-2 text-xs text-amber-700 dark:text-amber-300">
          Later activity changed this window; a fresh summary is being prepared.
        </p>
      )}

      {summary.status === "source_expired" && (
        <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">
          The retained conversation source has already expired, so this window cannot be reconstructed.
        </p>
      )}

      {summary.status === "failed" && (
        <div className="mt-2">
          <p className="text-xs text-red-600 dark:text-red-300">
            {summary.error || "Summary generation failed."}
          </p>
          <button
            type="button"
            onClick={() => onRetry(summary.windowStart)}
            disabled={retryDisabled || !canRetrySummary(summary)}
            className="mt-2 rounded border border-gray-300 px-2 py-1 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-800"
          >
            Retry
          </button>
        </div>
      )}

      <div className="mt-2 flex flex-wrap gap-x-2 text-[10px] text-gray-400 dark:text-gray-500">
        {summary.status === "partial" && summary.sourceThrough && (
          <span>Through {new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(summary.sourceThrough))}</span>
        )}
        <span>{summary.sourceMessageCount} source {summary.sourceMessageCount === 1 ? "event" : "events"}</span>
        {summary.provider && <span>via {summary.provider}{summary.model ? ` · ${summary.model}` : ""}</span>}
      </div>
    </article>
  );
}

export function PaneWorkSummaryList({
  cache,
  onRetry,
  retryDisabled = false,
}: {
  cache?: PaneWorkSummaryCache;
  onRetry: (windowStart: string) => void;
  retryDisabled?: boolean;
}) {
  // Before the first cache snapshot arrives, "unknown" only means that the
  // request is in flight. Showing the mixed-version warning at that point made
  // every drawer open look like summary support still needed confirmation.
  const message = !cache || (cache.loading && cache.availability === "unknown")
    ? null
    : summaryAvailabilityMessage(cache.availability);
  return (
    <>
      {message && (
        <div className="rounded-md border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          {message}
        </div>
      )}
      {cache?.loading && cache.summaries.length === 0 && (
        <div className="py-8 text-center text-sm text-gray-500" role="status">Loading summaries…</div>
      )}
      {!cache?.loading && cache?.summaries.length === 0 && (
        <div className="py-8 text-center text-sm text-gray-500">
          No meaningful retained activity has been summarized for this pane yet.
        </div>
      )}
      {cache?.summaries.map((summary) => (
        <PaneWorkSummaryCard
          key={`${summary.windowStart}-${summary.sourceDigest}`}
          summary={summary}
          onRetry={onRetry}
          retryDisabled={retryDisabled}
        />
      ))}
    </>
  );
}
