## MODIFIED Requirements

### Requirement: A person can put an OMP terminal tab on autopilot

The system SHALL let anyone who may send input to a terminal tab whose provider is OMP start autopilot on it by giving a prompt, a repeat timing and, if they want them, limits. While a tab is on autopilot, the tab SHALL submit that prompt again each time an iteration is due and the tab has settled, until autopilot pauses or ends. A start SHALL carry a non-empty prompt of at most 16,000 bytes with no control characters other than newline and tab, and a repeat timing that is either "as soon as each turn ends" or an interval between one minute and 24 hours. An iteration limit and a time limit SHALL each be optional, and the start dialog SHALL propose none. A start SHALL say, for each of the two, either the limit or that there is none. When set, an iteration limit SHALL be between 1 and 1,000,000, and a time limit SHALL be between one minute and 365 days and not shorter than the repeat interval. A start that breaks one of these bounds, or that does not say whether it has an iteration limit or a time limit, SHALL be refused with the reason, and the bounds SHALL be checked again on the tab's machine and by the tab itself. The start dialog SHALL offer the prompt, timing and limits of the last autopilot the tab reports. For a tab that reports no earlier autopilot, the dialog SHALL offer the default prompt template as the text of the prompt field, where the person can read and change it. The prompt a start carries SHALL be the text the prompt field holds when the person confirms the start, and nothing else: the server, the tab's machine and the tab SHALL supply no prompt of their own. A start for a tab that is on autopilot or has a paused autopilot SHALL be refused and SHALL leave that autopilot unchanged.

#### Scenario: Start on an idle tab
- **WHEN** a person starts autopilot on an idle OMP terminal tab with a prompt, "as soon as each turn ends", 20 iterations and 8 hours
- **THEN** the tab submits the prompt once and reports autopilot as on with those limits
- **AND** the tab is shown as Working

#### Scenario: Start without a limit
- **WHEN** a person starts autopilot and leaves the iteration limit and the time limit at "No limit", as the start dialog proposes
- **THEN** the tab submits the prompt once and reports autopilot as on with no iteration limit and no time limit

#### Scenario: Start while the tab is working
- **WHEN** autopilot is started on a tab whose agent is in the middle of a turn
- **THEN** nothing is submitted until that turn ends, and autopilot is shown as starting
- **AND** the first iteration is submitted once the tab has settled

#### Scenario: A start outside the bounds is refused
- **WHEN** a start carries an empty prompt, an iteration limit of zero, a time limit of ten seconds, a repeat interval of ten seconds, or a time limit shorter than its repeat interval
- **THEN** it is refused with the reason and nothing is submitted to the tab

#### Scenario: A start that does not say whether it is limited
- **WHEN** a start carries neither a number nor "none" for its iteration limit or for its time limit
- **THEN** it is refused with the reason, and no autopilot is started

#### Scenario: A tab already on autopilot
- **WHEN** a second start arrives for a tab that is on autopilot or has a paused autopilot
- **THEN** it is refused with "This tab is already on autopilot. Stop it first."
- **AND** the existing autopilot keeps its prompt, limits and progress

#### Scenario: Start again after an autopilot ended
- **WHEN** a person opens the start dialog on a tab whose last autopilot ended at its time limit
- **THEN** the dialog shows that autopilot's prompt, timing and limits, ready to change
- **AND** nothing is sent until the person confirms the start

#### Scenario: Start on a tab with no earlier autopilot
- **WHEN** a person opens the start dialog on a tab that reports no earlier autopilot
- **THEN** the prompt field holds the whole text of the default template, "Implement from a design document", and both limits read "No limit"
- **AND** nothing is sent until the person confirms the start

#### Scenario: Tabs that cannot be put on autopilot
- **WHEN** the selected tab is not a terminal tab, runs another provider, or is read-only legacy bot history
- **THEN** no autopilot control is offered for it, and a start sent for it is refused

## ADDED Requirements

### Requirement: The start dialog offers prompt templates

The start dialog, on the desktop and in the phone's sheet, SHALL have a Template control above the prompt field. The control SHALL offer three built-in templates, "Implement from a design document", "Work through TODO.md" and "Keep the tests green", and the entry "My own text". "Implement from a design document" SHALL be the default: it is what a tab with no earlier autopilot is offered. Choosing a template SHALL replace the text of the prompt field with that template's whole text and SHALL change nothing else: the repeat timing, the limits and the "End when an iteration makes no tool call" option SHALL stay as they were. The control SHALL name a template only while the prompt field holds that template's text unchanged, and SHALL read "My own text" for any other text, so that it cannot disagree with what will be sent. A template is a starting text only: a start SHALL carry exactly the text the prompt field holds, and SHALL carry nothing that says which template the text came from.

Under the control the dialog SHALL show one line that describes the selected template and says how its run ends with the no-tool-call option as it stands. For "Implement from a design document" and "Work through TODO.md" the line SHALL say "It ends by itself when nothing is left to do." when the option is ticked and SHALL say that it ends by itself only if the option is ticked when it is not. For "Keep the tests green" the line SHALL say "It has no end of its own. Set a limit, or stop it yourself." For "My own text" the line SHALL say "The prompt below is submitted unchanged on every iteration."

Text the person wrote SHALL be kept when a template is chosen, and "My own text" SHALL bring it back. While a template has set such text aside, the dialog SHALL say that the text is kept, that "My own text" gets it back, and that editing the prompt replaces it. A template whose text the person edits SHALL become the person's own text. When nothing of the person's is kept, "My own text" SHALL give an empty prompt field, and a start with an empty prompt SHALL be refused as before.

Every built-in template SHALL be plain text within the bounds of a start, SHALL contain no slash command, SHALL describe one step for one iteration, and SHALL tell the agent to commit its step and not to push.

#### Scenario: The default template is started unchanged
- **WHEN** a person opens the start dialog on a tab with no earlier autopilot and confirms the start without changing anything
- **THEN** one start is sent whose prompt is the text of "Implement from a design document", with no iteration limit and no time limit

#### Scenario: Another template is chosen
- **WHEN** a person who has set a timed repeat and an iteration limit chooses "Work through TODO.md"
- **THEN** the prompt field holds that template's text
- **AND** the repeat timing, the limits and the no-tool-call option are as the person left them

#### Scenario: A template that ends by itself, with the option cleared
- **WHEN** "Implement from a design document" is selected and "End when an iteration makes no tool call" is not ticked
- **THEN** the line under the control says the run ends by itself only if that option is ticked
- **AND** the dialog does not tick the option

#### Scenario: A template that has no end of its own
- **WHEN** a person chooses "Keep the tests green"
- **THEN** the line under the control says "It has no end of its own. Set a limit, or stop it yourself."

#### Scenario: A template is edited
- **WHEN** a person changes one character of a template's text in the prompt field
- **THEN** the Template control reads "My own text"
- **AND** a start carries the edited text

#### Scenario: The person's own text is set aside and recovered
- **WHEN** a person types a prompt, then chooses a template, then chooses "My own text"
- **THEN** while the template is shown, the dialog says the person's text is kept
- **AND** after "My own text" the prompt field holds what the person typed

#### Scenario: Nothing of the person's is kept
- **WHEN** a person who has typed nothing chooses "My own text"
- **THEN** the prompt field is empty
- **AND** a start is refused until a prompt is entered

#### Scenario: A tab whose last autopilot ended
- **WHEN** a person opens the start dialog on a tab whose last autopilot ended with a prompt that is not a template's text
- **THEN** the prompt field holds that prompt and the Template control reads "My own text"
- **AND** choosing a template and then "My own text" brings that prompt back

#### Scenario: The last prompt was a template unchanged
- **WHEN** a person opens the start dialog on a tab whose last autopilot ended with the unchanged text of "Work through TODO.md"
- **THEN** the Template control reads "Work through TODO.md"

#### Scenario: The phone's sheet
- **WHEN** a person opens "Start autopilot" on a phone
- **THEN** the sheet has the same Template control above the prompt field, with the same entries and the same default

### Requirement: The start dialog keeps its actions in view

The start dialog, on the desktop and in the phone's sheet, SHALL keep Cancel and Start autopilot visible however far its content is scrolled: the content SHALL scroll and the two actions SHALL stay below it. The reason a start was refused SHALL be shown beside those actions, so that it cannot be scrolled out of sight.

#### Scenario: A window shorter than the dialog
- **WHEN** a person opens the start dialog in a window that is not tall enough to show all of it
- **THEN** the statement, the template, the prompt, the timing and the limits scroll
- **AND** Cancel and Start autopilot stay visible below them

#### Scenario: A start is refused
- **WHEN** a person presses Start autopilot with an empty prompt
- **THEN** the reason is shown beside the two actions, wherever the content is scrolled to

#### Scenario: The phone's sheet
- **WHEN** a person scrolls the start sheet on a phone
- **THEN** Cancel and Start autopilot stay at the bottom of the sheet
