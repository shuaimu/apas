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

## 3. Verification

- [x] 3.1 Web lint, type-check, and tests clean
- [ ] 3.2 Live on the mobile browser against a pane sitting at a provider startup prompt
