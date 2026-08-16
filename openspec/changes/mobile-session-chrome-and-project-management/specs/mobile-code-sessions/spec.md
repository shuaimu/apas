## ADDED Requirements

### Requirement: The session screen favours the conversation over its chrome

On a session screen the controls used while following a conversation SHALL be reachable without displacing the conversation itself. Moving between the session's panes SHALL be available in the topmost row of the screen. Controls that navigate away from the session SHALL NOT appear there, and controls used occasionally against the selected pane SHALL be placed with the message composer rather than above the conversation.

#### Scenario: Opening a session

- **WHEN** a user opens a session on a phone
- **THEN** the pane list is in the topmost row, beside the control that returns to the session list
- **AND** the project's identity and state are still shown

#### Scenario: Switching panes

- **WHEN** a user switches panes
- **THEN** they do so from that same topmost row without scrolling the conversation away

#### Scenario: Occasional pane actions

- **WHEN** a user opens the raw terminal for a terminal pane, or its work summary
- **THEN** those controls are with the composer
- **AND** they act on the selected pane

#### Scenario: Leaving for account settings

- **WHEN** a user is on a session screen
- **THEN** no control there navigates to account settings

### Requirement: A project's allowed tab types can be managed from mobile

The application SHALL let a user open project management for the session they are viewing, and there see which tab types the project permits. A user who may manage the project SHALL be able to change that set; any other user SHALL see it without being able to change it. A project that has never been restricted SHALL show every tab type as permitted.

#### Scenario: An owner restricts what may be created

- **WHEN** a user who may manage the project clears a tab type from the permitted set
- **THEN** the project stops permitting new tabs of that type
- **AND** the change applies to the project rather than to that user's device

#### Scenario: A user who may not manage the project

- **WHEN** a user who may not manage the project opens project management
- **THEN** they see which tab types are permitted
- **AND** cannot change them

#### Scenario: A project with no restriction

- **WHEN** a project has never had its tab types restricted
- **THEN** every tab type shows as permitted
- **AND** nothing suggests the project is restricted

#### Scenario: Existing panes are unaffected

- **WHEN** a tab type is removed from the permitted set while panes of that type are open
- **THEN** those panes keep running and can still be relaunched
- **AND** only the creation of new tabs of that type is refused
