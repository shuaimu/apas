## ADDED Requirements

### Requirement: OpenCode terminal conversations are recovered from retained sessions
For an OpenCode terminal pane, the project host SHALL recover conversation turns from an OpenCode session whose recorded directory exactly matches the pane working directory. It SHALL expose real user and completed assistant text as pane conversation history, exclude synthetic, ignored, reasoning, tool, and attachment parts, and preserve reported model and token usage when available.

#### Scenario: OpenCode session exists for the pane directory
- **WHEN** the transcript watcher finds one or more retained OpenCode sessions whose directory exactly matches the pane working directory
- **THEN** it selects the most recently updated matching session and exports its conversation
- **AND** it does not select a session belonging to another directory

#### Scenario: Export contains internal OpenCode parts
- **WHEN** an exported conversation includes reasoning, tool, synthetic, ignored, or attachment parts alongside human-visible text
- **THEN** APAS emits only real user and assistant text into conversation history
- **AND** internal parts are not exposed as conversation messages

#### Scenario: Assistant response is still streaming
- **WHEN** an exported OpenCode assistant message has not reached its recorded completion boundary
- **THEN** APAS withholds that assistant message from persisted conversation history
- **AND** a later poll may emit the complete response without leaving an unrecoverable partial turn

#### Scenario: Final OpenCode response reports usage
- **WHEN** a completed assistant response contains model and token usage metadata
- **THEN** APAS attributes that metadata to the owning pane and completion boundary
- **AND** the terminal pane becomes idle only at a final non-tool-call completion

### Requirement: OpenCode terminal restoration continues retained work
When an APAS CLI process restores a persisted OpenCode terminal pane, it SHALL start a new PTY process using OpenCode's continuation behavior and SHALL NOT replay the pane's original initial instruction.

#### Scenario: APAS CLI restores an OpenCode terminal pane
- **WHEN** the APAS CLI restarts with a persisted OpenCode terminal pane
- **THEN** it re-executes the configured OpenCode CLI in the pane working directory using continuation mode
- **AND** it reports the new terminal process instance through normal lifecycle reconciliation
- **AND** it does not submit the original prompt again
