## Why

When a Claude terminal pane asks a question, the conversation view shows nothing. The turn is in the transcript — `AskUserQuestion` is recorded in full, with every question and option — but APAS drops it: `transcript.rs` skips any turn whose content carries no text, a rule written for `Bash` and `Read`. Across the transcripts on one developer machine, 170 `AskUserQuestion` calls were recorded and 169 were tool-use-only, so essentially every question an agent has ever asked from a terminal pane was discarded as noise.

The pane is then stuck: it is waiting for an answer the human cannot see, and on a phone the xterm view is close to unusable, which is exactly where the conversation view was supposed to be the answer. Agent panes have had a full question card, answer queue, and retransmit path since the structured stream carried the tool call; terminal panes lost it when the real TUI replaced stream-json.

## What Changes

- A Claude terminal pane's `AskUserQuestion` turn is **published rather than skipped**, carrying the tool call's own structure, so the existing `AskUserQuestionCard` renders it with no new wire message and no new renderer.
- The human answers it **from the conversation view**, reusing the `AnswerQuestion` path that already runs from the web through the server to the CLI. Only the final hop is new: a terminal pane has no stream-json control channel, so the CLI translates the answer into keystrokes on the pty instead of writing a `control_response` to stdin.
- An answer is **confirmed by reading, not by assuming**. The transcript's `tool_result` for that `tool_use_id` is published as the acknowledgement the web's pending-answer queue already waits for, so the card settles on what the agent actually recorded rather than on what APAS believes it sent.
- A question that is **already answered cannot be answered again**: once its `tool_result` exists, the pane refuses further keystrokes for that `tool_use_id`. Terminal writes are blind, and a stale browser tab replaying a selection into a live prompt is the failure this must not allow.
- An unanswered question is reported as **Pending answer**, distinct from Working, Idle, Usage limited, and Offline. Project lists, waiting-agent lists, pane tabs, and conversation status surfaces preserve that distinction until the provider records the answer.
- Scope is **Claude terminal panes**. `AskUserQuestion` is Claude's; Codex and OpenCode have their own approval interfaces and are untouched.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-pane-continuity`: terminal conversation history gains agent questions and their recorded answers, and the conversation view becomes able to answer a pending question rather than only reading history.

## Impact

- `crates/client-cli/src/transcript.rs`: the claude parser keeps a tool-use-only turn when the tool is a question, and carries its structured input and `tool_use_id` on `TurnRecord`; it also emits the matching `tool_result` as a turn.
- `crates/client-cli/src/mode/dual_pane.rs`: `conversation_turn_to_stream_messages` dresses a question turn as a `ToolUse` content block and an answer turn as a `ToolResult`, and `ServerToCli::AnswerQuestion` gains a terminal-pane branch that writes keystrokes to the pty.
- `crates/client-cli/src/terminal_pane.rs`: keystroke delivery for a selection, alongside the existing `TerminalInput` write path.
- `crates/server` and `crates/shared`: the existing pane-status cache classifies a question as awaiting an answer, reports it separately from active work in session summaries, and returns it to working only when the recorded tool result arrives.
- `packages/web`: the question card is reused as-is; project and waiting-agent lists plus pane/conversation indicators render Pending answer without treating it as working or ordinary idle.
- Docs: the terminal-pane section of `CLAUDE.md`, which currently states the conversation view shows only user/assistant turns.
