## ADDED Requirements

### Requirement: The session screen favours the conversation over its chrome

On a session screen the controls used while following a conversation SHALL be reachable without displacing the conversation itself. Moving between the session's panes SHALL be available in the topmost row of the screen. Controls that navigate away from the session SHALL NOT appear there. Actions used occasionally against the selected pane, and project management, SHALL be gathered behind a single control in that same row rather than each taking space of its own — neither above the conversation nor alongside the message composer, which is for composing.

#### Scenario: Opening a session

- **WHEN** a user opens a session on a phone
- **THEN** the pane list is in the topmost row, beside the control that returns to the session list
- **AND** the project's identity and state are still shown

#### Scenario: Switching panes

- **WHEN** a user switches panes
- **THEN** they do so from that same topmost row without scrolling the conversation away

#### Scenario: Occasional pane actions

- **WHEN** a user opens the control that gathers the occasional actions
- **THEN** it offers the raw terminal and the work summary for the selected pane, and closing that pane
- **AND** each acts on the selected pane

#### Scenario: The composer is for composing

- **WHEN** a user looks at the message composer
- **THEN** it carries only composing and sending
- **AND** the occasional pane actions are not duplicated there

#### Scenario: Closing the selected pane

- **WHEN** a user chooses to close the selected pane
- **THEN** the system confirms before anything is closed
- **AND** a pane holding an isolated worktree offers the same choice of what to do with that work as other surfaces do, rather than discarding it silently

#### Scenario: Leaving for account settings

- **WHEN** a user is on a session screen
- **THEN** no control there navigates to account settings

### Requirement: A project's allowed tab types can be managed from mobile

The application SHALL let a user reach project management for the session they are viewing from the control that gathers the session screen's occasional actions, and there see which tab types the project permits. A user who may manage the project SHALL be able to change that set; any other user SHALL see it without being able to change it. A project that has never been restricted SHALL show every tab type as permitted.

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

#### Scenario: A tab type the cluster policy already forbids

- **WHEN** a tab type is not permitted by the effective cluster policy for the project
- **THEN** project management SHALL NOT present it as permitted
- **AND** SHALL indicate that the restriction is not the project's to change
- **AND** no user, including one who may manage the project, may permit it there

#### Scenario: Existing panes are unaffected

- **WHEN** a tab type is removed from the permitted set while panes of that type are open
- **THEN** those panes keep running and can still be relaunched
- **AND** only the creation of new tabs of that type is refused
