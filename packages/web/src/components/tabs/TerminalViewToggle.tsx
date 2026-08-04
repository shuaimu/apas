"use client";

import type { TerminalViewMode } from "@/lib/terminalViewMode";

/**
 * Switches a terminal pane between the live pty and the structured
 * conversation read out of the provider's transcript.
 *
 * The conversation side is explicitly labelled read-only. It is a *reading* of
 * the transcript, so it lags the terminal by up to one poll, shows only
 * user/assistant turns, and cannot be typed into — presenting it as an equal
 * alternative would invite someone to try driving the agent from it and
 * wonder why nothing happens.
 */
export function TerminalViewToggle({
  mode,
  onChange,
  turnCount,
}: {
  mode: TerminalViewMode;
  onChange: (mode: TerminalViewMode) => void;
  turnCount: number;
}) {
  const base =
    "px-2 py-0.5 text-[11px] font-medium transition-colors first:rounded-l last:rounded-r";
  const on = "bg-gray-700 text-white dark:bg-gray-600";
  const off =
    "bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-400 dark:hover:bg-gray-700";

  return (
    <div className="flex items-center gap-2 border-b border-gray-200 px-3 py-1 dark:border-gray-700">
      <div className="flex overflow-hidden rounded" role="group" aria-label="Terminal pane view">
        <button
          type="button"
          aria-pressed={mode === "terminal"}
          onClick={() => onChange("terminal")}
          className={`${base} ${mode === "terminal" ? on : off}`}
          title="The live terminal — interactive"
        >
          Terminal
        </button>
        <button
          type="button"
          aria-pressed={mode === "conversation"}
          onClick={() => onChange("conversation")}
          className={`${base} ${mode === "conversation" ? on : off}`}
          title="Structured conversation read from the provider's transcript — read-only"
        >
          Conversation
        </button>
      </div>
      {mode === "conversation" && (
        <span className="text-[11px] text-gray-500 dark:text-gray-400">
          {turnCount === 0
            ? "No turns captured yet — the transcript is read every few seconds."
            : `${turnCount} turn${turnCount === 1 ? "" : "s"} · read-only, type in the Terminal view`}
        </span>
      )}
    </div>
  );
}
