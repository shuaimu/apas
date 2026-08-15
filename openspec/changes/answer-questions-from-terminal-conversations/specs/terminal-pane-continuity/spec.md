## ADDED Requirements

### Requirement: Terminal conversations surface agent questions

A terminal pane's conversation history SHALL include a question the agent asks, carrying the question text and every offered option, even when the underlying turn contains no prose. A question that has been answered SHALL also carry the recorded answer. Turns that are only tool activity and ask nothing SHALL continue to be omitted.

#### Scenario: Agent asks a question in a terminal pane

- **WHEN** an agent in a terminal pane asks the human a question
- **THEN** the conversation view shows that question with its options
- **AND** shows it as awaiting an answer

#### Scenario: Question is answered outside the conversation view

- **WHEN** the question is answered directly in the terminal instead
- **THEN** the conversation view shows the question with the answer that was recorded
- **AND** no longer presents it as awaiting an answer

#### Scenario: Ordinary tool activity stays out of the history

- **WHEN** the agent runs a tool without asking anything
- **THEN** that activity does not appear as a conversation turn

### Requirement: A pending terminal question can be answered from the conversation view

The conversation view SHALL let the human answer a pending question, and the system SHALL deliver that answer to the agent running in the pane. Delivery SHALL be reported as complete only once the agent has recorded the answer, and the recorded answer SHALL be what the view then shows. An answer that cannot be delivered SHALL leave the question pending rather than appear answered.

#### Scenario: Human answers from the conversation view

- **WHEN** the human selects an option for a pending question in the conversation view
- **THEN** the system delivers that selection to the agent in that pane
- **AND** the question settles once the agent records an answer
- **AND** the view shows the answer the agent recorded

#### Scenario: Recorded answer differs from the submitted one

- **WHEN** the answer the agent records is not the option that was submitted
- **THEN** the conversation view shows the recorded answer rather than the submitted one

#### Scenario: Answer is submitted while the pane is unreachable

- **WHEN** the human answers a question whose pane cannot currently be reached
- **THEN** the system does not present the question as answered
- **AND** the question remains answerable once the pane is reachable again

### Requirement: An answered terminal question is never answered twice

Once an answer has been recorded for a question, the system SHALL refuse further answers to that same question and SHALL send nothing to the pane for it. Input on behalf of an answer SHALL only ever be delivered for a question that is still pending.

#### Scenario: Stale client answers an already-answered question

- **WHEN** a client that has not seen the answer submits one for a question the agent already answered
- **THEN** the system refuses the submission
- **AND** sends nothing to the pane
- **AND** the recorded answer is unchanged

#### Scenario: The same answer arrives twice

- **WHEN** an answer for a pending question is submitted more than once
- **THEN** the agent receives the selection at most once
