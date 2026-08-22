## ADDED Requirements

### Requirement: An agent that has not started a conversation is distinguishable from a quiet one

Where a pane's turns are recovered from what the agent records, a pane with no recorded turns SHALL be presented as not having started a conversation, rather than as having no activity. The presentation SHALL note that the agent may be starting up or waiting at a prompt of its own, and SHALL offer the terminal view where that state is visible and answerable.

#### Scenario: A pane at its provider's startup prompt

- **WHEN** a user opens a terminal pane whose agent has recorded no turns
- **THEN** the view says the agent has not started a conversation
- **AND** offers the terminal view

#### Scenario: A pane whose turns are observed rather than recovered

- **WHEN** a user opens a pane whose activity is observed directly rather than recovered from a transcript
- **THEN** the ordinary empty state is shown, since no such inference is available

### Requirement: A message the agent never records is reported as unconfirmed

Where a message is delivered to an agent as keystrokes, the system SHALL NOT treat a successful write as delivery. It SHALL confirm delivery only by the agent recording the message, and SHALL report a message that remains unrecorded after a grace period as unconfirmed, offering the terminal view. A message recorded later SHALL clear the report.

#### Scenario: A message that lands

- **WHEN** the agent records a message the user sent
- **THEN** nothing is reported

#### Scenario: A message sent while the agent cannot receive it

- **WHEN** the agent has not recorded a sent message once the grace period has passed
- **THEN** the system reports it as not recorded
- **AND** offers the terminal view

#### Scenario: Confirmation arrives late

- **WHEN** a message is reported as unconfirmed and the agent then records it
- **THEN** the report is withdrawn

#### Scenario: Within the grace period

- **WHEN** a message has been sent and the grace period has not passed
- **THEN** nothing is reported, since recording legitimately lags

#### Scenario: The agent merely repeats the text

- **WHEN** the agent's own output contains the text but the user's message was never recorded
- **THEN** the message is still reported as not recorded

### Requirement: Provider prompts are operable from a mobile terminal

The mobile raw-terminal view SHALL provide touch controls for terminal keys
that ordinary phone keyboards omit. The controls SHALL send terminal input to
the selected pane through the same byte-preserving path as xterm input and
SHALL remain generic to the provider's TUI rather than recognizing a particular
startup screen.

#### Scenario: Navigate a resume picker without arrow keys

- **WHEN** a mobile user opens a terminal pane at a provider selection prompt
- **THEN** the user can send Up and Down arrow keys and Enter from touch controls

#### Scenario: Dismiss or interrupt a terminal prompt

- **WHEN** a mobile user needs to dismiss a prompt or interrupt the running command
- **THEN** the user can send Escape or Ctrl-C from touch controls

#### Scenario: Terminal input is disconnected

- **WHEN** the browser is not connected to the pane transport
- **THEN** the terminal key controls are disabled

### Requirement: A Codex terminal pane resumes its exact known conversation

APAS SHALL associate a Codex terminal pane with the actual user-session id from
the process-owned rollout it already uses for transcript recovery. Once known,
the id SHALL be persisted in the pane configuration and supplied to Codex on a
future restore. APAS SHALL NOT select the most recent cwd conversation as an
identity substitute.

#### Scenario: Restore after Codex identity was captured

- **WHEN** a Codex terminal pane is restored after APAS captured its session id
- **THEN** APAS launches `codex resume` with that exact id
- **AND** Codex does not require the user to choose among unrelated conversations

#### Scenario: Restore a legacy pane without a captured identity

- **WHEN** a Codex terminal pane has no verified Codex session id
- **THEN** APAS launches the ordinary Codex resume picker
- **AND** the mobile terminal controls make that picker operable

#### Scenario: Several Codex panes share a working directory

- **WHEN** multiple Codex conversations exist in the same working directory
- **THEN** APAS never uses `--last` to restore a pane
- **AND** only a session id attributed to that pane's provider process is persisted
