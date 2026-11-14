"use client";

import { useEffect, useRef, useState, type Ref } from "react";
import type { SavedAutopilotPrompt } from "@apas/protocol";
import { useStore } from "@/lib/store";
import {
  autopilotTemplateById,
  AUTOPILOT_OWN_TEXT,
  AUTOPILOT_OWN_TEXT_LABEL,
  AUTOPILOT_TEMPLATES,
  DEFAULT_AUTOPILOT_TEMPLATE,
  type AutopilotTemplate,
} from "@/lib/autopilotTemplates";
import {
  defaultSavedAutopilotPrompt,
  foldAutopilotPromptName,
  NO_PROJECT_AUTOPILOT_PROMPTS,
  PROJECT_AUTOPILOT_PROMPTS_CAPABILITY,
  type ProjectAutopilotPrompts,
} from "@/lib/autopilotPrompts";

/**
 * The prompt a dialog sends, chosen from the built-in templates and the
 * project's saved prompts, or written by the person. Shared by the autopilot
 * start dialog and the loop prompt dialog, so both offer the same entries and
 * keep the person's own text by the same rules.
 */

/** An entry of the Template control: a prompt saved to the project, or a
 *  built-in template. */
export type PromptEntry =
  | { key: string; kind: "saved"; saved: SavedAutopilotPrompt; prompt: string }
  | { key: string; kind: "template"; template: AutopilotTemplate; prompt: string };

const SAVED_KEY_PREFIX = "saved:";

export function savedEntryKey(name: string): string {
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

export const PROMPT_FIELD_BASE =
  "rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-zinc-100 focus:outline-none focus:ring-2 focus:ring-indigo-500 disabled:opacity-50";

export interface PromptChoice {
  /** What the prompt field holds, which is what is sent. */
  prompt: string;
  /** Choose an entry of the Template control by its value. */
  choose: (value: string) => void;
  /** The person typed in the prompt field. */
  edit: (value: string) => void;
  /** The saved prompt the next save is named after: the one chosen. */
  setChosenKey: (key: string | undefined) => void;
  chosenKey: string | undefined;
  /** The server carries the project's saved prompts. */
  promptsAvailable: boolean;
  list: ProjectAutopilotPrompts;
  savedPrompts: SavedAutopilotPrompt[];
  projectDefault: SavedAutopilotPrompt | undefined;
  /** The entry the field holds unchanged, else undefined: the person's own text. */
  current: PromptEntry | undefined;
  template: AutopilotTemplate | undefined;
  selectedSaved: SavedAutopilotPrompt | undefined;
  /** The saved prompt the field's text came from, also once it is edited. */
  originSaved: SavedAutopilotPrompt | undefined;
  /** A template has replaced text the person wrote, which is kept. */
  ownTextSetAside: boolean;
}

/**
 * The prompt and its Template control. With `initialPrompt` (an earlier
 * autopilot's), the dialog opens on it. Without, it opens on the project's
 * default saved prompt when there is one, else the default template, and a
 * project list that arrives later still gets its default in, unless the
 * person has already chosen or written something.
 */
export function usePromptChoice({ sessionId, initialPrompt }: {
  /** The tab's session. Without it only the built-in templates are offered. */
  sessionId?: string;
  initialPrompt?: string;
}): PromptChoice {
  const hasInitial = initialPrompt !== undefined;
  // The project's saved prompts, only where the server can carry them. A
  // viewer whose server cannot is offered exactly the built-in templates.
  const promptsAvailable = useStore((state) =>
    sessionId !== undefined && state.negotiatedCapabilities.has(PROJECT_AUTOPILOT_PROMPTS_CAPABILITY));
  const reportedList = useStore((state) => (sessionId ? state.projectAutopilotPrompts[sessionId] : undefined));
  const list = promptsAvailable ? reportedList ?? NO_PROJECT_AUTOPILOT_PROMPTS : NO_PROJECT_AUTOPILOT_PROMPTS;
  const savedPrompts = list.received && !list.unreadable ? list.prompts : [];
  const projectDefault = defaultSavedAutopilotPrompt(list);
  const entries: PromptEntry[] = [
    ...savedPrompts.map((saved): PromptEntry => ({ key: savedEntryKey(saved.name), kind: "saved", saved, prompt: saved.prompt })),
    ...TEMPLATE_ENTRIES,
  ];

  const [prompt, setPrompt] = useState<string>(() =>
    initialPrompt ?? projectDefault?.prompt ?? DEFAULT_AUTOPILOT_TEMPLATE.prompt);
  // The entry the person last chose, or the one the dialog opened on. It
  // only breaks ties between entries with the same text, and says which
  // saved prompt an edited text came from.
  const [chosenKey, setChosenKey] = useState<string | undefined>(() =>
    !hasInitial && projectDefault ? savedEntryKey(projectDefault.name) : undefined);
  // The Template control is not state of its own: it names the entry the
  // prompt field holds unchanged, else the person's own text. So it cannot
  // disagree with what will be sent.
  const current = entryForPrompt(entries, prompt, chosenKey);
  const template = current?.kind === "template" ? current.template : undefined;
  // What the person last wrote or changed in the field, which is also the
  // last autopilot's prompt until then. Choosing an entry leaves it alone,
  // so the own-text entry brings it back. It never holds an entry's exact
  // text: the own-text entry would then put that entry back and seem to
  // do nothing.
  const [ownText, setOwnText] = useState(() => (current ? "" : prompt));
  // Nothing of the person's is set aside while the field shows it.
  const ownTextSetAside = current !== undefined && ownText.trim() !== "" && ownText !== prompt;

  // The project's list can arrive after the dialog opened, as right after a
  // reconnect. A dialog with no earlier prompt then gets the project's
  // default, unless the person has already chosen or written something.
  const touched = useRef(false);
  useEffect(() => {
    if (hasInitial || touched.current || !projectDefault) return;
    setPrompt((value) => value === DEFAULT_AUTOPILOT_TEMPLATE.prompt ? projectDefault.prompt : value);
    setChosenKey(savedEntryKey(projectDefault.name));
  }, [hasInitial, projectDefault]);

  const selectedSaved = current?.kind === "saved" ? current.saved : undefined;
  const originSaved = selectedSaved
    ?? (chosenKey ? savedPrompts.find((saved) => savedEntryKey(saved.name) === chosenKey) : undefined);

  const choose = (value: string) => {
    touched.current = true;
    const entry = entries.find((candidate) => candidate.key === value);
    // Text that is no entry's is the person's own, also when the saved
    // prompt it was has since been deleted: keep it for the own-text entry.
    if (!current && entry) setOwnText(prompt);
    // The own-text entry is no saved prompt: nothing chosen is then a saved
    // prompt to pre-fill a save with, or to replace by accident. An edit of
    // a saved prompt's text keeps naming it, since that is how one is changed.
    setChosenKey(entry?.key);
    setPrompt(entry?.prompt ?? autopilotTemplateById(value)?.prompt ?? (current ? ownText : prompt));
  };
  const edit = (value: string) => {
    // An edited template is the person's own text from here on. An edit
    // taken back, so that the field holds a template's exact text again,
    // leaves nothing of the person's to keep.
    touched.current = true;
    setOwnText(entryForPrompt(entries, value) ? "" : value);
    setPrompt(value);
  };

  return {
    prompt,
    choose,
    edit,
    setChosenKey,
    chosenKey,
    promptsAvailable,
    list,
    savedPrompts,
    projectDefault,
    current,
    template,
    selectedSaved,
    originSaved,
    ownTextSetAside,
  };
}

/** The Template control: the project's saved prompts, when the server
 *  carries them, then the built-in templates, then the person's own text. */
export function PromptTemplateSelect({
  choice,
  id,
  describedBy,
  className,
  selectRef,
  disabled,
  autoFocus,
}: {
  choice: PromptChoice;
  id: string;
  describedBy?: string;
  className: string;
  selectRef?: Ref<HTMLSelectElement>;
  disabled?: boolean;
  autoFocus?: boolean;
}) {
  const { promptsAvailable, savedPrompts, projectDefault } = choice;
  return (
    <select
      ref={selectRef}
      id={id}
      value={choice.current?.key ?? AUTOPILOT_OWN_TEXT}
      aria-describedby={describedBy}
      disabled={disabled}
      autoFocus={autoFocus}
      onChange={(event) => choice.choose(event.target.value)}
      className={className}
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
  );
}
