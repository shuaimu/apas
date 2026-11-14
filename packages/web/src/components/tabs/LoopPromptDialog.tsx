"use client";

import { useEffect, useId, useRef, useState } from "react";
import { useStore } from "@/lib/store";
import { AUTOPILOT_OWN_TEXT_KEPT_TEXT } from "@/lib/autopilotTemplates";
import { unconfirmedDeliveries } from "@/lib/terminalDelivery";
import {
  loopPromptDeliveryText,
  loopPromptError,
  loopPromptMessage,
  loopPromptTemplateNote,
  loopPromptWarning,
  LOOP_PROMPT_OWN_TEXT_NOTE,
  LOOP_PROMPT_SEND_LABEL,
  LOOP_PROMPT_SHOWN_INSTRUCTION,
  type LoopPromptDelivery,
} from "@/lib/loopPrompt";
import { PromptTemplateSelect, PROMPT_FIELD_BASE, usePromptChoice } from "./PromptTemplateChoice";

/**
 * The loop prompt dialog for one Codex, Claude or OpenCode terminal tab. It
 * says first that this is a shortcut APAS does not watch, then offers the
 * autopilot dialog's templates and the project's saved prompts by the same
 * rules, and shows the loop instruction that goes before the prompt.
 *
 * Send loop prompt sends one ordinary message through the terminal
 * conversation input, the path the Conversation view's composer takes, and
 * only once: the dialog then stays open to say whether the agent recorded
 * it, and never offers to send it again. Saving, replacing or deleting a
 * project's saved prompt is left to the autopilot dialog.
 */
export function LoopPromptDialog({
  sessionId,
  paneId,
  tabLabel,
  hostname,
  disabledReason,
  onClose,
  variant = "dialog",
}: {
  /** The tab's session, for the project's saved prompts. */
  sessionId?: string;
  paneId: number;
  tabLabel: string;
  /** The machine the tab runs on. */
  hostname?: string;
  /** Why the tab cannot take a message now. Nothing can be sent while set. */
  disabledReason?: string;
  onClose: () => void;
  /** "sheet" is the mobile-browser screen's bottom sheet. */
  variant?: "dialog" | "sheet";
}) {
  const sheet = variant === "sheet";
  // iOS zooms the page when a focused field's text is smaller than 16px.
  const FIELD = `${PROMPT_FIELD_BASE} ${sheet ? "text-[16px]" : "text-sm"}`;
  const sendTerminalConversationMessage = useStore((state) => state.sendTerminalConversationMessage);
  const choice = usePromptChoice({ sessionId });
  const { template, selectedSaved, ownTextSetAside } = choice;

  // The one send this dialog makes. The ref is set before the store is
  // called, so a second click that lands before React renders again finds
  // it set and sends nothing.
  const sending = useRef(false);
  const [sendId, setSendId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const delivery = useStore((state) => (sendId
    ? state.pendingTerminalDeliveries.find((entry) => entry.id === sendId)
    : undefined));
  const awaiting = Boolean(delivery && !delivery.error);
  const [now, setNow] = useState(() => Date.now());
  // Nothing else renders again when the grace period simply elapses.
  useEffect(() => {
    if (!awaiting) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [awaiting]);
  // The store drops a delivery once the transcript records it: gone is sent.
  const outcome: LoopPromptDelivery | null = !sendId
    ? null
    : !delivery
      ? { kind: "sent" }
      : delivery.error
        ? { kind: "refused", reason: delivery.error }
        : unconfirmedDeliveries([delivery], now).length > 0
          ? { kind: "not_confirmed" }
          : { kind: "waiting" };
  const sent = sendId !== null;

  const titleId = useId();
  const warningId = useId();
  const promptId = useId();
  const templateId = useId();
  const templateNoteId = useId();
  const instructionId = useId();

  const send = () => {
    if (sending.current || disabledReason) return;
    const refusal = loopPromptError(choice.prompt);
    if (refusal) {
      setError(refusal);
      return;
    }
    sending.current = true;
    const result = sendTerminalConversationMessage(paneId, loopPromptMessage(choice.prompt));
    if (!result.success || !result.sendId) {
      // Refused before anything left the page: it may be tried again.
      sending.current = false;
      setError(result.error ?? "The loop prompt could not be sent.");
      return;
    }
    setError(null);
    setSendId(result.sendId);
  };

  return (
    // Closed only by its buttons: a click that lands on the backdrop, as when
    // a selection in the prompt is dragged past the edge, must not discard
    // what was typed.
    <div className={sheet
      ? "fixed inset-0 z-[96] flex items-end bg-black/45"
      : "fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-3 backdrop-blur-sm"}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={warningId}
        // As in the autopilot dialog: the body scrolls, and the footer with
        // Cancel and Send loop prompt stays in view below it.
        className={`flex w-full flex-col overflow-hidden border-zinc-700 bg-zinc-900 text-zinc-100 shadow-xl ${sheet
          ? "max-h-[92dvh] rounded-t-[1.4rem] border-t"
          : "max-h-full max-w-2xl rounded-lg border"}`}
      >
        <div
          data-testid="loop-prompt-body"
          className={`flex min-h-0 flex-col overflow-y-auto overscroll-contain ${sheet ? "px-4 pt-4" : "px-5 pt-5"}`}
        >
          <h3 id={titleId} className="mb-3 text-lg font-semibold">Loop prompt for {tabLabel}</h3>

          <ul
            id={warningId}
            aria-label="What a loop prompt is and is not"
            data-testid="loop-prompt-warning"
            className="mb-4 shrink-0 list-disc space-y-1 rounded border border-amber-600 bg-amber-950/50 py-2 pl-7 pr-3 text-sm text-amber-100"
          >
            {loopPromptWarning(hostname).map((line, index) => (
              <li key={line} className={index === 0 ? "font-semibold" : undefined}>{line}</li>
            ))}
          </ul>

          <div className={`mb-1 flex shrink-0 ${sheet ? "flex-col gap-1" : "items-center gap-2"}`}>
            <label htmlFor={templateId} className="text-xs text-zinc-300">Template</label>
            <PromptTemplateSelect
              choice={{ ...choice, choose: (value) => {
                setError(null);
                choice.choose(value);
              } }}
              id={templateId}
              describedBy={templateNoteId}
              disabled={sent}
              // On a phone the keyboard would cover the statement above.
              autoFocus={!sheet}
              className={`${FIELD} min-w-0 ${sheet ? "w-full" : "flex-1"}`}
            />
          </div>
          <div id={templateNoteId} data-testid="loop-prompt-template-note" className="mb-2 space-y-1 text-xs text-zinc-400">
            {template ? (
              <p>{loopPromptTemplateNote(template)}</p>
            ) : selectedSaved ? (
              <p>Saved to this project. {LOOP_PROMPT_OWN_TEXT_NOTE}</p>
            ) : (
              <p>{LOOP_PROMPT_OWN_TEXT_NOTE}</p>
            )}
            {ownTextSetAside && !sent && <p className="text-zinc-300">{AUTOPILOT_OWN_TEXT_KEPT_TEXT}</p>}
          </div>

          <label htmlFor={promptId} className="mb-1 text-xs text-zinc-300">Prompt</label>
          <textarea
            id={promptId}
            value={choice.prompt}
            readOnly={sent}
            onChange={(event) => {
              if (sent) return;
              setError(null);
              choice.edit(event.target.value);
            }}
            rows={5}
            // The one child that can shrink in a scrolling column: it would
            // collapse to a sliver on a phone otherwise.
            className={`${FIELD} mb-3 shrink-0 resize-y`}
          />

          <p id={instructionId} className="mb-1 text-xs text-zinc-300">APAS puts this before your prompt:</p>
          <p
            data-testid="loop-prompt-instruction"
            aria-labelledby={instructionId}
            className="mb-4 shrink-0 whitespace-pre-wrap break-words rounded border border-zinc-700 bg-zinc-800/60 px-2 py-1.5 text-xs text-zinc-300"
          >
            {LOOP_PROMPT_SHOWN_INSTRUCTION}
          </p>
        </div>

        <div
          data-testid="loop-prompt-footer"
          className={`shrink-0 border-t border-zinc-700 ${sheet ? "px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))]" : "px-5 py-3"}`}
        >
          {error && (
            <p role="alert" className="mb-3 rounded border border-red-800 bg-red-950/40 px-2 py-1.5 text-xs text-red-200">
              {error}
            </p>
          )}
          {!sent && disabledReason && (
            <p role="note" className="mb-3 text-xs text-amber-300">{disabledReason}</p>
          )}
          {outcome && (
            <p
              role="status"
              data-testid="loop-prompt-outcome"
              data-outcome={outcome.kind}
              className={`mb-3 rounded border px-2 py-1.5 text-xs ${outcome.kind === "sent"
                ? "border-emerald-800 bg-emerald-950/40 text-emerald-200"
                : outcome.kind === "waiting"
                  ? "border-zinc-700 text-zinc-300"
                  : "border-red-800 bg-red-950/40 text-red-200"}`}
            >
              {loopPromptDeliveryText(outcome)}
            </p>
          )}

          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700"
            >
              {sent ? "Close" : "Cancel"}
            </button>
            {/* Never offered again once sent: a second copy would be a
                second loop. */}
            {!sent && (
              <button
                type="button"
                onClick={send}
                disabled={Boolean(disabledReason)}
                title={disabledReason}
                className="rounded bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-500 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {LOOP_PROMPT_SEND_LABEL}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
