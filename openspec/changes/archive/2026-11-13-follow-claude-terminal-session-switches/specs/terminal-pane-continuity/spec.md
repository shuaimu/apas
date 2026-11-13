# terminal-pane-continuity Spec Delta

## ADDED Requirements

### Requirement: Claude terminal conversations follow in-TUI session switches

For a Claude terminal pane, the transcript watcher SHALL follow the conversation the user is in. When the provider reports the transcript it writes, that report SHALL decide which file the watcher reads, and a changed report SHALL move the watcher at once. A search for a more recently written session file SHALL be a fallback only. It SHALL run only after the tracked file has been unchanged for two consecutive polls, SHALL look in the directory of the tracked file, SHALL NOT select a file named for the conversation of another pane that the runtime runs or that `.apas` records for any machine, and SHALL NOT select a file that has not advanced since the watch began. A file selected by the fallback SHALL NOT keep the watcher away from a reported or assigned transcript that exists. Every change of source SHALL re-baseline to the new source's end without replaying prior turns.

#### Scenario: User resumes a different Claude session inside the TUI

- **WHEN** a user switches to another Claude session inside a terminal pane and the provider reports the new transcript
- **THEN** the watcher reads the reported file and later turns appear in the pane's conversation history
- **AND** the conversation history does not replay the turns from before the switch

#### Scenario: The tracked conversation is still in flight

- **WHEN** the tracked file changed in size or modification time within the two most recent polls
- **THEN** the watcher does not look for another file

#### Scenario: Another pane's conversation is never adopted

- **WHEN** a newer session file in the same directory is named for the conversation of another pane of the project, on this machine or on another machine that shares the home directory
- **THEN** the watcher never switches to that file

#### Scenario: A new pane has written nothing yet

- **WHEN** a pane's own transcript does not exist yet and older session files are present in its directory
- **THEN** the watcher adopts none of them unless one advances after the watch began

#### Scenario: User switches back to the assigned session

- **WHEN** a user resumes the pane's assigned session again inside the TUI
- **THEN** the watcher returns to that session's file
- **AND** the conversation history continues from its end without duplicates
