## 1. Loop prompt

- [x] 1.1 The loop instruction text, the wrapping of a prompt in it, and tests (it is plain text within a start's bounds, and says round after round, no waiting or asking, the end-of-round reading of "stop after one step", rereading files, and the two ways it stops)
- [x] 1.2 The Loop prompt dialog, sharing the template control, saved prompts and own-text rules with the autopilot start dialog; the warning; the shown loop instruction; Send once through the terminal conversation input, with sent / not confirmed
- [x] 1.3 The toolbar control on Codex, Claude (and DeepSeek) and OpenCode terminal tabs, and the phone pane action; not on OMP, non-terminal, legacy or read-only tabs
- [x] 1.4 Tests for 1.2 and 1.3

## 2. Codex locator identity

- [x] 2.1 `transcript/codex_daemon.rs` initializes as `codex_app_server_daemon`, the name Codex's own daemon client uses for read-only queries and one Codex treats as non-originating; request only what locating needs
- [x] 2.2 A test pins the name; check in the isolated harness (`~/.cache/apas-codex-check`, private daemon of the owner's version) that a typed turn's User-Agent is unchanged after a locator query and that the locator still binds the tab

## 3. Withdraw the Codex autopilot plan

- [x] 3.1 Remove `openspec/changes/add-codex-tab-autopilot`; record why in this proposal and in the harness notes

## 4. Documentation and verification

- [x] 4.1 `AGENTS.md`: the loop prompt and its limits; the locator's client name and why
- [x] 4.2 The complete gates; a browser look at the dialog on a Claude and a Codex tab at desktop and phone width
