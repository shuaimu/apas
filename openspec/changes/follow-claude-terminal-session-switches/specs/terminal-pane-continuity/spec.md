# terminal-pane-continuity Spec Delta

## ADDED Requirements

### Requirement: Claude terminal conversations follow in-TUI session switches

For a Claude terminal pane, the transcript watcher SHALL follow the conversation the user is actually in. When the APAS-pinned session file stops growing and a newer, unpinned session file appears in the pane's working-directory slug directory, the watcher SHALL switch to reading that file. An unpinned session file is one whose name does not match any session id pinned by APAS for a pane in the project. The watcher SHALL NOT abandon a pinned file that is still growing, SHALL NOT switch to a file pinned to a different pane, and SHALL re-baseline to the new source's end on every switch without replaying prior turns.

#### Scenario: User resumes a different Claude session inside the TUI

- **WHEN** a user switches to another Claude session inside a terminal pane using the in-TUI resume picker
- **THEN** the watcher detects that the pinned session file has stopped growing and that a newer unpinned session file exists in the pane's cwd slug directory
- **THEN** the watcher begins reading the unpinned file and subsequent user and assistant turns appear in the pane's conversation history
- **AND** the conversation history does not replay the turns from before the switch

#### Scenario: The pinned conversation is still in flight

- **WHEN** the pinned session file has changed (size or modification time) within the most recent polls
- **THEN** the watcher keeps reading the pinned file and does not switch, even if newer unpinned files exist

#### Scenario: A sibling pane's pinned session is never stolen

- **WHEN** a newer session file in the same cwd slug directory matches another pane's pinned session id
- **THEN** the watcher never switches to that file

#### Scenario: User switches back to the pinned session

- **WHEN** a user resumes the APAS-pinned session again inside the TUI
- **THEN** the watcher returns to reading the pinned session file
- **AND** the conversation history continues from the end of the pinned transcript without duplicates

#### Scenario: No switch has occurred

- **WHEN** the pinned session file keeps growing and no newer unpinned session file appears in the pane's cwd slug directory
- **THEN** the watcher continues reading the pinned file exactly as before
