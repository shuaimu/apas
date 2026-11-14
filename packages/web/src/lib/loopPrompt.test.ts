import { describe, expect, it } from "vitest";
import { AUTOPILOT_PROMPT_MAX_BYTES } from "./autopilot";
import { AUTOPILOT_TEMPLATES } from "./autopilotTemplates";
import {
  LOOP_PROMPT_END_SENTENCE,
  LOOP_PROMPT_INSTRUCTION,
  LOOP_PROMPT_SHOWN_INSTRUCTION,
  loopPromptError,
  loopPromptMessage,
  loopPromptTemplateNote,
  loopPromptWarning,
  paneCanHaveLoopPrompt,
} from "./loopPrompt";

const bytes = (text: string) => new TextEncoder().encode(text).length;

describe("the loop instruction", () => {
  it("puts the instruction before the prompt, and is exactly what the send path sends", () => {
    const message = loopPromptMessage("  Fix the first failing test.\n");
    expect(message).toBe(`${LOOP_PROMPT_SHOWN_INSTRUCTION}\n\nFix the first failing test.`);
    expect(message.startsWith(`${LOOP_PROMPT_INSTRUCTION}\n\nRound instructions:\n\n`)).toBe(true);
    // The store trims what it sends; the message is already trimmed.
    expect(message.trim()).toBe(message);
  });

  it("says round after round, without waiting or asking", () => {
    expect(LOOP_PROMPT_INSTRUCTION).toContain("Work in a loop on your own until I interrupt you.");
    expect(LOOP_PROMPT_INSTRUCTION).toContain("begin the next round at once, without waiting for me and without asking whether to continue");
  });

  it("reads a template's \"stop after one step\" as the end of a round", () => {
    expect(LOOP_PROMPT_INSTRUCTION).toContain(
      "Where those instructions say to stop after one step because the same prompt is sent again, that means: end this round and start the next one yourself.",
    );
    // The templates do say so.
    for (const template of AUTOPILOT_TEMPLATES) {
      expect(template.prompt).toContain("the same prompt is sent again");
    }
  });

  it("rereads the files every round", () => {
    expect(LOOP_PROMPT_INSTRUCTION).toContain("At the start of every round, read the files again instead of relying on what you remember.");
  });

  it("stops only on the templates' own end sentence, or an interrupt", () => {
    expect(LOOP_PROMPT_END_SENTENCE).toBe("Nothing is left to do.");
    expect(LOOP_PROMPT_INSTRUCTION).toContain(
      `Stop the loop only when a round ends with the sentence "${LOOP_PROMPT_END_SENTENCE}", or when I interrupt you.`,
    );
    for (const template of AUTOPILOT_TEMPLATES.filter((entry) => entry.endsWithoutToolCall)) {
      expect(template.prompt).toContain(`"${LOOP_PROMPT_END_SENTENCE}"`);
    }
  });

  it("is plain text: no slash command, no em dash and no arrow", () => {
    const shown = LOOP_PROMPT_SHOWN_INSTRUCTION;
    expect(shown).not.toMatch(/[—–→←⇒]|->|=>/);
    for (const line of shown.split("\n")) expect(line.trimStart().startsWith("/")).toBe(false);
  });
});

describe("the loop prompt's bounds", () => {
  it("keeps every built-in template within a start's prompt bounds", () => {
    for (const template of AUTOPILOT_TEMPLATES) {
      expect(bytes(loopPromptMessage(template.prompt)), template.id).toBeLessThanOrEqual(AUTOPILOT_PROMPT_MAX_BYTES);
      expect(loopPromptError(template.prompt), template.id).toBeUndefined();
    }
  });

  it("refuses an empty prompt, and one the instruction takes past the bounds", () => {
    expect(loopPromptError("  \n")).toBe("The loop prompt needs a prompt.");
    const room = AUTOPILOT_PROMPT_MAX_BYTES - bytes(loopPromptMessage(""));
    expect(loopPromptError("x".repeat(room))).toBeUndefined();
    expect(loopPromptError("x".repeat(room + 1))).toMatch(/longer than 16000 bytes/);
    // A prompt that would fit an autopilot start alone can be too long here.
    expect(loopPromptError("x".repeat(AUTOPILOT_PROMPT_MAX_BYTES))).toBeDefined();
  });
});

describe("which tabs offer it", () => {
  const terminal = (provider: string, extra: Record<string, unknown> = {}) =>
    ({ kind: "terminal", provider, mode: "interactive", ...extra });

  it("is offered on Codex, Claude, DeepSeek and OpenCode terminal tabs", () => {
    for (const provider of ["codex", "claude", "deepseek", "opencode"]) {
      expect(paneCanHaveLoopPrompt(terminal(provider)), provider).toBe(true);
    }
    expect(paneCanHaveLoopPrompt(terminal("claude", { model: "deepseek-v4-flash" }))).toBe(true);
  });

  it("is not offered on OMP, structured, legacy bot or retired tabs", () => {
    expect(paneCanHaveLoopPrompt(terminal("omp"))).toBe(false);
    expect(paneCanHaveLoopPrompt(terminal("pi"))).toBe(false);
    expect(paneCanHaveLoopPrompt({ kind: "agent", provider: "claude", mode: "interactive" })).toBe(false);
    expect(paneCanHaveLoopPrompt({ provider: "codex", mode: "interactive" })).toBe(false);
    expect(paneCanHaveLoopPrompt(terminal("claude", { mode: "deadloop" }))).toBe(false);
    expect(paneCanHaveLoopPrompt(terminal("claude", { model: "MiniMax-M2" }))).toBe(false);
    expect(paneCanHaveLoopPrompt(null)).toBe(false);
  });
});

describe("what the dialog says", () => {
  it("states that APAS only sends the prompt", () => {
    const warning = loopPromptWarning("zoo-005").join(" ");
    expect(warning).toContain("This is a shortcut, not autopilot.");
    expect(warning).toContain("APAS does not run or watch that loop.");
    expect(warning).toContain("It cannot count rounds, enforce a limit, pause, resume or stop it, or tell whether the agent is still looping.");
    expect(warning).toContain("on zoo-005");
    expect(warning).toContain("interrupt it in the terminal (Esc)");
    expect(loopPromptWarning().join(" ")).toContain("on its machine");
  });

  it("states that it spends the tab's login with no spending limit", () => {
    const lines = loopPromptWarning("zoo-005");
    expect(lines).toContain(
      "Until it is stopped, it spends the agent's login on zoo-005, whoever sent the prompt. APAS sets no spending limit.",
    );
    // Before how to stop it, which stays last.
    expect(lines.at(-1)).toBe("To stop it, interrupt it in the terminal (Esc).");
    expect(loopPromptWarning().join(" ")).toContain("spends the agent's login on its machine");
    expect(lines.join(" ")).not.toMatch(/[—–→←⇒]|->|=>/);
  });

  it("describes the templates in rounds, with how each ends", () => {
    for (const template of AUTOPILOT_TEMPLATES) {
      const note = loopPromptTemplateNote(template);
      expect(note, template.id).not.toMatch(/iteration/i);
      expect(note).toContain(template.endsWithoutToolCall ? LOOP_PROMPT_END_SENTENCE : "until you interrupt it");
    }
  });

  it("says a template with no ending repeats at once, also when nothing is left to fix", () => {
    // Under autopilot "Keep the tests green" is paired with "every N minutes";
    // the loop has no timing, so its rounds follow back to back.
    const testsGreen = AUTOPILOT_TEMPLATES.find((template) => template.id === "tests_green")!;
    expect(testsGreen.endsWithoutToolCall).toBe(false);
    expect(loopPromptTemplateNote(testsGreen)).toBe(
      "Each round runs the project's checks and fixes the first failure at its cause. It has no end of its own: each round starts again at once, also when the last one found nothing to fix, until you interrupt it.",
    );
    for (const template of AUTOPILOT_TEMPLATES.filter((entry) => entry.endsWithoutToolCall)) {
      expect(loopPromptTemplateNote(template), template.id).not.toContain("starts again at once");
    }
  });
});
