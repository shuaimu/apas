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
  });

  it("has the APAS worker sync its own branch with the git remote, and nothing more", () => {
    const { prompt, description } = DEFAULT_AUTOPILOT_TEMPLATE;
    expect(description).toContain("It merges what the branch's git remote has and pushes its commits there, so run one worker per branch.");
    // It merges first and pushes after every commit.
    expect(prompt).toContain("1. Find the whitepaper; only if one exists, sync as the remote rules say. Then read it in full.");
    expect(prompt).toContain('To sync, run "git fetch", and go no further while a report entry has stopped the sync.');
    expect(prompt).toContain('bring them in with "git merge --no-edit", never a rebase or a pull');
    expect(prompt).toContain("Then push if the branch has commits its upstream lacks.");
    expect(prompt).toContain('Push with a plain "git push" after every commit you make.');
    expect(prompt).toContain("Then push as the remote rules say.");
    expect(prompt).not.toContain("Do not push.");
    // Only its own branch, never by force, and nothing else on the remote.
    expect(prompt).toContain("Keep the branch you are on in step with its upstream, as git has it configured, and no other branch; never switch branches.");
    expect(prompt).toContain("Never force, never push another branch or a tag, never delete anything on the remote, and open no pull request.");
    // It never waits for input and never works around a refusal.
    expect(prompt).toContain("Run git here with GIT_TERMINAL_PROMPT=0 and GIT_EDITOR=true.");
    expect(prompt).toContain("or git refuses something these rules do not name, do not work around it: skip the sync this turn, work locally, and say so in your reply");
    // Other people's work, uncommitted, under way or arriving, is kept.
    expect(prompt).toContain("If step 1 finds a merge, rebase or cherry-pick already under way, it may be a person's: change and commit nothing, say so,");
    expect(prompt).toContain("If git refuses because of uncommitted changes you did not make, leave them and skip the sync this turn.");
    expect(prompt).toContain("Besides what git staged, stage only the files you resolved, and commit a merge without naming paths.");
    expect(prompt).toContain("Whitepaper changes that came in with a merge are the person's, not yours.");
    // A merge that needs a person stops the sync and is asked about; the
    // agent never settles a whitepaper conflict or a forced update itself.
    expect(prompt).not.toContain("take the upstream's side in the whitepaper");
    expect(prompt).toContain("If it reports a forced update of the upstream, do not merge: stop the sync as the next rule says.");
    expect(prompt).toContain('If the conflict is in the whitepaper, or you cannot resolve it or fix a check it breaks, stop the sync: before you commit anything, end an open merge with "git add" of the files you edited, then "git merge --abort"');
    expect(prompt).toContain("Every step waits for that entry, and you neither merge nor push while it is open; resolve it once the person has answered it or the upstream contains this branch.");
    // Two workers do not spend every other turn merging their notes.
    expect(prompt).toContain("If the conflict is only under docs/apas-worker, keep both sides' entries, commit the merge and go on.");
    expect(prompt).toContain("A note or report entry about what this machine lacks names the machine, and blocks no step on another one.");
    // What arrived is checked before the turn builds on it.
    expect(prompt).toContain("After a merge that brought code, run the checks of step 6 before step 2.");
    // The history rules still hold for it.
    expect(prompt).toContain("Do not stash, reset, clean, amend or rebase anything, and do not skip commit hooks.");
  });

  it("keeps the APAS worker's sync from breaking the two-turn ending", () => {
    const { prompt } = DEFAULT_AUTOPILOT_TEMPLATE;
    // The turn that must call no tool comes before anything that fetches.
    const guard = prompt.indexOf("call no tool and reply only");
    expect(guard).toBeGreaterThan(prompt.indexOf("Remote rules, for git:"));
    expect(prompt.indexOf("only if one exists, sync as the remote rules say")).toBeGreaterThan(guard);
    expect(prompt).not.toContain("Before step 1");
    // A rejected push must not be followed by the sentence that ends the run.
    expect(prompt).toContain('If a push is rejected because the upstream has moved, leave it and do not end this reply with "Nothing is left to do.": the next turn merges and pushes.');
    // The reply, which is seen from anywhere, says when the branch is behind.
    expect(prompt).toContain("and why if the branch is not in step with its upstream");
    // Nobody else syncs.
    for (const template of AUTOPILOT_TEMPLATES.filter((entry) => entry.id !== "apas_worker")) {
      expect(template.prompt).not.toContain("git push");
      expect(template.prompt).not.toContain("git fetch");
    }
  });

  it("has the APAS worker change a graphical interface only where it can look at the result", () => {
    const { prompt, description } = DEFAULT_AUTOPILOT_TEMPLATE;
    expect(description).toContain("It changes a graphical interface only where its agent can look at the result.");
    const rules = prompt.indexOf("Screen rules, for a graphical interface:");
    // The rules are read before the steps, and before the turn that calls no tool.
    expect(rules).toBeGreaterThan(prompt.indexOf("Remote rules, for git:"));
    expect(rules).toBeLessThan(prompt.indexOf("call no tool and reply only"));
    // Looking means a browser, computer use or a screenshot it can read.
    expect(prompt).toContain("needs you to look at the result in this turn, with a browser or computer-use tool, or a screenshot you take and read as an image.");
    expect(prompt).toContain("Reading the markup or passing tests is not looking, and a screenshot you cannot see as an image does not count.");
    expect(prompt).toContain("Use what is here; install nothing for it.");
    // Where it can look, the look is a check made before the commit.
    expect(prompt).toContain("look at every state the step changed before you commit, and fix what is wrong: this is one of the checks of step 6.");
    // Where it cannot, that part of the design is left alone and the person is told.
    expect(prompt).toContain("If you cannot look, or cannot run the interface here, do not work on that part of the design at all, its specs included: it is blocked.");
    expect(prompt).toContain("Write one report entry naming this machine and the parts that wait, and take another step; resolve the entry once you can look.");
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

  it("has the APAS worker prove Rust code with Verus instead of testing it, without cheating", () => {
    const prompt = autopilotTemplateById("apas_worker")!.prompt;
    expect(prompt).toContain("Verus rules, for Rust code:");
    expect(prompt).toContain("write its Verus specification (requires, ensures, loop invariants, decreases) and proof instead of a test, taking the property from its OpenSpec requirement, not from what the code does");
    // Nothing Verus trusts unchecked makes a proof count.
    for (const hatch of ["assume,", "admit,", "assume_specification", "external_body", "assume_new", "an axiom", "moving code out of verus!"]) {
      expect(prompt).toContain(hatch);
    }
    expect(prompt).toContain("Never make one go through by weakening or adding a requires, ensures or invariant");
    // A test goes only with a verified proof that covers it, and that is the
    // only exception to the rule against deleting tests.
    expect(prompt).toContain("An existing test may be deleted only in the commit that adds a verified proof covering it");
    expect(prompt).toContain("This is the only exception to the rule in step 6 against deleting tests.");
    // The judged tests are recorded, so a run does not retry them forever.
    expect(prompt).toContain("docs/apas-worker/verus.md");
    // Nothing is installed; a missing Verus is a question for the person.
    expect(prompt).toContain('before setup "verus --version"');
    expect(prompt).toContain("never install it");
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
      // Under the limit with room to add to it and for the loop prompt's
      // instruction around it (loopPrompt.test.ts). The APAS worker, which
      // states three layers, a report, the Verus rules and the remote
      // rules, is the longest.
      expect(bytes(template.prompt)).toBeLessThan(AUTOPILOT_PROMPT_MAX_BYTES * 0.9);
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
    expect(worker).toContain("If step 3 finds the three layers in line and no Rust test that a proof could replace, or only blocked steps are left,");

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
      // Only the APAS worker syncs with the remote; the others never push.
      if (template.id === "apas_worker") expect(prompt).not.toContain("Do not push.");
      else expect(prompt).toContain("Do not push.");
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
