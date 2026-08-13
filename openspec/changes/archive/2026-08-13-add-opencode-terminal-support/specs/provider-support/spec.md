## ADDED Requirements

### Requirement: OpenCode is available as a policy-controlled terminal provider
APAS SHALL offer OpenCode as a user-created terminal provider on supported launch surfaces when `terminal:opencode:official:default` is permitted by the effective project policy. The project host SHALL run the configured OpenCode interactive CLI with a non-blocking permission mode, SHALL deliver a fresh launch instruction using OpenCode's supported initial-prompt interface, and SHALL use OpenCode's continuation interface when restoring the pane.

#### Scenario: User creates an allowed OpenCode terminal
- **WHEN** the effective project policy permits the OpenCode terminal profile and a user selects OpenCode from a terminal launch surface
- **THEN** APAS creates an unmanaged terminal pane that hosts the real OpenCode interactive CLI
- **AND** the pane receives terminal input, resize, lifecycle, and output through the same terminal transport used by other supported providers

#### Scenario: Project policy disallows OpenCode terminal launch
- **WHEN** a user attempts to create an OpenCode terminal while its launch profile is absent from the effective project allowlist
- **THEN** the server and project host reject the launch
- **AND** no OpenCode process is spawned

#### Scenario: OpenCode binary is unavailable
- **WHEN** an authorized OpenCode terminal launch reaches a project host whose configured OpenCode binary cannot be executed
- **THEN** the pane reports an actionable spawn error identifying the configured binary
- **AND** other panes in the project remain available

### Requirement: Retained headless OpenCode panes use native OpenCode event semantics
For persisted legacy or managed panes that still use the structured agent path, APAS SHALL invoke OpenCode with its supported non-interactive JSON interface and SHALL translate OpenCode text, tool-use, completion, usage, and error events into the shared pane message model. APAS SHALL NOT pass an APAS UUID as though it were an OpenCode-generated session identifier.

#### Scenario: Headless OpenCode emits text and completion events
- **WHEN** a retained structured OpenCode pane emits native JSON text followed by a final completion event
- **THEN** APAS displays the assistant text and records a successful turn completion
- **AND** preserves reported token usage and cost when present

#### Scenario: OpenCode emits an intermediate tool-call completion
- **WHEN** OpenCode finishes a tool-call step but continues executing the same user turn
- **THEN** APAS does not mark the pane idle at that intermediate boundary
- **AND** waits for a final assistant completion or error

#### Scenario: Headless OpenCode resumes previous work
- **WHEN** a retained structured OpenCode pane continues after its first invocation
- **THEN** APAS uses OpenCode's continuation behavior for the pane working directory
- **AND** does not submit the APAS pane session UUID as an OpenCode session ID
