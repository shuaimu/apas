## 1. Verify the keystroke encoding against the real TUI

- [x] 1.1 Drive `claude --dangerously-skip-permissions` on a pty with a prompt that reliably triggers `AskUserQuestion`, and record which byte sequences move and confirm a selection (digits, arrows + CR, or both)
- [x] 1.2 Establish what a multi-question call requires: whether the TUI advances to the next question on confirm, and what separates one selection from the next
- [x] 1.3 Write the finding into the encoder as a comment, so a future break is traceable to an observed behavior rather than an assumption

## 2. Publish questions from the transcript

- [x] 2.1 Extend `TurnRecord` with an optional structured question (tool_use_id, tool name, raw `input`) and an optional answer (tool_use_id, recorded text)
- [x] 2.2 In the claude parser, keep a tool-use-only turn when its tool is `AskUserQuestion` instead of skipping it, and populate the question fields
- [x] 2.3 Emit the matching `tool_result` as an answer turn, keyed by `tool_use_id`
- [x] 2.4 Leave every other tool-use-only turn skipped, and leave the codex and OpenCode parsers untouched
- [x] 2.5 Tests: a question turn survives with its options, its answer turn is produced, a `Bash` tool-use-only turn is still dropped, and an unanswered question yields no answer turn

## 3. Dress questions as the stream messages the card already reads

- [x] 3.1 In `conversation_turn_to_stream_messages`, emit a question turn as an assistant message whose content is a `ToolUse` block with the original id, name, and input
- [x] 3.2 Emit an answer turn as `ClaudeStreamMessage::User` carrying a `ToolResult` block for that id — the variant the server's converter actually reads for tool results
- [x] 3.3 Confirm no new wire message, storage path, or web renderer is required for either
- [x] 3.4 Tests: both conversions produce the shapes the existing card and ack path consume

## 4. Deliver an answer to the pty

- [x] 4.1 Derive pending questions per pane from the transcript: a `tool_use_id` with a `tool_use` and no `tool_result`
- [x] 4.2 Handle `ServerToCli::AnswerQuestion` for terminal panes by encoding the answers into keystrokes and writing them to the pane's pty
- [x] 4.3 Refuse the answer when the `tool_use_id` is not currently pending, when it has already been delivered, or when the pane has no live pty — sending nothing in each case
- [x] 4.4 Keep the existing agent-pane branch (`control_response` on stdin) unchanged and select on pane kind
- [x] 4.5 Tests: a pending question delivers once, a repeat delivers nothing, an answered question is refused, an unknown pane is refused, and an agent pane still takes the stdin path

## 5. Web

- [x] 5.1 Confirm the conversation view renders the question card for a terminal pane and that its answer reaches `WebToServer::AnswerQuestion` with the pane's session
- [x] 5.2 Ensure the card settles on the recorded answer from the `tool_result` rather than on the submission
- [x] 5.3 Tests: a terminal pane question renders and is answerable; a question whose recorded answer differs shows the recorded one

## 6. Documentation and verification

- [x] 6.1 Update `CLAUDE.md`: the conversation view shows agent questions and can answer them, and why the answer is confirmed from the transcript rather than from the write
- [x] 6.2 `cargo test` for the workspace and `cargo clippy` clean
- [x] 6.3 `npm run lint` and `npm test` clean in `packages/web`
- [ ] 6.4 End-to-end against a real pane: ask a question from a Claude terminal pane, answer it from the conversation view, confirm the agent proceeds with that answer and the card shows what was recorded

## 7. Report pending questions as human attention

- [x] 7.1 Add a canonical Pending answer pane status, report it separately from `is_working` and idle recency in server/mobile session summaries, and retain it until a matching recorded tool result arrives
- [x] 7.2 Teach the web store to reconcile live and snapshot Pending answer state without restoring stale Working indicators
- [x] 7.3 Render Pending answer in project lists, waiting-agent lists, pane selectors, pane overview cards, and conversation status bars, ahead of ordinary idle and usage-limited panes
- [x] 7.4 Add Rust and web regression coverage for question → pending → recorded answer → working transitions and list/status presentation
- [x] 7.5 Update the terminal-pane documentation and run focused Rust/web tests, formatting, lint, and full relevant suites
