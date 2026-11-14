"use client";

import { useId, useState } from "react";
import type { AutopilotSpec } from "@apas/protocol";
import {
  autopilotLimitsStatement,
  autopilotStartSpec,
  autopilotStatement,
  AUTOPILOT_NO_LIMIT_TEXT,
  type AutopilotDurationUnit,
  type AutopilotStartForm,
} from "@/lib/autopilot";
import {
  autopilotTemplateById,
  autopilotTemplateEndingNote,
  autopilotTemplateForPrompt,
  autopilotTemplateStartForm,
  AUTOPILOT_OWN_TEXT,
  AUTOPILOT_OWN_TEXT_DESCRIPTION,
  AUTOPILOT_OWN_TEXT_KEPT_TEXT,
  AUTOPILOT_OWN_TEXT_LABEL,
  AUTOPILOT_TEMPLATES,
} from "@/lib/autopilotTemplates";

const FIELD_BASE =
  "rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-zinc-100 focus:outline-none focus:ring-2 focus:ring-indigo-500 disabled:opacity-50";

/**
 * The start dialog for one tab's autopilot. It states what the mode is and is
 * not before anything is sent. It proposes the values of the last autopilot
 * the tab reports; a tab with none is offered the default prompt template,
 * shown in full in the prompt field, where it can be changed. No limit is the
 * proposal for both limits, and the dialog says what that means. Nothing is
 * sent until Start autopilot is pressed, and only once.
 */
export function AutopilotStartDialog({
  tabLabel,
  hostname,
  sharedCheckout,
  last,
  onCancel,
  onStart,
  variant = "dialog",
}: {
  tabLabel: string;
  /** The machine the tab keeps running on. */
  hostname?: string;
  /** The tab works in the project's shared checkout, not a worktree of its own. */
  sharedCheckout: boolean;
  /** The tab's last ended autopilot: its values are offered, ready to change. */
  last?: AutopilotSpec;
  onCancel: () => void;
  /** Send the start. A failure keeps the dialog open with the reason. */
  onStart: (spec: AutopilotSpec) => { success: boolean; error?: string };
  /** "sheet" is the mobile-browser screen's bottom sheet: the same choices
   *  and statement, laid out for a phone. */
  variant?: "dialog" | "sheet";
}) {
  const sheet = variant === "sheet";
  // iOS zooms the page when a focused field's text is smaller than 16px.
  const FIELD = `${FIELD_BASE} ${sheet ? "text-[16px]" : "text-sm"}`;
  const [form, setForm] = useState<AutopilotStartForm>(() => autopilotTemplateStartForm(last));
  // The Template control is not state of its own: it names the template the
  // prompt field holds unchanged, else the person's own text. So it cannot
  // disagree with what will be sent.
  const template = autopilotTemplateForPrompt(form.prompt);
  // What the person last wrote or changed in the field, which is also the
  // last autopilot's prompt until then. Choosing a template leaves it alone,
  // so the own-text entry brings it back. It never holds a template's exact
  // text: the own-text entry would then put that template back and seem to
  // do nothing.
  const [ownText, setOwnText] = useState(() => (template ? "" : form.prompt));
  // Nothing of the person's is set aside while the field shows it.
  const ownTextSetAside = template !== undefined && ownText.trim() !== "";
  // The no-tool-call ending is proposed on only for "as soon as each turn
  // ends". The proposal follows the timing until the person decides.
  const [toolCallDecided, setToolCallDecided] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const promptId = useId();
  const templateId = useId();
  const templateNoteId = useId();
  const update = (patch: Partial<AutopilotStartForm>) => {
    setError(null);
    setForm((current) => ({ ...current, ...patch }));
  };
  const chooseTemplate = (value: string) => {
    // The prompt only: timing, limits and the no-tool-call option stay as
    // the person left them, and the note under the control says what the
    // template's ending needs.
    update({ prompt: autopilotTemplateById(value)?.prompt ?? ownText });
  };
  const chooseRepeat = (repeat: AutopilotStartForm["repeat"]) => update({
    repeat,
    ...(toolCallDecided ? {} : { endWithoutToolCall: repeat === "after_each_turn" }),
  });

  const submit = () => {
    const checked = autopilotStartSpec(form);
    if (checked.error !== undefined) {
      setError(checked.error);
      return;
    }
    const sent = onStart(checked.spec);
    if (!sent.success) setError(sent.error ?? "The start could not be sent.");
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
        // The dialog itself never scrolls: its body does, and the footer with
        // Cancel and Start autopilot stays in view below it. The dialog is
        // taller than a laptop window, and with one scrolling column those two
        // buttons were below the fold of a 1440x900 window.
        className={`flex w-full flex-col overflow-hidden border-zinc-700 bg-zinc-900 text-zinc-100 shadow-xl ${sheet
          ? "max-h-[92dvh] rounded-t-[1.4rem] border-t"
          : "max-h-full max-w-2xl rounded-lg border"}`}
      >
        <div
          data-testid="autopilot-start-body"
          // A scrolling column, so a child that may shrink would collapse:
          // see the prompt field below.
          className={`flex min-h-0 flex-col overflow-y-auto overscroll-contain ${sheet ? "px-4 pt-4" : "px-5 pt-5"}`}
        >
        <h3 id={titleId} className="mb-1 text-lg font-semibold">Start autopilot — {tabLabel}</h3>
        <p className="mb-3 text-xs text-zinc-400">
          The tab submits the prompt again each time an iteration is due and the tab has settled, until a limit you set or until someone stops it.
        </p>

        <ul
          aria-label="What autopilot is and is not"
          className="mb-4 list-disc space-y-1 rounded border border-amber-700/70 bg-amber-950/30 py-2 pl-7 pr-3 text-xs text-amber-100"
        >
          {autopilotStatement({ hostname, sharedCheckout }).map((line) => <li key={line}>{line}</li>)}
        </ul>

        {/* Beside its label in the dialog, which is already about as tall as
            a laptop window; under it on a phone, where a 16px control needs
            the sheet's whole width for the longest name. */}
        <div className={`mb-1 flex shrink-0 ${sheet ? "flex-col gap-1" : "items-center gap-2"}`}>
          <label htmlFor={templateId} className="text-xs text-zinc-300">Template</label>
          <select
            id={templateId}
            value={template?.id ?? AUTOPILOT_OWN_TEXT}
            aria-describedby={templateNoteId}
            onChange={(event) => chooseTemplate(event.target.value)}
            className={`${FIELD} min-w-0 ${sheet ? "w-full" : "flex-1"}`}
          >
            {AUTOPILOT_TEMPLATES.map((entry) => <option key={entry.id} value={entry.id}>{entry.name}</option>)}
            <option value={AUTOPILOT_OWN_TEXT}>{AUTOPILOT_OWN_TEXT_LABEL}</option>
          </select>
        </div>
        <div id={templateNoteId} data-testid="autopilot-template-note" className="mb-3 space-y-1 text-xs text-zinc-400">
          {template ? (
            <>
              <p>{template.description} {autopilotTemplateEndingNote(template, form.endWithoutToolCall)}</p>
              {ownTextSetAside && <p className="text-zinc-300">{AUTOPILOT_OWN_TEXT_KEPT_TEXT}</p>}
            </>
          ) : (
            <p>{AUTOPILOT_OWN_TEXT_DESCRIPTION}</p>
          )}
        </div>

        <label htmlFor={promptId} className="mb-1 text-xs text-zinc-300">Prompt</label>
        <textarea
          id={promptId}
          value={form.prompt}
          onChange={(event) => {
            // An edited template is the person's own text from here on. An
            // edit taken back, so that the field holds a template's exact
            // text again, leaves nothing of the person's to keep.
            setOwnText(autopilotTemplateForPrompt(event.target.value) ? "" : event.target.value);
            update({ prompt: event.target.value });
          }}
          rows={5}
          // On a phone the keyboard would cover the statement above, which is
          // to be read before anything is typed.
          autoFocus={!sheet}
          // It is the one child that can shrink, and the dialog's body
          // scrolls: on a phone or a short window it would collapse to a
          // sliver otherwise.
          className={`${FIELD} mb-3 shrink-0 resize-y`}
        />

        <fieldset className="mb-3">
          <legend className="mb-1 text-xs text-zinc-300">Repeat</legend>
          <label className="flex items-center gap-2 py-0.5 text-sm">
            <input
              type="radio"
              name={`${titleId}-repeat`}
              checked={form.repeat === "after_each_turn"}
              onChange={() => chooseRepeat("after_each_turn")}
            />
            As soon as each turn ends
          </label>
          <div className="flex flex-wrap items-center gap-2 py-0.5 text-sm">
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name={`${titleId}-repeat`}
                checked={form.repeat === "every"}
                onChange={() => chooseRepeat("every")}
              />
              Every
            </label>
            <input
              type="number"
              min={1}
              aria-label="Repeat interval"
              value={form.intervalValue}
              disabled={form.repeat !== "every"}
              onChange={(event) => update({ intervalValue: event.target.value })}
              className={`${FIELD} w-24`}
            />
            <select
              aria-label="Repeat interval unit"
              value={form.intervalUnit}
              disabled={form.repeat !== "every"}
              onChange={(event) => update({ intervalUnit: event.target.value as AutopilotStartForm["intervalUnit"] })}
              className={FIELD}
            >
              <option value="minutes">minutes</option>
              <option value="hours">hours</option>
            </select>
            <span className="text-xs text-zinc-400">one minute to 24 hours, from one submission to the next</span>
          </div>
        </fieldset>

        <fieldset className="mb-3">
          <legend className="mb-1 text-xs text-zinc-300">Iteration limit</legend>
          <label className="flex items-center gap-2 py-0.5 text-sm">
            <input
              type="radio"
              name={`${titleId}-iterations`}
              checked={!form.iterationLimited}
              onChange={() => update({ iterationLimited: false })}
            />
            {AUTOPILOT_NO_LIMIT_TEXT}
          </label>
          <div className="flex flex-wrap items-center gap-2 py-0.5 text-sm">
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name={`${titleId}-iterations`}
                checked={form.iterationLimited}
                onChange={() => update({ iterationLimited: true })}
              />
              At most
            </label>
            <input
              type="number"
              min={1}
              aria-label="Iteration limit"
              value={form.iterationLimit}
              disabled={!form.iterationLimited}
              onChange={(event) => update({ iterationLimit: event.target.value })}
              className={`${FIELD} w-28`}
            />
            <span>iterations</span>
          </div>
        </fieldset>

        <fieldset className="mb-3">
          <legend className="mb-1 text-xs text-zinc-300">Time limit</legend>
          <label className="flex items-center gap-2 py-0.5 text-sm">
            <input
              type="radio"
              name={`${titleId}-time`}
              checked={!form.timeLimited}
              onChange={() => update({ timeLimited: false })}
            />
            {AUTOPILOT_NO_LIMIT_TEXT}
          </label>
          <div className="flex flex-wrap items-center gap-2 py-0.5 text-sm">
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name={`${titleId}-time`}
                checked={form.timeLimited}
                onChange={() => update({ timeLimited: true })}
              />
              At most
            </label>
            <input
              type="number"
              min={1}
              aria-label="Time limit"
              value={form.timeLimitValue}
              disabled={!form.timeLimited}
              onChange={(event) => update({ timeLimitValue: event.target.value })}
              className={`${FIELD} w-28`}
            />
            <select
              aria-label="Time limit unit"
              value={form.timeLimitUnit}
              disabled={!form.timeLimited}
              onChange={(event) => update({ timeLimitUnit: event.target.value as AutopilotDurationUnit })}
              className={FIELD}
            >
              <option value="minutes">minutes</option>
              <option value="hours">hours</option>
              <option value="days">days</option>
            </select>
            <span className="text-xs text-zinc-400">from the start; paused and waiting time counts</span>
          </div>
        </fieldset>

        <label className="mb-3 flex items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-1"
            checked={form.endWithoutToolCall}
            onChange={(event) => {
              setToolCallDecided(true);
              update({ endWithoutToolCall: event.target.checked });
            }}
          />
          <span>
            End when an iteration makes no tool call
            <span className="block text-xs text-zinc-400">
              A rule of thumb for &ldquo;nothing left to do&rdquo;: it can end autopilot early, or never.
            </span>
          </span>
        </label>

        <div data-testid="autopilot-limits-statement" className="mb-3 shrink-0 space-y-1 text-xs text-zinc-300">
          {autopilotLimitsStatement(form).map((line) => <p key={line}>{line}</p>)}
        </div>
        </div>

        {/* Outside the scrolling body, so it is on screen wherever the body
            is scrolled to. A refusal stands here too: it answers the button
            beside it and must not be scrolled out of sight. */}
        <div
          data-testid="autopilot-start-footer"
          className={`shrink-0 border-t border-zinc-700 ${sheet ? "px-4 pt-3 pb-[max(1rem,env(safe-area-inset-bottom))]" : "px-5 py-3"}`}
        >
        {error && (
          <p role="alert" className="mb-3 rounded border border-red-800 bg-red-950/40 px-2 py-1.5 text-xs text-red-200">
            {error}
          </p>
        )}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            className="rounded bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-500"
          >
            Start autopilot
          </button>
        </div>
        </div>
      </div>
    </div>
  );
}
