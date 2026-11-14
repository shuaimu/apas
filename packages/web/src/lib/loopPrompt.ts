import { AUTOPILOT_PROMPT_MAX_BYTES } from "@/lib/autopilot";
import type { AutopilotTemplate } from "@/lib/autopilotTemplates";
import { isRetiredProviderModel } from "@/lib/providerOptions";

/**
 * The loop prompt: a shortcut for Codex, Claude (and DeepSeek, which is the
 * Claude terminal) and OpenCode terminal tabs, where APAS cannot run
 * autopilot. It sends one ordinary message, the loop instruction followed by
 * the person's prompt, and the agent is asked to keep repeating that prompt's
 * instructions by itself.
 *
 * APAS does nothing after the send. Autopilot on an OMP tab runs inside the
 * tab's own process and sees every turn; here there is no such code in the
 * tab, so nothing counts rounds, keeps a limit, pauses or stops the loop, or
 * knows whether the agent is still looping. The dialog says so before
 * anything is sent, and the message creates no autopilot state at all.
 *
 * How the instruction is written:
 * - The built-in templates were written for autopilot, which submits the same
 *   prompt again for every step, so they say to do one step and stop. The
 *   instruction reads that as the end of a round, not of the loop.
 * - The templates end a run with the sentence "Nothing is left to do."; the
 *   instruction stops the loop on that same sentence, word for word.
 * - The agent's memory of earlier rounds is its conversation, which can be
 *   compacted, so it is told to read the files again every round.
 * - It is sent as plain text: no slash command, no em dash or arrow.
 */

/** The sentence the templates end a run with, and the loop's own end. */
export const LOOP_PROMPT_END_SENTENCE = "Nothing is left to do.";

/** The text before the round instructions, as the dialog shows it. */
export const LOOP_PROMPT_INSTRUCTION = [
  "Work in a loop on your own until I interrupt you.",
  "In each round, carry out the instructions after the line \"Round instructions:\" from the start, then begin the next round at once, without waiting for me and without asking whether to continue.",
  "Where those instructions say to stop after one step because the same prompt is sent again, that means: end this round and start the next one yourself.",
  "At the start of every round, read the files again instead of relying on what you remember.",
  `Stop the loop only when a round ends with the sentence "${LOOP_PROMPT_END_SENTENCE}", or when I interrupt you.`,
].join(" ");

const ROUND_INSTRUCTIONS = "Round instructions:";

/** The one message a loop prompt sends: the loop instruction, then the
 *  prompt. The prompt is trimmed, as the send path trims what it sends, so
 *  this is exactly the text the tab receives. */
export function loopPromptMessage(prompt: string): string {
  return `${LOOP_PROMPT_INSTRUCTION}\n\n${ROUND_INSTRUCTIONS}\n\n${prompt.trim()}`;
}

/** The loop instruction as the dialog shows it, read-only: everything the
 *  message holds before the prompt. */
export const LOOP_PROMPT_SHOWN_INSTRUCTION = `${LOOP_PROMPT_INSTRUCTION}\n\n${ROUND_INSTRUCTIONS}`;

/** Why the prompt cannot be sent, or undefined. The whole message keeps
 *  within an autopilot start's bounds, so a prompt that fits one fits the
 *  other less the instruction. */
export function loopPromptError(prompt: string): string | undefined {
  if (!prompt.trim()) return "The loop prompt needs a prompt.";
  const bytes = new TextEncoder().encode(loopPromptMessage(prompt)).length;
  if (bytes > AUTOPILOT_PROMPT_MAX_BYTES) {
    return `With the loop instruction, the message is longer than ${AUTOPILOT_PROMPT_MAX_BYTES} bytes. Shorten the prompt.`;
  }
  return undefined;
}

/** Providers whose terminal tabs offer the loop prompt. DeepSeek runs the
 *  Claude terminal. OMP has autopilot instead. */
const LOOP_PROMPT_PROVIDERS: ReadonlySet<string> = new Set(["claude", "deepseek", "codex", "opencode"]);

/** The tab offers the loop prompt: an interactive terminal tab of one of
 *  those providers. Never a structured pane, a legacy bot's read-only
 *  history or a retired provider's tab. */
export function paneCanHaveLoopPrompt(
  pane: { kind?: string | null; provider?: string | null; model?: string | null; mode?: string | null } | null | undefined,
): boolean {
  if (!pane || pane.kind !== "terminal" || pane.mode === "deadloop") return false;
  if (!pane.provider || !LOOP_PROMPT_PROVIDERS.has(pane.provider)) return false;
  return !isRetiredProviderModel(pane.provider, pane.model);
}

export const LOOP_PROMPT_LABEL = "Loop prompt";
export const LOOP_PROMPT_SEND_LABEL = "Send loop prompt";
/** The phone's pane actions entry. */
export const LOOP_PROMPT_ACTION_LABEL = "Send a loop prompt";
export const LOOP_PROMPT_OFFLINE_TEXT = "Reconnect to send a loop prompt.";

/** What the dialog says first, before anything can be sent. */
export function loopPromptWarning(hostname?: string): string[] {
  const host = hostname ?? "its machine";
  return [
    "This is a shortcut, not autopilot. It sends one prompt that asks the agent to keep working in a loop by itself.",
    "APAS does not run or watch that loop. It cannot count rounds, enforce a limit, pause, resume or stop it, or tell whether the agent is still looping.",
    `The agent keeps this tab's permissions on ${host}, including tools that run without asking.`,
    `Until it is stopped, it spends the agent's login on ${host}, whoever sent the prompt. APAS sets no spending limit.`,
    "To stop it, interrupt it in the terminal (Esc).",
  ];
}

/** The note under the Template control for a built-in template. The
 *  descriptions were written for autopilot's iterations; here one is a round
 *  of the loop. A template with no ending of its own is normally paired with
 *  "every N minutes" under autopilot; the loop has no timing, so the note says
 *  outright that rounds follow back to back even when nothing needs fixing. */
export function loopPromptTemplateNote(template: AutopilotTemplate): string {
  const description = template.description.replace(/\biteration\b/g, "round").replace(/\bIteration\b/g, "Round");
  const ending = template.endsWithoutToolCall
    ? `It ends the loop by itself with "${LOOP_PROMPT_END_SENTENCE}" once nothing is left.`
    : "It has no end of its own: each round starts again at once, also when the last one found nothing to fix, until you interrupt it.";
  return `${description} ${ending}`;
}

/** The note for a saved prompt or the person's own text. */
export const LOOP_PROMPT_OWN_TEXT_NOTE = "The prompt below is sent once, after the loop instruction.";

/** Where the send stands, from the tab's pending terminal deliveries. */
export type LoopPromptDelivery =
  | { kind: "waiting" }
  | { kind: "sent" }
  | { kind: "not_confirmed" }
  | { kind: "refused"; reason: string };

export function loopPromptDeliveryText(delivery: LoopPromptDelivery): string {
  switch (delivery.kind) {
    case "waiting": return "Sent to terminal. Waiting for the agent to record it.";
    case "sent": return "Sent. The agent recorded the loop prompt.";
    case "not_confirmed":
      return "Not confirmed. The agent has not recorded the loop prompt: a terminal dialog may have consumed it, or recording may be delayed. It was not sent again. Check the terminal before you send another.";
    case "refused": return `Not delivered: ${delivery.reason}`;
  }
}
