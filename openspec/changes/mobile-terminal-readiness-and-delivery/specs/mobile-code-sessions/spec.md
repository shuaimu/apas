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
