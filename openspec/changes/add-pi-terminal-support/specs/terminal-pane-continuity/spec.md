## ADDED Requirements

### Requirement: Pi terminal conversations are recovered from the provider's session files
For a Pi terminal pane, the project host SHALL recover conversation turns from the session file carrying the pane's exact pinned identity. It SHALL expose real user and completed assistant text as pane conversation history, SHALL exclude thinking, tool-result, shell-execution, extension, compaction, branch-summary, label, and model bookkeeping entries, and SHALL preserve reported model, token usage, and cost when available.

#### Scenario: Pane session contains conversation turns
- **WHEN** the transcript watcher reads the session file for a Pi terminal pane
- **THEN** it emits each real user message and each assistant text response as conversation history for that pane

#### Scenario: Session contains non-conversation entries
- **WHEN** a Pi session includes thinking blocks, tool results, shell-execution records, extension entries, compaction or branch summaries, labels, or model and thinking-level changes
- **THEN** APAS does not expose those entries as conversation messages
- **AND** internal data is not published as user or assistant text

#### Scenario: Assistant response continues into tool use
- **WHEN** a persisted assistant message records a tool-use continuation rather than a completed turn
- **THEN** APAS records the assistant text but does not mark the pane idle
- **AND** the pane leaves working state only at a provider-confirmed terminal completion

#### Scenario: Completed response reports usage and cost
- **WHEN** a completed assistant message carries model, token usage, or cost metadata
- **THEN** APAS attributes that metadata to the owning pane
- **AND** reports the recorded cost rather than a fabricated value when the provider supplies one

#### Scenario: Multiple Pi panes share a working directory
- **WHEN** more than one Pi terminal pane runs in the same working directory
- **THEN** each pane's history is recovered from its own pinned session identity
- **AND** no pane displays another pane's conversation

### Requirement: Pi terminal restoration continues the exact retained session
When an APAS CLI process restores a persisted Pi terminal pane, it SHALL start a new PTY process attached to the pane's exact retained session and SHALL NOT replay the pane's original initial instruction.

#### Scenario: APAS CLI restores a Pi terminal pane
- **WHEN** the APAS CLI restarts with a persisted Pi terminal pane whose exact session is retained
- **THEN** it re-executes the configured Pi CLI in the pane working directory attached to that session
- **AND** it reports the new terminal process instance through normal lifecycle reconciliation
- **AND** it does not submit the original prompt again

### Requirement: Pi structured questions are not answered from the conversation view
APAS SHALL NOT treat a Pi tool call as an answerable question until a provider-confirmed structured question interface can be verified against the real TUI. Pi assistant text SHALL still be published normally, and APAS SHALL NOT deliver blind keystrokes to a Pi pane on behalf of a question.

#### Scenario: Pi session contains a tool call the agent used to ask something
- **WHEN** a Pi session records a tool call that is not a proven structured question interface
- **THEN** APAS does not publish it as an answerable question card
- **AND** does not write arbitrary keystrokes into the pane's terminal on the human's behalf
