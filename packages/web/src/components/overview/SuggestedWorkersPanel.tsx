"use client";

import { useEffect } from "react";
import { Check, X, GitBranch, UserCircle2 } from "lucide-react";
import { useStore } from "@/lib/store";
import type { SuggestedWorker } from "@/lib/store";

/**
 * Manager-proposed worker suggestions, parsed from `suggested-workers.md`.
 * Each card has Accept (spawns the worker as a managed pane) and Dismiss
 * (drops the section from the file). Empty state hints at the Suggest
 * workers button on the page header.
 */
export function SuggestedWorkersPanel() {
  const sessionId = useStore((s) => s.sessionId);
  const suggestions = useStore((s) =>
    s.sessionId ? s.suggestedWorkersBySession.get(s.sessionId) ?? null : null,
  );
  const fetchSuggestedWorkers = useStore((s) => s.fetchSuggestedWorkers);
  const acceptSuggestion = useStore((s) => s.acceptSuggestion);
  const dismissSuggestion = useStore((s) => s.dismissSuggestion);

  useEffect(() => {
    if (sessionId) {
      fetchSuggestedWorkers();
    }
  }, [sessionId, fetchSuggestedWorkers]);

  if (suggestions === null) {
    return (
      <div className="rounded border border-dashed border-gray-300 p-3 text-sm text-gray-500 dark:border-gray-700 dark:text-gray-400">
        Loading suggestions…
      </div>
    );
  }

  if (suggestions.length === 0) {
    return (
      <div className="rounded border border-dashed border-gray-300 p-3 text-sm text-gray-500 dark:border-gray-700 dark:text-gray-400">
        No suggestions yet. Click <span className="font-medium">Suggest workers</span> above to ask the Manager for proposals — they'll appear here with one-click Accept buttons.
      </div>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
      {suggestions.map((s) => (
        <SuggestionCard
          key={s.id}
          suggestion={s}
          onAccept={() => acceptSuggestion(s)}
          onDismiss={() => dismissSuggestion(s.id)}
        />
      ))}
    </div>
  );
}

function SuggestionCard({
  suggestion,
  onAccept,
  onDismiss,
}: {
  suggestion: SuggestedWorker;
  onAccept: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="flex flex-col rounded border border-emerald-300 bg-emerald-50 p-3 dark:border-emerald-800 dark:bg-emerald-900/20">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <UserCircle2 className="h-4 w-4 flex-shrink-0 text-emerald-600 dark:text-emerald-400" />
          <h4 className="truncate text-sm font-semibold text-gray-900 dark:text-gray-100">
            {suggestion.label || suggestion.role || suggestion.id}
          </h4>
        </div>
        <span className="rounded bg-emerald-200 px-1.5 py-0.5 font-mono text-xs text-emerald-800 dark:bg-emerald-800 dark:text-emerald-100">
          {suggestion.id}
        </span>
      </div>

      <div className="mb-2 space-y-1 text-xs text-gray-700 dark:text-gray-300">
        {suggestion.role && (
          <div>
            <span className="font-medium text-gray-500 dark:text-gray-400">role:</span>{" "}
            {suggestion.role}
          </div>
        )}
        {suggestion.goal && (
          <div>
            <span className="font-medium text-gray-500 dark:text-gray-400">goal:</span>{" "}
            {suggestion.goal}
          </div>
        )}
        {suggestion.backstory && (
          <div className="line-clamp-2">
            <span className="font-medium text-gray-500 dark:text-gray-400">backstory:</span>{" "}
            {suggestion.backstory}
          </div>
        )}
        {suggestion.needs_worktree && (
          <div className="flex items-center gap-1 text-amber-700 dark:text-amber-400">
            <GitBranch className="h-3 w-3" />
            <span>Will get an isolated worktree</span>
          </div>
        )}
      </div>

      <div className="mt-auto flex gap-2">
        <button
          type="button"
          onClick={onAccept}
          className="flex flex-1 items-center justify-center gap-1 rounded bg-emerald-600 px-2 py-1 text-xs font-medium text-white transition-colors hover:bg-emerald-700"
          title="Spawn this worker as a managed team member"
        >
          <Check className="h-3 w-3" />
          Accept
        </button>
        <button
          type="button"
          onClick={onDismiss}
          className="flex items-center justify-center gap-1 rounded border border-gray-300 bg-white px-2 py-1 text-xs font-medium text-gray-700 transition-colors hover:bg-gray-100 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700"
          title="Drop this suggestion"
        >
          <X className="h-3 w-3" />
          Dismiss
        </button>
      </div>
    </div>
  );
}
