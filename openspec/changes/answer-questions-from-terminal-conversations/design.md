## Context

See proposal.md — Why. What already exists matters more than what is missing:

- **The question is in the transcript.** A claude `AskUserQuestion` is an assistant record with `stop_reason: "tool_use"` whose content is a single `tool_use` block; `input.questions[].question` and `.options[].label` carry the full text. The answer arrives as the matching `tool_result` ("The user answered: …"). Verified against real transcripts, not assumed.
- **`transcript.rs` drops it on purpose.** `if text.trim().is_empty() { continue }` — "the tool calls themselves are not conversation". Correct for `Bash`, wrong for a question.
- **The whole answer pipeline exists.** `WebToServer::AnswerQuestion { session_id, tool_use_id, pane_id, answers }` → `ServerToCli::AnswerQuestion` → the CLI's streaming worker writes a `control_response` onto claude's stdin. The web has `AskUserQuestionCard`, a persisted `PendingAnswer` queue, retransmission, and an `answeredQuestions` mirror that survives refresh.
- **The ack the web waits for is "the answered tool_result arrives"**, which is exactly what a terminal transcript produces.
- **Permission prompts are not in scope and do not occur**: terminal panes launch with `--dangerously-skip-permissions` (`terminal_pane.rs:575`), so the only blocking questions are ones the agent asks deliberately.

## Goals / Non-Goals

**Goals:**

- A pending question is visible and answerable from the conversation view, on a phone, without touching the xterm view.
- A pane blocked on that question is visibly Pending answer everywhere pane activity is shown, rather than being collapsed into Working or Idle.
- Reuse the existing card, queue, retransmit, and wire messages; the only genuinely new mechanism is keystroke delivery.
- Never report an answer as delivered on the strength of having written bytes at a pty.

**Non-Goals:**

- Parsing the TUI's output to detect dialog state (the coupling terminal panes exist to avoid).
- Codex and OpenCode approval flows, which are their own interfaces.
- Permission prompts, which this configuration does not produce.
- `ExitPlanMode`: same shape in principle, but no plan-mode record was available to verify against, so it stays out until one is.

## Decisions

1. **Publish the question as the tool call it already is.** The parser keeps a tool-use-only turn when its tool is `AskUserQuestion`, and `conversation_turn_to_stream_messages` emits a `ClaudeContentBlock::ToolUse { id, name, input }` instead of a `Text` block. `AskUserQuestionCard` then renders it with no web change, no new wire message, and no new storage path — the property that made terminal conversation history cheap in the first place. Alternative considered: flattening the question into markdown text — cheaper still, but it produces a description of a question rather than an answerable one, which is the entire point.

2. **The answer reuses `AnswerQuestion` and diverges only at the pty.** The web already sends `tool_use_id` plus a question→option map. For a terminal pane the CLI translates that map into keystrokes rather than a `control_response`. Everything above the CLI — routing, `resolve_target_session`, the pending queue, retransmit — is untouched, so the paths cannot drift and mobile inherits it for free.

3. **The transcript is the acknowledgement.** The CLI publishes the `tool_result` for an answered question as `ClaudeStreamMessage::User` carrying a `ToolResult` block — deliberately the one variant the server's converter reads for tool results, and the reason ordinary non-assistant turns avoid it. The card settles on the *recorded* answer, so a keystroke that selected the wrong option shows the truth rather than the submission. Alternative considered: acking on write success — rejected, since a successful pty write proves only that bytes were accepted.

4. **Pending state is derived, not stored.** A question is pending when its `tool_use_id` has a `tool_use` and no `tool_result` in the transcript. This rests on the record landing *before* the answer, which is verified rather than assumed: three transcripts on this machine hold an `AskUserQuestion` with no `tool_result` at all — one from a real project in July, two from probe runs killed while the picker was on screen. A question is therefore visible to APAS during exactly the window when someone needs to answer it. That is the same evidence the conversation view is built on, it survives a CLI restart, and it needs no new bookkeeping. The CLI refuses an answer for any `tool_use_id` that is not currently pending, which is what makes a stale tab or a duplicate retransmit harmless.

5. **Keystroke encoding is `↑/↓` then `Enter`, established by observation.** Driving the real TUI (Claude Code 2.1.233) on a pty showed:
   - the picker prints its own contract in the footer — `Enter to select · ↑/↓ to navigate · Esc to cancel`;
   - `\x1b[B` moves the selection (watched the `❯` marker step from option 1 to option 2);
   - a digit does not move it, so number shortcuts are not a usable encoding;
   - the agent's own options occupy positions 1..N, followed by the TUI's own `Type something` and `Chat about this`, so stepping down by the option's index from the default first entry lands on the intended option and never on a TUI-added entry.

   So the encoder sends `Down × index` then `CR`. What is *not* yet proven end to end is a keystroke producing a recorded `tool_result` in one automated run; the harness kept losing the race on prompt submission, and this is cheaper and more honest to verify through a real APAS pane, where spawning, trust, and submission are already solved. That check is task 6.4, and decision 3 is what makes it a check rather than an assumption: a wrong encoding shows up as a recorded answer that differs, not as a silent success.

6. **One answer, one delivery.** The pane sends keystrokes for a `tool_use_id` at most once. Combined with decision 4 this makes the web's existing retransmit safe: a retransmitted answer for a still-pending question is refused as already delivered, and one for an answered question is refused as settled.

7. **Pending answer is an additive pane-status classification.** The existing status string continues to carry detailed working text and now uses one canonical Pending answer value for a blocked question. Session summaries add a defaulted `awaiting_answer` flag per pane; `is_working` excludes that value, and idle recency excludes it too. This keeps rolling compatibility while letting list snapshots heal missed live frames. The server sets Pending answer when it receives the question and returns the pane to Working only when the matching transcript `tool_result` arrives—not merely when answer bytes are queued. Web surfaces give Pending answer visual and ordering precedence over ordinary idle and usage-limited states.

## Risks / Trade-offs

- [The write is blind: nothing proves the pane is sitting on the question when the keys land] → Decision 4 narrows the window to "the transcript says this question is unanswered", decision 6 stops repeats, and decision 3 reports what was actually recorded. What remains is a genuine race — the human answering in the terminal at the same instant — whose worst outcome is one stray keystroke, and which the recorded-answer display makes visible rather than silent.
- [A multi-question call is a sequence, and a partially delivered sequence leaves the picker mid-dialog] → Deliver the whole sequence in one write and confirm from the recorded answer, which covers every question in the call. A mismatch is displayed, not hidden.
- [Keystroke encoding could change with a Claude Code release] → It is observed rather than assumed, and pinned to a version in decision 5. The recorded-answer check surfaces a break as a visible mismatch instead of a silent wrong selection, and nothing else in terminal panes depends on it.
- [The TUI appends its own `Type something` / `Chat about this` entries below the agent's options] → Stepping down by index from the first entry cannot reach them; only an out-of-range index could, which the encoder rejects rather than sends.
- [A question asked before this ships is in the transcript but was never published] → It appears the first time its pane's transcript is re-read, which is the existing behavior for history generally; if it was already answered it arrives settled.

## Migration Plan

The original answer path remains behavior-only. Pending-answer presentation adds a defaulted summary field and a canonical status value, so older clients ignore the field and continue treating the non-empty status as activity during a rolling upgrade. Deploy server/shared first, then web, then CLI if CLI changes are needed. A newer web against an older server simply lacks the pending flag and retains today's Working display. Rollback is a binary swap.
