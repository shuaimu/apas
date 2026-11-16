import type { AutopilotSpec } from "@apas/protocol";
import { autopilotStartForm, type AutopilotStartForm } from "@/lib/autopilot";

/**
 * Built-in prompts the Start autopilot dialog offers. A template is only a
 * starting text: the dialog puts it in the prompt field, where it can be
 * read and changed, and what is sent is whatever the field holds. Nothing
 * about a start depends on which template the text came from.
 *
 * How each prompt has to be written, because of how autopilot uses it:
 *
 * - The same text is submitted on every iteration, so it describes one step
 *   and says to stop after it. The agent remembers earlier iterations only
 *   through the repository and the tab's conversation, so a prompt that needs
 *   more than that says where its notes are kept.
 * - It is submitted as plain text. Slash commands and skills in it are not
 *   run, so no line starts with one.
 * - The dialog's "End when an iteration makes no tool call" option is the
 *   only way a prompt can end its own run. Finding out that nothing is left
 *   takes tool calls (reading a file), so such a prompt ends in two turns:
 *   one that finds nothing left and ends its reply with one fixed sentence,
 *   and the next, which sees that sentence and calls no tool. That second
 *   turn answers with a different sentence, so the turn after it looks
 *   again: a run started later, or a timed repeat with the option off, must
 *   not answer "nothing left" for ever without reading the files.
 * - It runs unattended, with no limit unless one is set, often in a checkout
 *   other people and agents work in. So each prompt says what to do with a
 *   step that cannot be done (mark it, so the next turn does not take it
 *   again), and forbids touching work that is not the agent's own.
 */
export interface AutopilotTemplate {
  /** Stable, and the value of the dialog's Template control. */
  id: string;
  name: string;
  /** One line, shown under the Template control. */
  description: string;
  prompt: string;
  /** The prompt tells the agent to call no tool once nothing is left, so the
   *  dialog's no-tool-call option can end the run. */
  endsWithoutToolCall: boolean;
}

// Shared by the templates, word for word: the retired bot's "decide without
// asking, no shortcuts" rule, the two-turn ending described above, and the
// rules for a shared checkout.
const DECIDE = "Do not ask me anything. Where a choice is open, take the option that is honest, complete and cuts no corner.";
const NOTHING_LEFT = "Nothing is left to do.";
const NOTHING_CHANGED = "Nothing has changed since I last looked.";
const END_GUARD = `If your last reply in this conversation ended with the sentence "${NOTHING_LEFT}" and nobody has written anything since except this prompt, call no tool and reply only: "${NOTHING_CHANGED}" In every other case, also when that second sentence was your last reply, do these steps:`;
const SAY_NOTHING_LEFT = `end your reply with the sentence: "${NOTHING_LEFT}"`;
const CHECKS = "Run the project's checks: tests, linter, type checks and build, as the README, AGENTS.md or the CI configuration names them. Fix every failure at its cause. Never disable, skip, delete or weaken a test or a check to make it pass.";
// Another person's or agent's work in progress can make a check fail. It is
// left alone; the two endings say what the turn does instead.
const NOT_YOURS = "A failure that comes from uncommitted changes you did not make is not yours to fix: leave those changes alone, say so,";
const NOT_YOURS_STEP = `${NOT_YOURS} and judge your own step by the checks those changes do not break.`;
const GIVE_UP = "commit nothing that is broken, and take your own unfinished edits of this turn back out, only those";
const COMMIT_RULES_LOCAL = "Commit only files that belong to this step: no temporary files, logs or build output. Leave changes you did not make as they are, whether they are committed or not: do not undo them and do not commit them. Do not stash, reset, clean, amend or rebase anything, and do not skip commit hooks.";
const COMMIT_RULES = `${COMMIT_RULES_LOCAL} Do not push.`;

// The APAS worker keeps three layers in line, each owned differently: the
// whitepaper (a person's; the agent reads it and never changes it), the
// OpenSpec specs (the agent's, bound by the whitepaper) and the code (the
// agent's, bound by the specs). What the agent would change in the
// whitepaper goes to a report the person reads; a question that blocks the
// work is also asked with the agent's question tool, which holds the turn,
// and so the autopilot, until someone answers it.
//
// It is the one template that syncs with the git remote (owner, 2026-11-15):
// a project's worker can run in several checkouts, and the person reads and
// answers the report from wherever they are. It merges, never rebases, pushes
// only its own branch and never forces. Where it cannot reach the remote it
// works on locally; where a merge needs a person (a conflict it cannot
// resolve, a whitepaper conflict, a forced update) it stops syncing, asks,
// and waits. Every rule reads git's state or a file, never the agent's
// memory of an earlier turn. The fetch is part of step 1, below the guard,
// so the no-tool turn that ends a run stays without a tool call. The other
// templates still do not push. A plain push sends every commit on the branch
// in that checkout, also ones other tabs made there.
const WORKER_DIR = "docs/apas-worker";
const WORKER_REPORT = `${WORKER_DIR}/report.md`;
const WORKER_PROGRESS = `${WORKER_DIR}/progress.md`;
const WORKER_VERUS = `${WORKER_DIR}/verus.md`;

const APAS_WORKER_PROMPT = `You are this project's APAS worker. Keep three layers in line with each other, and do one step of that work in this turn, then stop: the same prompt is sent again for the next step. Decide by yourself everything the whitepaper leaves open, taking the option that is honest, complete and cuts no corner. Ask the person only what the whitepaper's author alone can settle, as the paragraph after step 9 says.

The three layers, and what you may do with each:
- The whitepaper is the design a person writes and owns: docs/whitepaper.md, or the files under docs/whitepaper/ or docs/book/, all of them if several exist. If none exists, it is the document or folder the README or AGENTS.md names as the project's whitepaper, book or main design document, never anything under openspec/. You may read it and nothing else: never create, edit, move, reformat or delete anything in it. When you think it is wrong, unclear, contradicts itself or should say more, write that in ${WORKER_REPORT} instead. If docs/apas-worker lies inside the whitepaper, use apas-worker/ in its place.
- The OpenSpec specs (openspec/specs and openspec/changes) are yours to write, change and reorganise, as long as no requirement contradicts the whitepaper. Details it leaves open are yours to decide.
- The implementation (the code, its proofs and tests, and its other documentation) is yours to change, as long as it does what the specs require and nothing they forbid. For Rust code, prove rather than test, as below.

Verus rules, for Rust code:
- Where Verus can check a property of Rust code, write its Verus specification (requires, ensures, loop invariants, decreases) and proof instead of a test, taking the property from its OpenSpec requirement, not from what the code does. Add no dependency but vstd, and change no interface or behaviour, to fit Verus: such code keeps its tests. Test the rest: files, network, terminal, timing, other crates, concurrent or unsafe code no proof models.
- A proof counts only when the project's verification command verifies it. Never make one go through by weakening or adding a requires, ensures or invariant, or with anything Verus trusts unchecked: assume, admit, assume_specification, an external or external_body attribute, assume_new, an axiom, or moving code out of verus!. Only a function calling another crate or using unsafe code may get external_body or assume_specification, listed as trusted in ${WORKER_VERUS} and the commit message.
- An existing test may be deleted only in the commit that adds a verified proof covering it: its inputs and asserted results, written as a kept function inside verus! calling what the test called, verify, and the proof rests on nothing listed as trusted. Name both in the commit message. This is the only exception to the rule in step 6 against deleting tests.
- ${WORKER_VERUS}, part of the notes, lists each Rust test judged: replaced, not replaceable, or blocked, with why. Judge a blocked test again only when its code or requirement has changed.
- Verus is available when the project's verification command, or before setup "verus --version", runs here. If not, write one report entry asking the person to make it available or to keep tests; every Verus step waits for it; resolve it once Verus runs. An answer to keep tests ends these rules.
- If the project does not use Verus yet, set it up as a code step before the first proof, with the Verus here; never install it. Pin its version and vstd, and write the verification command into the README or AGENTS.md and any CI configuration.

Remote rules, for git:
- Keep the branch you are on in step with its upstream, as git has it configured, and no other branch; never switch branches. Run git here with GIT_TERMINAL_PROMPT=0 and GIT_EDITOR=true. If HEAD is detached, the branch has no upstream, git cannot reach the remote or lacks the credential, or git refuses something these rules do not name, do not work around it: skip the sync this turn, work locally, and say so in your reply.
- If step 1 finds a merge, rebase or cherry-pick already under way, it may be a person's: change and commit nothing, say so, and ${SAY_NOTHING_LEFT}
- To sync, run "git fetch", and go no further while a report entry has stopped the sync. If it reports a forced update of the upstream, do not merge: stop the sync as the next rule says. If the upstream has commits this branch lacks, bring them in with "git merge --no-edit", never a rebase or a pull. If git refuses because of uncommitted changes you did not make, leave them and skip the sync this turn. After a merge that brought code, run the checks of step 6 before step 2. Then push if the branch has commits its upstream lacks.
- In a merge conflict keep what both sides meant; where both did the same step, keep the upstream's. Besides what git staged, stage only the files you resolved, and commit a merge without naming paths. If the conflict is only under ${WORKER_DIR}, keep both sides' entries, commit the merge and go on. Any other merge you resolved by hand is this turn's step: run the checks of step 6, commit it, push, and stop. If the conflict is in the whitepaper, or you cannot resolve it or fix a check it breaks, stop the sync: before you commit anything, end an open merge with "git add" of the files you edited, then "git merge --abort", and write one report entry naming this machine, this checkout and what happened, asking the person to bring the branch in step here. Ask it as the paragraph after step 9 says. Every step waits for that entry, and you neither merge nor push while it is open; resolve it once the person has answered it or the upstream contains this branch.
- Push with a plain "git push" after every commit you make. Never force, never push another branch or a tag, never delete anything on the remote, and open no pull request. If a push is rejected because the upstream has moved, leave it and do not end this reply with "${NOTHING_LEFT}": the next turn merges and pushes.
- A note or report entry about what this machine lacks names the machine, and blocks no step on another one.

${END_GUARD}

1. Find the whitepaper; only if one exists, sync as the remote rules say. Then read it in full. Read ${WORKER_REPORT} and ${WORKER_PROGRESS} if they exist: the report holds what you asked or proposed and the person's answers, the progress file holds the notes of earlier turns.
2. If the person has answered an entry in the report (a line starting with "Answer:" under it) and you have not acted on it yet, acting on it is this turn's step: go to step 5. An answer never lets you edit the whitepaper: if it accepts a change there, write under the entry that the person makes it, and the entry stays open until the whitepaper says it. An entry the whitepaper now settles is answered too.
3. Compare the specs with the whitepaper: a requirement that contradicts it, or something it asks for that no requirement covers. Then compare the code with the specs: a requirement or scenario that is missing, incomplete or wrong in it, or behaviour no spec covers. Read the files themselves; do not rely on the notes alone. Nothing is removed only because the layer above does not mention it: write the spec for uncovered behaviour, or, if that spec would contradict the whitepaper, ask about it in the report and leave the code as it is. For Rust code, also judge the tests ${WORKER_VERUS} does not list yet.
4. Pick the single most valuable step. A failing check in committed code comes first, then a spec that contradicts the whitepaper, then code that contradicts a spec, then what is missing, the spec before the code, and last an existing Rust test that a Verus proof can replace; in each, a part that others build on comes before the parts built on it. Pass over a step the notes mark blocked, unless you have a new way to try it. A step that waits for an open entry in the report counts as blocked. If the step is too large for one turn, take its first part that is useful by itself and write the rest into the notes.
5. Do it in its layer. A change to the specs is an OpenSpec change: create it with "openspec new change <name>", write the proposal, the spec deltas, a design where one helps, and tasks, and check it with "openspec validate <name> --strict --no-interactive", which must pass. A change to the code either carries out tasks of an open OpenSpec change, ticking each as it is done, or makes the code meet a requirement already in openspec/specs, named in the commit message. Once all of a change's tasks are done and its checks pass, archive it with "openspec archive <name> --yes", never with --skip-specs. Never do a task outside this checkout, such as deploying or changing a server or an account: leave it unticked; a change waiting only on such tasks counts as blocked. If the project has no openspec directory yet, this turn's step is "openspec init --tools none". A command this machine does not have blocks the step: do not install it. Implement completely: no stubs, no placeholder values, nothing left "for later" in the code. For Rust code, add a proof Verus verifies in this turn, otherwise tests that would fail without the change.
6. ${CHECKS} Where the project uses Verus, its verification is one of these checks; a timeout fails it. If it cannot run here, change no code inside verus! and judge your step by the other checks. ${NOT_YOURS_STEP}
7. Rewrite ${WORKER_PROGRESS}: what this turn did, what is left in the order you would do it, which steps are blocked and why, and anything the next turn must know. Keep it short and current. Add to ${WORKER_REPORT} anything for the person: a proposed change to the whitepaper (where, what it says now, what you would write, why), a question, or an assumption later steps depend on, where the whitepaper allowed more than one reading. Start a new report with a line telling the person to answer an entry with a line starting "Answer:" under it. Give each entry a heading with the date and a short title, and a line "Status: open"; set it to resolved once you have acted on the answer. Do not repeat an open entry, and never delete one the person has not resolved.
8. Check with git status that you added, changed, moved or removed nothing under the whitepaper in this turn. If you did, undo only your own edit, by hand, never with git checkout or restore: the person may have uncommitted edits there. Whitepaper changes that came in with a merge are the person's, not yours. Then commit the step with the notes, with a message that says what was done and which layer it was in. ${COMMIT_RULES_LOCAL} Then push as the remote rules say. Changing committed specs and code is still your job here, and the person's answers are committed with your changes to the report.
9. Say in two or three sentences what you did, what comes next, and why if the branch is not in step with its upstream.

If a step needs an answer only the whitepaper's author can give, write the question in ${WORKER_REPORT} and commit it. If another step can go ahead, take that one instead. If none can, also ask it with your tool for asking the person questions if you have one, then wait for the answer, never twice for one entry. Write an answer given there under the entry as its "Answer:" line and act on it as step 2 says. A question dismissed, cancelled or timed out is not answered: never take a default or recommended option, and the step is blocked.

If you cannot finish the step or make a check pass in this turn, ${GIVE_UP}. Write in ${WORKER_PROGRESS} what you tried and why it failed, mark that step blocked, and commit the notes alone.

If no whitepaper exists, change nothing, say where you looked, and ${SAY_NOTHING_LEFT} If step 3 finds the three layers in line and no Rust test that a proof could replace, or only blocked steps are left, skip steps 4 to 9: do not invent work and never write a whitepaper yourself. Record that in ${WORKER_PROGRESS} if it does not say so already, commit it, name the report entries that wait for an answer, and end your reply with the same sentence.`;

const DESIGN_DOCUMENT_PROMPT = `Bring this project's implementation up to its design document. Do one step of that work in this turn, then stop: the same prompt is sent again for the next step. ${DECIDE}

${END_GUARD}

1. Find the design document. Use docs/design.md if it exists. If it does not, use the project's main design document: the one document that the README or AGENTS.md names as the project's current design, architecture or specification. Do not choose a file by its name alone, and do not use a plan, a proposal or a document about a feature that was removed. If no such document is named, the project has no design document. Read the design document in full. Then read docs/autopilot-progress.md if it exists: it holds the notes of earlier turns.
2. Compare the design with the code as it is now. Read the code itself; do not rely on the notes alone. Work out what the design asks for that is missing, incomplete or wrong.
3. Pick the single most valuable gap or defect. Something broken comes before something missing, and a part that others build on comes before the parts that build on it. Pass over a gap the notes mark blocked, unless you have a new way to try it. If the gap is too large for one turn, take its first part that is useful by itself and write the other parts into the notes.
4. Implement it completely: no stubs, no placeholder values, nothing left "for later" in the code. Add tests that would fail without the change.
5. ${CHECKS} If a check already failed before your change and the failure is in committed code, fix that first and count it as this turn's step. ${NOT_YOURS_STEP}
6. Rewrite docs/autopilot-progress.md: what this turn did, what is left in the order you would do it, which gaps are blocked and why, and anything the next turn must know. Keep it short and current, not a growing log.
7. Commit the change and the notes together, with a message that says what was done. ${COMMIT_RULES}
8. Say in two or three sentences what you did and what comes next.

If you cannot finish the step or make a check pass in this turn, ${GIVE_UP}. Write in docs/autopilot-progress.md what you tried and why it failed, mark that gap blocked, and commit the notes alone.

If the project has no design document, change nothing, say where you looked, and ${SAY_NOTHING_LEFT} If step 2 finds nothing left, or only blocked gaps, do not invent work and do not write a design yourself. Record that in docs/autopilot-progress.md if it does not say so already, commit it, and end your reply with the same sentence.`;

const TODO_PROMPT = `Work through the tasks in TODO.md. Do one task in this turn, then stop: the same prompt is sent again for the next one. ${DECIDE}

${END_GUARD}

1. Read TODO.md as it is now; it may have changed since the last turn. Pick the first open task of the highest priority that is not marked blocked (high, then medium, then low; without priorities, the first such task from the top). If that task has subtasks, pick its first open one that is not marked blocked.
2. If the task is too large for one turn (more than about 500 changed lines), break it down in TODO.md into smaller tasks, nested where that helps, and take the first of them.
3. Do the task completely: no stubs, and nothing left "for later" unless you add it to TODO.md as a task of its own. Add tests that would fail without the change.
4. ${CHECKS} ${NOT_YOURS_STEP}
5. Mark the task done in TODO.md with today's date and one line on what was done or found.
6. Commit the change and TODO.md together, with a message that says what was done. ${COMMIT_RULES}
7. Say in two or three sentences what you did and which task comes next.

If you cannot do the task because it needs a person, a credential or something this machine does not have, or you cannot finish it or make a check pass in this turn, ${GIVE_UP}. Do not mark the task done: write one line under it that starts with "Blocked:" and says why, and commit TODO.md alone.

If TODO.md does not exist or has no open task that is not blocked, change nothing and ${SAY_NOTHING_LEFT}`;

const TESTS_GREEN_PROMPT = `Keep this project's tests green. Fix at most one failure in this turn, then stop: the same prompt is sent again for the next check. ${DECIDE}

1. Run the project's whole test suite, and its linter, type checks and build, as the README, AGENTS.md or the CI configuration names them. Run them now: do not rely on an earlier result, the code may have changed since.
2. If everything passes, change nothing and say so in one sentence.
3. Otherwise take the first failure. ${NOT_YOURS} and take the next failure. Read the failing test and the code it exercises before you change either, and decide which of the two is wrong.
4. Fix the cause. Never disable, skip, delete or weaken a test or a check to make it pass, never change an expected value only to match what the code does now, and add no special case for the test. If the test itself is wrong, correct it and say why in the commit message.
5. Run the failing check again, then everything from step 1, to see that the fix holds and breaks nothing else.
6. Commit the fix, with a message that names the failure and its cause. ${COMMIT_RULES}
7. Say in two or three sentences what failed, what the cause was, and whether other failures remain.

If you cannot fix the failure in this turn, say what you found, ${GIVE_UP}. If this conversation shows that you already tried this failure twice, leave it and take the next failure. If no failure is left that you can take, change nothing and say so in one sentence.`;

/** The first one is what a tab with no earlier autopilot is offered. */
export const AUTOPILOT_TEMPLATES: readonly AutopilotTemplate[] = [
  {
    id: "apas_worker",
    name: "APAS worker",
    description: `Keeps the OpenSpec specs in line with your whitepaper and the code in line with the specs, one step per iteration, with Verus proofs in place of Rust tests where Verus can check them. It merges what the branch's git remote has and pushes its commits there, so run one worker per branch. It never edits the whitepaper: proposals and questions go to ${WORKER_REPORT}.`,
    prompt: APAS_WORKER_PROMPT,
    endsWithoutToolCall: true,
  },
  {
    id: "design_document",
    name: "Implement from a design document",
    description: "Each iteration closes one gap between docs/design.md and the code, with tests and a commit.",
    prompt: DESIGN_DOCUMENT_PROMPT,
    endsWithoutToolCall: true,
  },
  {
    id: "todo",
    name: "Work through TODO.md",
    description: "Each iteration does the next open task in TODO.md, with tests and a commit.",
    prompt: TODO_PROMPT,
    endsWithoutToolCall: true,
  },
  {
    id: "tests_green",
    name: "Keep the tests green",
    description: "Each iteration runs the project's checks and fixes the first failure at its cause.",
    prompt: TESTS_GREEN_PROMPT,
    endsWithoutToolCall: false,
  },
];

export const DEFAULT_AUTOPILOT_TEMPLATE: AutopilotTemplate = AUTOPILOT_TEMPLATES[0];

/** The Template control's entry for a prompt that is not a template's. It is
 *  not a template id. */
export const AUTOPILOT_OWN_TEXT = "own_text";
export const AUTOPILOT_OWN_TEXT_LABEL = "My own text";
export const AUTOPILOT_OWN_TEXT_DESCRIPTION =
  "The prompt below is submitted unchanged on every iteration.";
/** Shown while a template has replaced text the person wrote. */
export const AUTOPILOT_OWN_TEXT_KEPT_TEXT =
  `Your own text is kept: choose "${AUTOPILOT_OWN_TEXT_LABEL}" to get it back. Editing this prompt replaces it.`;

export function autopilotTemplateById(id: string): AutopilotTemplate | undefined {
  return AUTOPILOT_TEMPLATES.find((template) => template.id === id);
}

/** The template a prompt is, unchanged; a prompt that differs from every
 *  template by even one character is the person's own text. */
export function autopilotTemplateForPrompt(prompt: string): AutopilotTemplate | undefined {
  return AUTOPILOT_TEMPLATES.find((template) => template.prompt === prompt);
}

/**
 * The dialog's proposal with templates: `autopilotStartForm`, except that a
 * tab with no earlier autopilot is offered the project's default saved
 * prompt, when it has one, else the default template's prompt, instead of an
 * empty field. A tab with one keeps that autopilot's prompt. Timing, limits
 * and the no-tool-call option are never taken from a template.
 */
export function autopilotTemplateStartForm(last?: AutopilotSpec, projectDefaultPrompt?: string): AutopilotStartForm {
  const form = autopilotStartForm(last);
  return last ? form : { ...form, prompt: projectDefaultPrompt ?? DEFAULT_AUTOPILOT_TEMPLATE.prompt };
}

/**
 * What the dialog says about how a template's run ends, given the
 * no-tool-call option as it stands. Choosing a template never ticks or
 * clears that option; this line says what the combination means instead.
 */
export function autopilotTemplateEndingNote(template: AutopilotTemplate, endWithoutToolCall: boolean): string {
  if (!template.endsWithoutToolCall) {
    return "It has no end of its own. Set a limit, or stop it yourself.";
  }
  return endWithoutToolCall
    ? "It ends by itself when nothing is left to do."
    : "It ends by itself only if \"End when an iteration makes no tool call\" is ticked below.";
}
