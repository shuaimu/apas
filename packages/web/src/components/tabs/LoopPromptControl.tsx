"use client";

import { useState } from "react";
import { RotateCw } from "lucide-react";
import { LOOP_PROMPT_LABEL } from "@/lib/loopPrompt";
import { LoopPromptDialog } from "./LoopPromptDialog";

/**
 * The pane toolbar's Loop prompt control for a Codex, Claude or OpenCode
 * terminal tab, where an OMP tab has Autopilot. It only opens the dialog: it
 * shows no state, because a loop prompt has none APAS could report. Where the
 * tab cannot take a message it stays, disabled, with the reason. It touches
 * neither the terminal nor the selected view.
 */
export function LoopPromptControl({
  sessionId,
  paneId,
  tabLabel,
  hostname,
  disabledReason,
}: {
  sessionId: string;
  paneId: number;
  tabLabel: string;
  hostname?: string;
  /** Why the tab cannot take a message now. */
  disabledReason?: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div data-testid="loop-prompt-control" className="relative flex min-w-0 items-center gap-2">
      <button
        type="button"
        disabled={Boolean(disabledReason)}
        title={disabledReason ?? "Send one prompt that asks the agent to keep working in a loop. APAS does not watch the loop."}
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
        className="flex flex-shrink-0 items-center gap-1 rounded border border-gray-300 bg-white px-2 py-0.5 text-[11px] font-medium text-gray-700 transition-colors hover:bg-gray-100 disabled:cursor-not-allowed disabled:opacity-50 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
      >
        <RotateCw aria-hidden="true" className="h-3 w-3" />
        {LOOP_PROMPT_LABEL}
      </button>
      {disabledReason && (
        <span
          role="note"
          title={disabledReason}
          className="min-w-0 max-w-[22rem] truncate text-[11px] text-amber-700 dark:text-amber-300"
        >
          {disabledReason}
        </span>
      )}
      {open && (
        <LoopPromptDialog
          sessionId={sessionId}
          paneId={paneId}
          tabLabel={tabLabel}
          hostname={hostname}
          disabledReason={disabledReason}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}
