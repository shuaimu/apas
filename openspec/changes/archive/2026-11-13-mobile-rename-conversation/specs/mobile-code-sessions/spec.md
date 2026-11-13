## ADDED Requirements

### Requirement: Mobile users can rename the selected conversation

The mobile session conversation view SHALL let an authorized user rename its
currently selected real pane from the pane's More actions. The rename editor
SHALL start with the pane's current display label, accept only a trimmed,
non-empty label, and apply the saved label to that exact pane everywhere its
name is shown. The control SHALL not offer a usable mutation when no real pane
is selected, the project is not running, or the client is offline.

#### Scenario: Rename the selected conversation

- **WHEN** an online user opens More actions for a selected pane in a running project
- **AND** chooses Rename conversation, edits the prefilled label, and saves a non-empty name
- **THEN** surrounding whitespace is removed
- **AND** the selected pane's conversation heading and selector display the new name
- **AND** the rename follows the existing pane-label persistence and synchronization behavior

#### Scenario: Cancel a rename

- **WHEN** a user opens the rename editor and cancels or dismisses it
- **THEN** the selected pane's label remains unchanged
- **AND** no rename mutation is sent

#### Scenario: Reject an empty name

- **WHEN** the rename draft contains only whitespace
- **THEN** the user cannot save it
- **AND** the existing label remains visible

#### Scenario: No renameable pane is available

- **WHEN** no real pane is selected, the project is stopped, or the client is offline
- **THEN** Rename conversation is unavailable
- **AND** no pane-label mutation is sent
