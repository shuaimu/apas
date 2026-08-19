## ADDED Requirements

### Requirement: Managed team roles run as terminal panes

Managed team roles SHALL run on the same pane kind as ordinary work. The system SHALL allow a managed pane of that kind, and SHALL provide such a pane with the delegation tools the roles use, the working directory its role requires, and the diffs its role publishes.

#### Scenario: Starting a team

- **WHEN** a team is started
- **THEN** its roles run as terminal panes
- **AND** each can read and write the team's shared records

#### Scenario: A developer role with isolated work

- **WHEN** a developer role runs in an isolated worktree
- **THEN** its pane works in that directory
- **AND** its diff is available as for any other pane

#### Scenario: A pane that already exists as the retired kind

- **WHEN** a project still holds a managed pane of the retired kind
- **THEN** the system reports that it can no longer be run
- **AND** does not silently present it as running

### Requirement: Repeating work is driven through the pane's own session

Where a role repeats work on an interval, the system SHALL deliver each iteration into the pane's running session and SHALL treat the iteration as complete when the provider records the turn. An iteration the provider never records SHALL be reported rather than repeated, since a provider that cannot accept input will not accept a retry either.

#### Scenario: An iteration completes

- **WHEN** a repeating role is given its prompt and the provider records the turn
- **THEN** the iteration is complete
- **AND** the next one is scheduled after the configured interval

#### Scenario: The provider never records the iteration

- **WHEN** an iteration is not recorded within the grace period
- **THEN** the system reports the pane as stalled
- **AND** does not silently repeat the prompt

#### Scenario: Work in progress is not interrupted

- **WHEN** a role's provider is still working on the previous iteration
- **THEN** no new iteration is delivered

### Requirement: A managed pane reports whether it is working

The system SHALL report whether a managed pane is currently working, derived from what its provider records, so that the surfaces which schedule and display work do not have to distinguish pane kinds.

#### Scenario: A working role

- **WHEN** a role's provider has recorded activity more recently than its last completed turn
- **THEN** the pane reports as working

#### Scenario: An idle role

- **WHEN** a role's provider has completed its last turn
- **THEN** the pane reports as not working

## REMOVED Requirements

### Requirement: Retained headless OpenCode panes use native OpenCode event semantics

**Reason**: The headless pane kind these semantics belong to is retired with this change; OpenCode runs as a terminal pane, whose turns are recovered from the provider's own transcript.

**Migration**: Existing headless OpenCode panes are reported as no longer runnable and are re-created as terminal panes, which is the same path every other provider takes.
