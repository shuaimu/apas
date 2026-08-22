## Why

A terminal pane's conversation view cannot tell you whether the agent is ready
to be talked to. A Codex pane sitting at its own startup prompt — "resume a
previous session?" — renders as "No activity yet", which is exactly how a
healthy, quiet agent renders. Typing `hello` there writes the keystrokes into a
menu, where they select something or nothing; the sender sees no difference and
reasonably concludes the message was delivered.

The conversation view writes into a pty and never reads back what happened. That
is by design — it is the practical way to drive an agent from a phone — but it
means "sent" has been asserted on the strength of a successful write, which
proves only that bytes were accepted.

## What Changes

- **A terminal pane with no recorded turns says so.** It has demonstrably not
  begun a conversation, and the view says that instead of "No activity yet",
  noting it may be at a prompt of its own and offering the raw terminal.
- **A message the agent never records is reported as unconfirmed.** After a
  grace period, the sender is told it was not recorded and pointed at the raw
  terminal, rather than being left to assume.
- **Confirmation is by reading, never by writing** — the same rule the answering
  of agent questions already follows. A message that comes back as a user turn
  landed; one that never does did not.
- **No attempt to recognise the startup screens themselves.** Detecting "Codex
  is asking about resuming" means parsing TUI output, which is exactly what APAS
  avoids by reading transcripts, and would fail silently whenever a provider
  redesigns its startup.
- **The mobile raw terminal supplies the keys a phone keyboard omits.** A small
  terminal accessory row sends Escape, arrow, Enter, Tab, and Ctrl-C bytes to
  the selected pane, so provider menus remain operable without a hardware
  keyboard.
- **A restored Codex pane resumes its own conversation when APAS knows it.**
  Once the pane's process-owned rollout reveals Codex's actual session id, APAS
  persists that id and supplies it to `codex resume` on future restores. A
  legacy pane with no captured id keeps Codex's picker; APAS does not use
  `--last`, which could select another pane's conversation in the same working
  directory.

## Capabilities

### Modified Capabilities

- `mobile-code-sessions`: the conversation view distinguishes an agent that has
  not started a conversation from one that is merely quiet, reports a message
  the agent never recorded, and makes provider prompts operable on a phone.

## Impact

- `packages/web/src/lib/terminalDelivery.ts`: the confirmation rule, kept out of
  the component so it can be tested against turns directly.
- `packages/web/src/components/mobile/MobileSessionActivity.tsx`: both notices,
  each offering the raw terminal and its mobile key accessory.
- `crates/client-cli/src/transcript.rs` and the terminal-pane runtime: capture
  Codex's exact session identity from the rollout APAS already attributes to
  the pane, persist it in the existing `PaneConfig.session_id`, and use it on
  restore.
- No server or protocol change: terminal input and pane persistence already
  carry the required bytes and identity.
