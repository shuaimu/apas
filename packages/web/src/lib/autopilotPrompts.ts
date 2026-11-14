import type { AutopilotPromptChange, SavedAutopilotPrompt } from "@apas/protocol";
import { autopilotStartSpec, autopilotStartForm } from "./autopilot";
import { AUTOPILOT_TEMPLATES } from "./autopilotTemplates";

/**
 * A project's saved autopilot prompts, as the start dialog offers them.
 *
 * They are kept in the project's `.apas` file, which only the project's APAS
 * on its machine writes. The page asks for a change with one
 * `update_autopilot_prompts`, learns what became of it from one
 * `autopilot_prompts_receipt`, and shows the list the project's primary
 * instance last reported in `project_autopilot_prompts`. Saving a prompt
 * sends nothing to any tab and never starts, pauses or stops an autopilot.
 * The Rust statement of the contract is `crates/shared/src/autopilot_prompts.rs`.
 */

/** Mirrors `shared::PROJECT_AUTOPILOT_PROMPTS_CAPABILITY`. Advertised in
 *  `authenticate`; only when the server echoes it does the page send a
 *  change or show the project's prompts. */
export const PROJECT_AUTOPILOT_PROMPTS_CAPABILITY = "project_autopilot_prompts_v1";

/** Mirrors `shared::AUTOPILOT_PROMPT_NAME_MAX_CHARS`. */
export const AUTOPILOT_PROMPT_NAME_MAX_CHARS = 80;
/** Mirrors `shared::PROJECT_AUTOPILOT_PROMPTS_MAX`. */
export const PROJECT_AUTOPILOT_PROMPTS_MAX = 50;

/** What the dialog says when the project's `.apas` holds a list that does
 *  not parse. Nothing can be saved until someone fixes the file. */
export const AUTOPILOT_PROMPTS_UNREADABLE_TEXT = "This project's saved prompts cannot be read.";

/** One project's saved prompts, by session, as its primary instance last
 *  reported them. `received` is false until a list has arrived: an empty
 *  list is a project with none, not one that has not reported. */
export interface ProjectAutopilotPrompts {
  prompts: SavedAutopilotPrompt[];
  default?: string;
  unreadable: boolean;
  received: boolean;
}

export const NO_PROJECT_AUTOPILOT_PROMPTS: ProjectAutopilotPrompts = Object.freeze({
  prompts: [],
  unreadable: false,
  received: false,
}) as ProjectAutopilotPrompts;

/** One change asked for, until its receipt. The receipt names no change, so
 *  the request keeps it, to say what was done. */
export interface AutopilotPromptsRequest {
  requestId: string;
  sessionId: string;
  change: AutopilotPromptChange;
  status: "pending" | "saved" | "refused" | "unconfirmed";
  /** Why it was refused. */
  reason?: string;
  /** Why it is not confirmed: no receipt in time, the connection closed, or
   *  the server could not learn the machine's answer. */
  cause?: "timeout" | "connection" | "uncertain";
  sentAt: number;
}

/** Names compare trimmed and ignoring case, as `fold_name` does in Rust. */
export function foldAutopilotPromptName(name: string): string {
  return name.trim().toLowerCase();
}

export function findSavedAutopilotPrompt(
  prompts: readonly SavedAutopilotPrompt[],
  name: string,
): SavedAutopilotPrompt | undefined {
  const wanted = foldAutopilotPromptName(name);
  return prompts.find((saved) => foldAutopilotPromptName(saved.name) === wanted);
}

/** The project's default saved prompt, when it has one that is listed. */
export function defaultSavedAutopilotPrompt(list: ProjectAutopilotPrompts | undefined): SavedAutopilotPrompt | undefined {
  if (!list?.received || list.unreadable || !list.default) return undefined;
  return findSavedAutopilotPrompt(list.prompts, list.default);
}

/** Parse a `project_autopilot_prompts` frame. Entries that are not whole
 *  saved prompts are left out rather than shown half. */
export function parseProjectAutopilotPromptsFrame(
  data: Record<string, unknown>,
): { sessionId: string; list: ProjectAutopilotPrompts } | null {
  if (typeof data.session_id !== "string" || !Array.isArray(data.prompts)) return null;
  const prompts: SavedAutopilotPrompt[] = [];
  for (const value of data.prompts as unknown[]) {
    if (!value || typeof value !== "object") continue;
    const saved = value as Record<string, unknown>;
    if (typeof saved.name !== "string" || typeof saved.prompt !== "string" || typeof saved.updated_at !== "string") continue;
    prompts.push({ name: saved.name, prompt: saved.prompt, updated_at: saved.updated_at });
  }
  const unreadable = data.unreadable === true;
  return {
    sessionId: data.session_id,
    list: {
      prompts: unreadable ? [] : prompts,
      ...(!unreadable && typeof data.default === "string" && data.default ? { default: data.default } : {}),
      unreadable,
      received: true,
    },
  };
}

// The Rust texts, word for word (`validate_autopilot_prompt_change`), so a
// refusal reads the same wherever it is caught. The built-in names are the
// templates' own; a Rust test keeps `AUTOPILOT_BUILTIN_TEMPLATE_NAMES` in step.
const BUILTIN_TEMPLATE_NAMES = AUTOPILOT_TEMPLATES.map((template) => template.name);

function hasControlCharacter(text: string): boolean {
  // Rust's `char::is_control`: U+0000..U+001F and U+007F..U+009F.
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return true;
  }
  return false;
}

/**
 * Why a change would be refused against the list this page holds, or
 * undefined. It is the server's and the machine's check, made early so an
 * obvious mistake is not sent; they check again against the file, which is
 * what decides.
 */
export function autopilotPromptChangeError(
  change: AutopilotPromptChange,
  prompts: readonly SavedAutopilotPrompt[],
): string | undefined {
  if (change.kind === "save") {
    const name = change.name.trim();
    if (!name) return "Give the saved prompt a name.";
    if ([...name].length > AUTOPILOT_PROMPT_NAME_MAX_CHARS) {
      return `A saved prompt's name is at most ${AUTOPILOT_PROMPT_NAME_MAX_CHARS} characters.`;
    }
    if (hasControlCharacter(name)) return "A saved prompt's name cannot contain control characters.";
    const builtin = BUILTIN_TEMPLATE_NAMES.find((candidate) =>
      foldAutopilotPromptName(candidate) === foldAutopilotPromptName(name));
    if (builtin) return `"${builtin}" is the name of a built-in template. Choose another name.`;
    const existing = findSavedAutopilotPrompt(prompts, name);
    if (existing && !change.replace) return `A saved prompt named "${existing.name.trim()}" already exists.`;
    if (!existing && prompts.length >= PROJECT_AUTOPILOT_PROMPTS_MAX) {
      return `A project keeps at most ${PROJECT_AUTOPILOT_PROMPTS_MAX} saved prompts. Delete one first.`;
    }
    // The prompt's bounds are a start's.
    return autopilotStartSpec({ ...autopilotStartForm(), prompt: change.prompt }).error;
  }
  if (change.kind === "delete" || (change.kind === "set_default" && change.name != null)) {
    const name = change.name ?? "";
    if (!findSavedAutopilotPrompt(prompts, name)) return `No saved prompt is named "${name.trim()}".`;
  }
  return undefined;
}

/** What became of a request, in one line, for the dialog. */
export function autopilotPromptsRequestText(request: AutopilotPromptsRequest): string {
  if (request.status === "pending") {
    switch (request.change.kind) {
      case "save": return "Saving…";
      case "delete": return "Deleting…";
      default: return "Changing the default…";
    }
  }
  if (request.status === "refused") return request.reason ?? "The change was refused.";
  if (request.status === "unconfirmed") {
    const why = request.cause === "connection"
      ? "The connection closed before the project's machine answered."
      : "The project's machine did not answer in time.";
    return `Not confirmed. ${why} It was not sent again: check the list before you try again.`;
  }
  const change = request.change;
  switch (change.kind) {
    case "save": return `Saved to the project as "${change.name.trim()}".`;
    case "delete": return `Deleted "${change.name.trim()}".`;
    default: return change.name != null
      ? `"${change.name.trim()}" is the project's default.`
      : "The project has no default prompt now.";
  }
}
