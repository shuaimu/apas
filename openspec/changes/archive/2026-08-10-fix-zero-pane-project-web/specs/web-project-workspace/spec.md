## Purpose

Defines how the web workspace represents selected projects, including empty projects and creation of their first pane.

## ADDED Requirements

### Requirement: Selected zero-pane projects remain operable
When a project session is selected and its authoritative pane configuration is empty, the web client SHALL render the normal project workspace with the Overview and pane-management controls. The web client MUST treat zero panes as a valid project state rather than as the absence of a project.

#### Scenario: Open a newly created project
- **WHEN** a user selects an attached project whose authoritative pane configuration contains zero panes
- **THEN** the web client displays the normal project workspace with the Overview selected
- **AND** the pane creation control is visible and usable
- **AND** the no-project fallback is not displayed

### Requirement: Users can create the first pane from the web
The web workspace SHALL allow a user to submit an allowed pane configuration when the selected project currently has no panes, using the same creation flow and policy filtering used for projects that already have panes.

#### Scenario: Create the first pane
- **WHEN** a user chooses an allowed pane type from the creation control in a selected zero-pane project
- **THEN** the web client submits a pane creation request for the selected project session
- **AND** the pane appears as a selectable workspace tab when the authoritative pane configuration is updated

#### Scenario: Apply pane policy to the first pane
- **WHEN** a selected zero-pane project disallows a pane launch profile
- **THEN** the creation control does not offer that profile as an available first-pane choice

### Requirement: No-project fallback remains distinct
The web client SHALL display the no-project fallback only when there is no selected project session. That fallback SHALL NOT expose project-specific pane-management controls.

#### Scenario: No project is selected
- **WHEN** the web client has no selected project session
- **THEN** it displays the no-project fallback
- **AND** it does not display the project Overview or pane creation control
