## 1. Template

- [x] 1.1 Add the "APAS worker" template as the first and default entry in `packages/web/src/lib/autopilotTemplates.ts`, built from the shared sentences the other templates use
- [x] 1.2 Revise its wording after an independent review of how an agent would read it over many iterations
- [x] 1.3 Tests: the default, the three layers and the report in the prompt, the shared rules for every template, the dialog's list of names

## 2. Saved prompts

- [ ] 2.1 Shared types: the request, the change kinds, the answer, the list frame for CLIs and viewers, the capability names, and the validation of names and prompts (one Rust statement, mirrored in the web)
- [ ] 2.2 CLI: read and write `autopilot_prompts` and `default_autopilot_prompt` through the locked `.apas` merge; apply a change and answer it; report the list at registration, after a change and when it changes on disk
- [ ] 2.3 Server: authorize with runtime access, validate, route to the project's instances, relay the primary's answer to the requester only, cache the primary's list and send it to capable viewers on change and on attach
- [ ] 2.4 Protocol: regenerate and add fixtures
- [ ] 2.5 Web: the store's list and requests; the dialog's groups, Save to project, Delete, Make default and Clear default on desktop and the phone sheet; the starting text rule
- [ ] 2.6 Tests in each layer, including a malformed list left untouched, two instances, an older instance, an older viewer, and a save while a tab is on autopilot

## 3. Documentation

- [ ] 3.1 `AGENTS.md`: the four templates, the default, the three layers and the report and progress files; saved prompts, where they are kept and what that means for clones and separate checkouts

## 4. Verification and rollout

- [ ] 4.1 The complete gates: Rust workspace, static musl CLI build, protocol, web suite with lint, typecheck and build, native unchanged, `openspec validate --strict`
- [ ] 4.2 Look at the dialog with saved prompts in a browser at desktop and phone width
- [ ] 4.3 Run "APAS worker" on a real OMP tab in a scratch project with a whitepaper and an OpenSpec tree: a spec step, an implementation step, a report entry, an answered entry, a blocking question, and the two-turn ending
- [ ] 4.4 Deploy server, then web, then push the CLI
