"use client";

import { useEffect, useId, useRef, useState } from "react";
import type { AutopilotPromptChange, AutopilotSpec, SavedAutopilotPrompt } from "@apas/protocol";
import { useStore } from "@/lib/store";
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
  autopilotTemplateStartForm,
  AUTOPILOT_OWN_TEXT,
  AUTOPILOT_OWN_TEXT_DESCRIPTION,
  AUTOPILOT_OWN_TEXT_KEPT_TEXT,
  AUTOPILOT_OWN_TEXT_LABEL,
  AUTOPILOT_TEMPLATES,
  DEFAULT_AUTOPILOT_TEMPLATE,
  type AutopilotTemplate,
} from "@/lib/autopilotTemplates";
import {
  autopilotPromptChangeError,
  autopilotPromptsRequestText,
  AUTOPILOT_PROMPT_NAME_MAX_CHARS,
  AUTOPILOT_PROMPTS_UNREADABLE_TEXT,
  defaultSavedAutopilotPrompt,
  findSavedAutopilotPrompt,
  foldAutopilotPromptName,
  NO_PROJECT_AUTOPILOT_PROMPTS,
  PROJECT_AUTOPILOT_PROMPTS_CAPABILITY,
} from "@/lib/autopilotPrompts";

/** An entry of the Template control: a prompt saved to the project, or a
 *  built-in template. */
type PromptEntry =
  | { key: string; kind: "saved"; saved: SavedAutopilotPrompt; prompt: string }
  | { key: string; kind: "template"; template: AutopilotTemplate; prompt: string };

const SAVED_KEY_PREFIX = "saved:";

function savedEntryKey(name: string): string {
  return `${SAVED_KEY_PREFIX}${foldAutopilotPromptName(name)}`;
}

const TEMPLATE_ENTRIES: PromptEntry[] = AUTOPILOT_TEMPLATES.map((template) => ({
  key: template.id,
  kind: "template",
  template,
  prompt: template.prompt,
}));

/**
 * The entry a prompt is, unchanged. The entry the person last chose wins
 * when several hold the same text; otherwise the project's prompts come
 * before the built-in ones, as the control lists them.
 */
function entryForPrompt(entries: readonly PromptEntry[], prompt: string, preferred?: string): PromptEntry | undefined {
  const chosen = preferred ? entries.find((entry) => entry.key === preferred) : undefined;
  if (chosen && chosen.prompt === prompt) return chosen;
  return entries.find((entry) => entry.prompt === prompt);
}

const FIELD_BASE =
  "rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-zinc-100 focus:outline-none focus:ring-2 focus:ring-indigo-500 disabled:opacity-50";

/**
 * The start dialog for one tab's autopilot. It states what the mode is and is
 * not before anything is sent. It proposes the values of the last autopilot
 * the tab reports; a tab with none is offered the project's default saved
 * prompt, else the default prompt template, shown in full in the prompt
 * field, where it can be changed. No limit is the proposal for both limits,
 * and the dialog says what that means. Nothing is sent until Start autopilot
 * is pressed, and only once.
 *
 * With `sessionId`, and a server that negotiated saved prompts, it also lists
 * the project's saved prompts and can save the prompt field to the project,
 * replace or delete a saved prompt, and set or clear the project's default.
 * Those change the project's `.apas` only: they never start or change an
 * autopilot, are each sent once, and leave the dialog open.
 */
export function AutopilotStartDialog({
  sessionId,
  tabLabel,
  hostname,
  sharedCheckout,
  last,
  onCancel,
  onStart,
  variant = "dialog",
}: {
  /** The tab's session. Without it the dialog offers the built-in templates
   *  only. */
  sessionId?: string;
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
  const ACTION = `rounded border border-zinc-600 bg-zinc-800 px-2 text-zinc-100 hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-50 ${sheet ? "py-1.5 text-sm" : "py-0.5 text-xs"}`;

  // The project's saved prompts, only where the server can carry them. A
  // viewer whose server cannot is offered exactly the built-in templates.
  const promptsAvailable = useStore((state) =>
    sessionId !== undefined && state.negotiatedCapabilities.has(PROJECT_AUTOPILOT_PROMPTS_CAPABILITY));
  const reportedList = useStore((state) => (sessionId ? state.projectAutopilotPrompts[sessionId] : undefined));
  const promptRequests = useStore((state) => state.autopilotPromptRequests);
  const updateAutopilotPrompts = useStore((state) => state.updateAutopilotPrompts);
  const list = promptsAvailable ? reportedList ?? NO_PROJECT_AUTOPILOT_PROMPTS : NO_PROJECT_AUTOPILOT_PROMPTS;
  const savedPrompts = list.received && !list.unreadable ? list.prompts : [];
  const projectDefault = defaultSavedAutopilotPrompt(list);
  const entries: PromptEntry[] = [
    ...savedPrompts.map((saved): PromptEntry => ({ key: savedEntryKey(saved.name), kind: "saved", saved, prompt: saved.prompt })),
    ...TEMPLATE_ENTRIES,
  ];

  const [form, setForm] = useState<AutopilotStartForm>(() => autopilotTemplateStartForm(last, projectDefault?.prompt));
  // The entry the person last chose, or the one the dialog opened on. It
  // only breaks ties between entries with the same text, and says which
  // saved prompt an edited text came from.
  const [chosenKey, setChosenKey] = useState<string | undefined>(() =>
    !last && projectDefault ? savedEntryKey(projectDefault.name) : undefined);
  // The Template control is not state of its own: it names the entry the
  // prompt field holds unchanged, else the person's own text. So it cannot
  // disagree with what will be sent.
  const current = entryForPrompt(entries, form.prompt, chosenKey);
  const template = current?.kind === "template" ? current.template : undefined;
  // What the person last wrote or changed in the field, which is also the
  // last autopilot's prompt until then. Choosing an entry leaves it alone,
  // so the own-text entry brings it back. It never holds an entry's exact
  // text: the own-text entry would then put that entry back and seem to
  // do nothing.
  const [ownText, setOwnText] = useState(() => (current ? "" : form.prompt));
  // Nothing of the person's is set aside while the field shows it.
  const ownTextSetAside = current !== undefined && ownText.trim() !== "" && ownText !== form.prompt;

  // The project's list can arrive after the dialog opened, as right after a
  // reconnect. A tab with no earlier autopilot then gets the project's
  // default, unless the person has already chosen or written something.
  const touched = useRef(false);
  useEffect(() => {
    if (last || touched.current || !projectDefault) return;
    setForm((value) => value.prompt === DEFAULT_AUTOPILOT_TEMPLATE.prompt
      ? { ...value, prompt: projectDefault.prompt }
      : value);
    setChosenKey(savedEntryKey(projectDefault.name));
  }, [last, projectDefault]);

  // Saving to the project. The outcome line answers the last change asked
  // for here; a new one replaces it. Nothing is sent again by itself.
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [promptsRequestId, setPromptsRequestId] = useState<string | null>(null);
  const [promptsError, setPromptsError] = useState<string | null>(null);
  // Focus inside this modal dialog. Opening the save form or the delete
  // confirmation moves focus into it; when the element holding focus goes
  // away, as the confirmation does on "Delete it" and the save form on a
  // save, focus returns to the button that opened it, or to the Template
  // control when that button is gone or cannot take it. Left alone it falls
  // to the page behind the dialog.
  const templateRef = useRef<HTMLSelectElement>(null);
  const saveNameRef = useRef<HTMLInputElement>(null);
  const focusReturn = useRef<HTMLElement | null>(null);
  const promptsRequest = promptsRequestId ? promptRequests[promptsRequestId] : undefined;
  const promptsPending = promptsRequest?.status === "pending";
  const saveFormShown = saveOpen
    && !(promptsRequest?.change.kind === "save" && promptsRequest.status === "saved");
  const selectedSaved = current?.kind === "saved" ? current.saved : undefined;
  // The saved prompt the field's text came from, also once it is edited:
  // saving it again under that name is how a saved prompt is changed.
  const originSaved = selectedSaved
    ?? (chosenKey ? savedPrompts.find((saved) => savedEntryKey(saved.name) === chosenKey) : undefined);
  const replacing = findSavedAutopilotPrompt(savedPrompts, saveName);
  const isDefault = selectedSaved !== undefined && projectDefault !== undefined
    && foldAutopilotPromptName(projectDefault.name) === foldAutopilotPromptName(selectedSaved.name);
  // The no-tool-call ending is proposed on only for "as soon as each turn
  // ends". The proposal follows the timing until the person decides.
  const [toolCallDecided, setToolCallDecided] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();
  const promptId = useId();
  const templateId = useId();
  const templateNoteId = useId();
  const saveNameId = useId();
  const update = (patch: Partial<AutopilotStartForm>) => {
    setError(null);
    setForm((current) => ({ ...current, ...patch }));
  };
  const chooseTemplate = (value: string) => {
    touched.current = true;
    setConfirmingDelete(false);
    const entry = entries.find((candidate) => candidate.key === value);
    // Text that is no entry's is the person's own, also when the saved
    // prompt it was has since been deleted: keep it for the own-text entry.
    if (!current && entry) setOwnText(form.prompt);
    // The own-text entry is no saved prompt: nothing chosen is then a saved
    // prompt to pre-fill a save with, or to replace by accident. An edit of
    // a saved prompt's text keeps naming it, since that is how one is changed.
    setChosenKey(entry?.key);
    // The prompt only: timing, limits and the no-tool-call option stay as
    // the person left them, and the note under the control says what the
    // template's ending needs.
    update({ prompt: entry?.prompt ?? autopilotTemplateById(value)?.prompt ?? (current ? ownText : form.prompt) });
  };

  const sendPromptsChange = (change: AutopilotPromptChange) => {
    if (!sessionId) return false;
    setPromptsRequestId(null);
    const refusal = autopilotPromptChangeError(change, savedPrompts);
    if (refusal) {
      setPromptsError(refusal);
      return false;
    }
    const result = updateAutopilotPrompts(sessionId, change);
    if (!result.success || !result.requestId) {
      setPromptsError(result.error ?? "The change could not be sent.");
      return false;
    }
    setPromptsError(null);
    setPromptsRequestId(result.requestId);
    return true;
  };
  const openSave = (opener: HTMLElement) => {
    focusReturn.current = opener;
    setConfirmingDelete(false);
    setPromptsRequestId(null);
    setPromptsError(null);
    setSaveName(originSaved?.name ?? "");
    setSaveOpen(true);
  };
  const save = () => {
    const name = saveName.trim();
    // Replacing is asked for only when the dialog has said so.
    const sent = sendPromptsChange({ kind: "save", name, prompt: form.prompt, replace: replacing !== undefined });
    // Once the list has it, the control names it.
    if (!sent) return;
    setChosenKey(savedEntryKey(name));
    // The Save button is disabled while the change is pending, which would
    // drop its focus: the name field stays, read-only, beside "Saving…".
    saveNameRef.current?.focus();
  };

  // Where an element holding focus was removed or disabled, focus has fallen
  // to the page. Only then: a person who has moved focus somewhere is left
  // there, and opening the dialog on a phone does not focus anything.
  // It acts on a change of what is shown, never on the dialog's own mount
  // (React runs a mount's effect twice in development).
  const focusSeen = useRef<unknown[] | null>(null);
  useEffect(() => {
    const seen = [saveFormShown, confirmingDelete, promptsPending, selectedSaved];
    const before = focusSeen.current;
    focusSeen.current = seen;
    if (!before || before.every((value, index) => value === seen[index])) return;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    const usable = (element: HTMLElement | null) => element !== null && element.isConnected
      && !(element as HTMLButtonElement).disabled;
    const target = [focusReturn.current, templateRef.current].find(usable);
    target?.focus();
  }, [saveFormShown, confirmingDelete, promptsPending, selectedSaved]);
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
            ref={templateRef}
            id={templateId}
            value={current?.key ?? AUTOPILOT_OWN_TEXT}
            aria-describedby={templateNoteId}
            onChange={(event) => chooseTemplate(event.target.value)}
            className={`${FIELD} min-w-0 ${sheet ? "w-full" : "flex-1"}`}
          >
            {promptsAvailable ? (
              <>
                {savedPrompts.length > 0 && (
                  <optgroup label="This project">
                    {savedPrompts.map((saved) => (
                      <option key={savedEntryKey(saved.name)} value={savedEntryKey(saved.name)}>
                        {projectDefault && savedEntryKey(projectDefault.name) === savedEntryKey(saved.name)
                          ? `${saved.name} (default)`
                          : saved.name}
                      </option>
                    ))}
                  </optgroup>
                )}
                <optgroup label="Built in">
                  {AUTOPILOT_TEMPLATES.map((entry) => <option key={entry.id} value={entry.id}>{entry.name}</option>)}
                </optgroup>
              </>
            ) : (
              AUTOPILOT_TEMPLATES.map((entry) => <option key={entry.id} value={entry.id}>{entry.name}</option>)
            )}
            <option value={AUTOPILOT_OWN_TEXT}>{AUTOPILOT_OWN_TEXT_LABEL}</option>
          </select>
        </div>
        <div id={templateNoteId} data-testid="autopilot-template-note" className="mb-2 space-y-1 text-xs text-zinc-400">
          {template ? (
            <p>{template.description} {autopilotTemplateEndingNote(template, form.endWithoutToolCall)}</p>
          ) : selectedSaved ? (
            <p>
              Saved to this project.{isDefault ? " It is the project's default." : ""} The prompt below is submitted unchanged on every iteration.
            </p>
          ) : (
            <p>{AUTOPILOT_OWN_TEXT_DESCRIPTION}</p>
          )}
          {ownTextSetAside && <p className="text-zinc-300">{AUTOPILOT_OWN_TEXT_KEPT_TEXT}</p>}
        </div>

        {promptsAvailable && (
          <div data-testid="autopilot-saved-prompts" className="mb-3 shrink-0 space-y-2 text-xs text-zinc-300">
            {list.unreadable ? (
              <p role="note" className="text-amber-200">{AUTOPILOT_PROMPTS_UNREADABLE_TEXT}</p>
            ) : (
              <>
                <div className="flex flex-wrap items-center gap-2">
                  <button type="button" onClick={(event) => openSave(event.currentTarget)} disabled={promptsPending} aria-expanded={saveFormShown} className={ACTION}>
                    Save to project…
                  </button>
                  {selectedSaved && (
                    <>
                      <button
                        type="button"
                        disabled={promptsPending}
                        onClick={(event) => {
                          focusReturn.current = event.currentTarget;
                          setSaveOpen(false);
                          setConfirmingDelete(false);
                          sendPromptsChange(isDefault
                            ? { kind: "set_default", name: null }
                            : { kind: "set_default", name: selectedSaved.name });
                        }}
                        className={ACTION}
                      >
                        {isDefault ? "Clear default" : "Make default"}
                      </button>
                      <button
                        type="button"
                        disabled={promptsPending}
                        aria-expanded={confirmingDelete}
                        onClick={(event) => {
                          focusReturn.current = event.currentTarget;
                          setSaveOpen(false);
                          setPromptsRequestId(null);
                          setPromptsError(null);
                          setConfirmingDelete(true);
                        }}
                        className={ACTION}
                      >
                        Delete
                      </button>
                    </>
                  )}
                </div>

                {saveFormShown && (
                  <div data-testid="autopilot-save-prompt" className="space-y-1 rounded border border-zinc-700 p-2">
                    <label htmlFor={saveNameId} className="block text-xs text-zinc-300">Name</label>
                    <div className={`flex gap-2 ${sheet ? "flex-col" : "items-center"}`}>
                      <input
                        ref={saveNameRef}
                        id={saveNameId}
                        type="text"
                        value={saveName}
                        // Into the form as it opens, for the name.
                        autoFocus
                        // A pending change keeps its "Saving…" until its
                        // answer: the name it was sent with cannot change
                        // under it, and nothing else can be sent meanwhile.
                        readOnly={promptsPending}
                        // Room past the limit, so an over-long name gets
                        // the refusal that says why instead of being cut.
                        maxLength={AUTOPILOT_PROMPT_NAME_MAX_CHARS * 2}
                        onChange={(event) => {
                          if (promptsPending) return;
                          // A new name makes the last answer stale.
                          setPromptsError(null);
                          setPromptsRequestId(null);
                          setSaveName(event.target.value);
                        }}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" && !promptsPending) {
                            event.preventDefault();
                            save();
                          }
                        }}
                        className={`${FIELD} min-w-0 ${sheet ? "w-full" : "flex-1"}`}
                      />
                      <div className="flex gap-2">
                        <button type="button" disabled={promptsPending} onClick={save} className={ACTION}>
                          {replacing ? "Replace" : "Save"}
                        </button>
                        <button
                          type="button"
                          aria-label="Cancel saving"
                          onClick={() => {
                            setSaveOpen(false);
                            setPromptsError(null);
                          }}
                          className={ACTION}
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                    {replacing && (
                      <p className="text-zinc-400">This replaces the saved prompt &quot;{replacing.name}&quot;.</p>
                    )}
                  </div>
                )}

                {confirmingDelete && selectedSaved && (
                  <div data-testid="autopilot-delete-prompt" className="space-y-1 rounded border border-zinc-700 p-2">
                    <p>
                      Delete the saved prompt &quot;{selectedSaved.name}&quot; from this project? An autopilot that is running keeps its own prompt.
                    </p>
                    <div className="flex gap-2">
                      <button
                        type="button"
                        disabled={promptsPending}
                        onClick={() => {
                          setConfirmingDelete(false);
                          sendPromptsChange({ kind: "delete", name: selectedSaved.name });
                        }}
                        className={`${ACTION} border-red-800 text-red-200`}
                      >
                        Delete it
                      </button>
                      {/* The safe choice takes focus as the question opens. */}
                      <button type="button" autoFocus onClick={() => setConfirmingDelete(false)} className={ACTION}>
                        Keep it
                      </button>
                    </div>
                  </div>
                )}

                {(promptsError || promptsRequest) && (
                  <p
                    role="status"
                    data-testid="autopilot-prompts-outcome"
                    className={promptsError || promptsRequest?.status === "refused" || promptsRequest?.status === "unconfirmed"
                      ? "text-red-300"
                      : "text-zinc-300"}
                  >
                    {promptsError ?? (promptsRequest ? autopilotPromptsRequestText(promptsRequest) : null)}
                  </p>
                )}
              </>
            )}
          </div>
        )}

        <label htmlFor={promptId} className="mb-1 text-xs text-zinc-300">Prompt</label>
        <textarea
          id={promptId}
          value={form.prompt}
          onChange={(event) => {
            // An edited template is the person's own text from here on. An
            // edit taken back, so that the field holds a template's exact
            // text again, leaves nothing of the person's to keep.
            touched.current = true;
            setOwnText(entryForPrompt(entries, event.target.value) ? "" : event.target.value);
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
