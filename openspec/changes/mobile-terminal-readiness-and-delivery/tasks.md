## 1. Confirmation rule

- [x] 1.1 Decide delivery by whether the agent recorded the message, kept out of the component so it can be tested against turns
- [x] 1.2 Ignore turns older than the send, and require the record to be the user's rather than the agent repeating it
- [x] 1.3 Hold a grace period so a polled transcript does not make every message flicker a warning
- [x] 1.4 Tests: recorded, framed, whitespace-different, echoed by the agent, pre-dating the send, absent, and late

## 2. Conversation view

- [x] 2.1 Present a terminal pane with no recorded turns as not started, offering the terminal view
- [x] 2.2 Leave the ordinary empty state for panes whose activity is observed directly
- [x] 2.3 Report unconfirmed messages above the composer, offering the terminal view
- [x] 2.4 Withdraw the report when the agent records the message
- [x] 2.5 Tests: both states render, the plain empty state survives, and the report appears only after the grace period

## 3. Mobile terminal controls

- [x] 3.1 Add a touch key accessory that emits byte-exact Escape, arrow, Enter, Tab, and Ctrl-C input
- [x] 3.2 Show the accessory in the mobile raw-terminal view and disable it while disconnected
- [x] 3.3 Test every emitted key sequence and the mobile integration

## 4. Exact Codex terminal resume

- [x] 4.1 Read the Codex session id from the process-owned rollout metadata already selected for the pane
- [x] 4.2 Persist a verified Codex session id into the pane roster and publish the updated pane list
- [x] 4.3 Restore Codex with the exact captured id, retaining the picker for unknown legacy ids and never using `--last`
- [x] 4.4 Test metadata parsing, identity persistence, exact resume arguments, and legacy fallback

## 5. Verification

- [x] 5.1 Earlier delivery-confirmation web lint, type-check, and tests clean
- [x] 5.2 New focused web and Rust tests clean
- [x] 5.3 Full web lint/type-check/test and relevant Rust crate tests clean
- [ ] 5.4 Live on the mobile browser against a pane sitting at a provider startup prompt
