import { describe, expect, it } from "vitest";
import type { AutopilotSpec } from "@apas/protocol";
import { autopilotStartForm, autopilotStartSpec, AUTOPILOT_PROMPT_MAX_BYTES } from "@/lib/autopilot";
import {
  autopilotTemplateById,
  autopilotTemplateEndingNote,
  autopilotTemplateForPrompt,
  autopilotTemplateStartForm,
  AUTOPILOT_OWN_TEXT,
  AUTOPILOT_TEMPLATES,
  DEFAULT_AUTOPILOT_TEMPLATE,
} from "./autopilotTemplates";

const bytes = (text: string) => new TextEncoder().encode(text).length;

describe("the built-in autopilot templates", () => {
  it("are a short list with distinct ids, names and prompts, none of them the own-text entry", () => {
    expect(AUTOPILOT_TEMPLATES.length).toBeGreaterThanOrEqual(3);
    expect(AUTOPILOT_TEMPLATES.length).toBeLessThanOrEqual(4);
    for (const key of ["id", "name", "prompt"] as const) {
      expect(new Set(AUTOPILOT_TEMPLATES.map((template) => template[key])).size).toBe(AUTOPILOT_TEMPLATES.length);
    }
    expect(AUTOPILOT_TEMPLATES.map((template) => template.id)).not.toContain(AUTOPILOT_OWN_TEXT);
  });

  it("start with the APAS worker, which is the default", () => {
    expect(DEFAULT_AUTOPILOT_TEMPLATE).toBe(AUTOPILOT_TEMPLATES[0]);
    expect(DEFAULT_AUTOPILOT_TEMPLATE.id).toBe("apas_worker");
    expect(DEFAULT_AUTOPILOT_TEMPLATE.name).toBe("APAS worker");
    expect(DEFAULT_AUTOPILOT_TEMPLATE.endsWithoutToolCall).toBe(true);
    const { prompt } = DEFAULT_AUTOPILOT_TEMPLATE;
    expect(prompt).toContain("do one step of that work in this turn, then stop");
    expect(prompt).toContain("Pick the single most valuable step.");
    expect(prompt).toContain("Never disable, skip, delete or weaken a test or a check");
    expect(prompt).toContain("Do not push.");
  });

  it("gives the APAS worker three layers, and the whitepaper to the person alone", () => {
    const prompt = autopilotTemplateById("apas_worker")!.prompt;
    // Layer one: read only. Where it lives, and that nothing in it changes.
    expect(prompt).toContain("docs/whitepaper.md, or the files under docs/whitepaper/ or docs/book/");
    expect(prompt).toContain("You may read it and nothing else: never create, edit, move, reformat or delete anything in it.");
    expect(prompt).toContain("never write a whitepaper yourself");
    // Undoing an edit there must not throw away the person's own edits.
    expect(prompt).toContain("Check with git status that you added, changed, moved or removed nothing under the whitepaper in this turn.");
    expect(prompt).toContain("undo only your own edit, by hand, never with git checkout or restore");
    // What the agent would change in it goes to the report, with answers.
    expect(prompt).toContain("write that in docs/apas-worker/report.md instead");
    expect(prompt).toContain('a line starting "Answer:"');
    expect(prompt).toContain("An answer never lets you edit the whitepaper");
    expect(prompt).toContain("never delete one the person has not resolved");
    // Layer two: the specs may decide what the whitepaper leaves open.
    expect(prompt).toContain("as long as no requirement contradicts the whitepaper. Details it leaves open are yours to decide.");
    expect(prompt).toContain('"openspec validate <name> --strict --no-interactive"');
    expect(prompt).toContain('"openspec archive <name> --yes", never with --skip-specs');
    // Layer three: the code, bound by the specs.
    expect(prompt).toContain("as long as it does what the specs require and nothing they forbid");
    // Nothing is deleted because a layer above does not mention it.
    expect(prompt).toContain("Nothing is removed only because the layer above does not mention it");
    // A question only the author can answer may hold the turn, once.
    expect(prompt).toContain("ask it with your tool for asking the person questions if you have one, then wait for the answer, never twice for one entry");
    expect(prompt).toContain("never take a default or recommended option");
    // Nothing outside the checkout, and nothing installed.
    expect(prompt).toContain("Never do a task outside this checkout");
    expect(prompt).toContain("do not install it");
    expect(prompt).not.toContain("Do not ask me anything.");
  });

  it("keep the design-document template, which does not guess a document or work without one", () => {
    const prompt = autopilotTemplateById("design_document")!.prompt;
    expect(prompt).toContain("Use docs/design.md if it exists.");
    expect(prompt).toContain("Do one step of that work in this turn, then stop");
    expect(prompt).toContain("docs/autopilot-progress.md");
    expect(prompt).toContain("Do not ask me anything.");
    expect(prompt).toContain("Commit the change and the notes together");
    // A project can hold old plans and proposals for removed features: a file
    // that merely looks like a design is not built toward, unattended.
    expect(prompt).toContain("the one document that the README or AGENTS.md names as the project's current design, architecture or specification");
    expect(prompt).toContain("Do not choose a file by its name alone");
    expect(prompt).toContain("do not use a plan, a proposal or a document about a feature that was removed");
    expect(prompt).not.toContain("look in docs/ and in the repository root");
    // Without one it writes nothing, not even the notes.
    expect(prompt).toContain("If the project has no design document, change nothing, say where you looked, and end your reply with the sentence: \"Nothing is left to do.\"");
    expect(prompt).toContain("do not invent work and do not write a design yourself");
  });

  it("include the old bot's TODO.md loop and a test watch", () => {
    expect(autopilotTemplateById("todo")?.prompt).toContain("Work through the tasks in TODO.md. Do one task in this turn");
    expect(autopilotTemplateById("tests_green")?.prompt).toContain("Fix at most one failure in this turn");
    expect(autopilotTemplateById("nope")).toBeUndefined();
    expect(autopilotTemplateById(AUTOPILOT_OWN_TEXT)).toBeUndefined();
  });

  it.each(AUTOPILOT_TEMPLATES.map((template) => [template.id, template] as const))(
    "%s can be started unchanged: it passes the start's own validation",
    (_id, template) => {
      const checked = autopilotStartSpec({ ...autopilotStartForm(), prompt: template.prompt });
      expect(checked.error).toBeUndefined();
      expect(checked.spec?.prompt).toBe(template.prompt);
      // Well under the limit, so there is room to add to it. The APAS
      // worker, which states three layers and a report, is the longest.
      expect(bytes(template.prompt)).toBeLessThan(AUTOPILOT_PROMPT_MAX_BYTES / 2);
    },
  );

  it.each(AUTOPILOT_TEMPLATES.map((template) => [template.id, template] as const))(
    "%s is plain text: no slash command, no control character, nothing to trim",
    (_id, template) => {
      const { prompt } = template;
      expect(prompt).toBe(prompt.trim());
      // Newline is the only character below U+0020: no tab, no carriage return.
      expect(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/.test(prompt)).toBe(false);
      // Slash commands are not run from an autopilot prompt, so none is written.
      for (const line of prompt.split("\n")) {
        expect(line.trimStart().startsWith("/")).toBe(false);
        expect(line).toBe(line.trimEnd());
      }
      // No unfilled placeholder left from composing it.
      expect(prompt).not.toMatch(/\$\{|undefined/);
      expect(template.name.trim()).not.toBe("");
      expect(template.description).not.toContain("\n");
      for (const text of [prompt, template.name, template.description]) {
        expect(text).not.toMatch(/[—→]/);
      }
    },
  );

  it("say to call no tool exactly when they are marked as ending that way", () => {
    for (const template of AUTOPILOT_TEMPLATES) {
      expect(template.prompt.includes("call no tool")).toBe(template.endsWithoutToolCall);
      // The turn that finds nothing left says the sentence the next turn looks for.
      expect(template.prompt.includes("\"Nothing is left to do.\"")).toBe(template.endsWithoutToolCall);
    }
    expect(AUTOPILOT_TEMPLATES.map((template) => template.endsWithoutToolCall)).toEqual([true, true, true, false]);
  });

  it("skip the check only once after each real one, so a later run or a timed repeat looks again", () => {
    const skipped = "\"Nothing has changed since I last looked.\"";
    for (const template of AUTOPILOT_TEMPLATES) {
      const { prompt } = template;
      expect(prompt.includes(skipped)).toBe(template.endsWithoutToolCall);
      if (!template.endsWithoutToolCall) continue;
      // The no-tool turn is taken only right after the exact sentence, and
      // answers with another one. A reply that merely said nothing is left,
      // or the no-tool turn's own reply, sends the next turn to the steps.
      expect(prompt).toContain(
        `If your last reply in this conversation ended with the sentence "Nothing is left to do." and nobody has written anything since except this prompt, call no tool and reply only: ${skipped}`,
      );
      expect(prompt).toContain("In every other case, also when that second sentence was your last reply, do these steps:");
      expect(prompt).not.toContain("do not check again");
      expect(prompt).not.toContain("reply with that same sentence");
      // Every way of finding nothing left ends the reply with the sentence
      // the guard looks for.
      expect(prompt).toContain("end your reply with the sentence: \"Nothing is left to do.\"");
    }
  });

  it("say what to do with a step that cannot be done, so the next turn does not take it again", () => {
    const worker = autopilotTemplateById("apas_worker")!.prompt;
    expect(worker).toContain("Pass over a step the notes mark blocked, unless you have a new way to try it.");
    expect(worker).toContain("mark that step blocked, and commit the notes alone");
    expect(worker).toContain("If step 3 finds the three layers in line, or only blocked steps are left,");

    const design = autopilotTemplateById("design_document")!.prompt;
    expect(design).toContain("Pass over a gap the notes mark blocked, unless you have a new way to try it.");
    expect(design).toContain("mark that gap blocked, and commit the notes alone");
    expect(design).toContain("If step 2 finds nothing left, or only blocked gaps,");

    const todo = autopilotTemplateById("todo")!.prompt;
    expect(todo).toContain("Pick the first open task of the highest priority that is not marked blocked");
    expect(todo).toContain("needs a person, a credential or something this machine does not have");
    expect(todo).toContain("Do not mark the task done: write one line under it that starts with \"Blocked:\" and says why, and commit TODO.md alone.");
    expect(todo).toContain("If TODO.md does not exist or has no open task that is not blocked, change nothing");

    const watch = autopilotTemplateById("tests_green")!.prompt;
    expect(watch).toContain("If this conversation shows that you already tried this failure twice, leave it and take the next failure.");
    expect(watch).toContain("If no failure is left that you can take, change nothing and say so in one sentence.");

    for (const template of AUTOPILOT_TEMPLATES) {
      expect(template.prompt).toContain("commit nothing that is broken, and take your own unfinished edits of this turn back out, only those");
    }
  });

  it("leave other people's work and the history alone in a shared checkout", () => {
    for (const template of AUTOPILOT_TEMPLATES) {
      const { prompt } = template;
      expect(prompt).toContain("Leave changes you did not make as they are, whether they are committed or not: do not undo them and do not commit them.");
      expect(prompt).toContain("Do not stash, reset, clean, amend or rebase anything, and do not skip commit hooks.");
      expect(prompt).toContain("Do not push.");
      expect(prompt).toContain("A failure that comes from uncommitted changes you did not make is not yours to fix: leave those changes alone, say so,");
    }
    // An earlier failure is the turn's step only when it is in committed code.
    expect(autopilotTemplateById("apas_worker")!.prompt).toContain("A failing check in committed code comes first");
    expect(autopilotTemplateById("design_document")!.prompt).toContain(
      "If a check already failed before your change and the failure is in committed code, fix that first",
    );
  });
});

describe("matching a prompt to a template", () => {
  it("names a template only for its unchanged text", () => {
    for (const template of AUTOPILOT_TEMPLATES) {
      expect(autopilotTemplateForPrompt(template.prompt)).toBe(template);
      expect(autopilotTemplateForPrompt(`${template.prompt} `)).toBeUndefined();
    }
    expect(autopilotTemplateForPrompt("")).toBeUndefined();
    expect(autopilotTemplateForPrompt("Check the build.")).toBeUndefined();
  });
});

describe("the start dialog's proposal with templates", () => {
  it("offers the default template to a tab with no earlier autopilot, and changes nothing else", () => {
    const form = autopilotTemplateStartForm();
    expect(form).toEqual({ ...autopilotStartForm(), prompt: DEFAULT_AUTOPILOT_TEMPLATE.prompt });
    expect(form.iterationLimited).toBe(false);
    expect(form.timeLimited).toBe(false);
    expect(autopilotStartSpec(form).spec).toEqual({
      prompt: DEFAULT_AUTOPILOT_TEMPLATE.prompt,
      repeat: { kind: "after_each_turn" },
      iteration_limit: { kind: "none" },
      time_limit: { kind: "none" },
      end_without_tool_call: true,
    });
  });

  it("keeps the last autopilot's values, prompt included, for a tab that had one", () => {
    const last: AutopilotSpec = {
      prompt: "Check the build and fix what fails.",
      repeat: { kind: "every", seconds: 900 },
      iteration_limit: { kind: "at_most", value: 20 },
      time_limit: { kind: "none" },
      end_without_tool_call: false,
    };
    expect(autopilotTemplateStartForm(last)).toEqual(autopilotStartForm(last));
    expect(autopilotStartSpec(autopilotTemplateStartForm(last)).spec).toEqual(last);
  });
});

describe("what the dialog says about a template's ending", () => {
  it("depends on the no-tool-call option for a template that ends that way", () => {
    const on = autopilotTemplateEndingNote(DEFAULT_AUTOPILOT_TEMPLATE, true);
    const off = autopilotTemplateEndingNote(DEFAULT_AUTOPILOT_TEMPLATE, false);
    expect(on).toBe("It ends by itself when nothing is left to do.");
    expect(off).toBe("It ends by itself only if \"End when an iteration makes no tool call\" is ticked below.");
  });

  it("says a template that always checks has no end of its own, whatever the option", () => {
    const watch = autopilotTemplateById("tests_green")!;
    const note = "It has no end of its own. Set a limit, or stop it yourself.";
    expect(autopilotTemplateEndingNote(watch, true)).toBe(note);
    expect(autopilotTemplateEndingNote(watch, false)).toBe(note);
  });
});
