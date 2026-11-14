## Why

Two requests from the owner on 2026-11-14, shipped together because both change what the start dialog offers.


The owner works with three layers: a whitepaper or book that a person writes as the main design, OpenSpec specs derived from it, and the implementation that follows the specs. Autopilot's default template ("Implement from a design document") treats the design document and the code as two layers and gives no rule for who may change the design. The owner asked for a template that keeps the three layers in line and leaves the whitepaper to the person.

## What Changes

- A new built-in autopilot template, "APAS worker", becomes the first entry and the default for a tab with no earlier autopilot.
- It tells the agent, one step per iteration, to keep the specs in line with the whitepaper (no requirement may contradict it) and the code in line with the specs. The whitepaper is read only for the agent; what it would change there, its questions and the assumptions it made go to `docs/apas-worker/report.md`, where the person answers. A question that blocks all remaining work is also asked with the agent's question tool, which holds the autopilot until someone answers. Its notes between iterations are in `docs/apas-worker/progress.md`.
- Changes to the specs go through OpenSpec changes, checked with `openspec validate --strict` and archived once implemented.
- It ends its run like the other self-ending templates: two turns, and only with "End when an iteration makes no tool call" ticked.
- "Implement from a design document", "Work through TODO.md" and "Keep the tests green" stay, in that order after it.
- **Saved prompts per project.** Anyone who may start autopilot on a project's tabs can save the dialog's prompt to the project under a name, replace or delete a saved prompt, and make one the project's default. They are kept in the project's `.apas` file, listed in the Template control before the built-ins, and a project's default replaces "APAS worker" as the starting text for its tabs.

## Capabilities

### New Capabilities

### Modified Capabilities
- `omp-tab-autopilot`: the start dialog offers four templates, with "APAS worker" as the default, and a project's own saved prompts with an optional project default.

## Impact

- Web: `packages/web/src/lib/autopilotTemplates.ts`, the start dialog (desktop and phone sheet), the store, and their tests.
- Shared types and protocol: the save request, its answer and the project's list, with capability markers for CLIs and viewers.
- Server: authorization, routing to the project's instances, and an in-memory cache of each project's list for viewers that attach.
- CLI: reading and writing the list in `.apas` through the existing locked merge, and reporting it.
- `AGENTS.md`, the autopilot section's template description.
- The rule is in the prompt only: nothing stops an agent from editing the whitepaper except its instructions and its own check before each commit. A project that wants it enforced can add a commit hook or protect the files.
