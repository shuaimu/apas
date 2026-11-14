## Purpose
Lets a person start an agent on Codex, Claude or OpenCode terminal tabs working in a loop by itself, with one click and a clear statement that APAS only sends the prompt and does not run or watch the loop.

## ADDED Requirements

### Requirement: Codex, Claude and OpenCode terminal tabs offer a loop prompt

For a terminal tab whose provider is Codex, Claude (including the DeepSeek profile, which runs the Claude terminal) or OpenCode, the system SHALL offer a Loop prompt action to anyone who may send input to the tab, in the pane toolbar where OMP tabs show the Autopilot control and among the phone's pane actions. It SHALL NOT be offered on OMP tabs, on non-terminal panes, on read-only legacy bot history, or on a tab that does not accept input. Its dialog SHALL offer the same built-in templates and the project's saved prompts as the autopilot start dialog, with the same rules for choosing them and for keeping the person's own text, and SHALL start on the project's default saved prompt when there is one and otherwise on the default template.

#### Scenario: A Codex tab
- **WHEN** a person who may type into a Codex terminal tab opens its toolbar
- **THEN** a Loop prompt control is shown where an OMP tab shows Autopilot

#### Scenario: An OMP tab
- **WHEN** an OMP terminal tab is selected
- **THEN** it shows the Autopilot control and no Loop prompt

### Requirement: A loop prompt is presented as a shortcut that APAS does not watch

The dialog SHALL say, before anything can be sent, that this is a shortcut that sends one prompt asking the agent to keep working in a loop by itself; that APAS does not run or watch that loop and cannot count its rounds, enforce a limit, pause, resume or stop it, or tell whether the agent is still looping; that the agent keeps the tab's permissions on its machine; and that the loop is stopped in the terminal. It SHALL show the loop instruction APAS adds before the prompt. A sent loop prompt SHALL create no autopilot state, marker, status or list entry, and SHALL NOT change how the tab is shown.

#### Scenario: Reading the dialog
- **WHEN** a person opens Loop prompt on a Claude terminal tab
- **THEN** the dialog states that APAS only sends the prompt and cannot monitor, limit, pause or stop the loop, and that it is stopped in the terminal, and it shows the loop instruction

### Requirement: A loop prompt is sent once, as an ordinary message

Sending SHALL put the loop instruction followed by the prompt into the tab as one message, through the same input and confirmation path as a message typed in the Conversation view: it is shown as confirmed only once the provider has recorded it, and as not confirmed otherwise. It SHALL be sent at most once per click and SHALL never be resent automatically. The loop instruction SHALL tell the agent to repeat the prompt's instructions round after round without waiting or asking, to treat an instruction to stop after one step as the end of a round, to read the files again each round, and to stop only when a round ends with the sentence "Nothing is left to do." or when it is interrupted.

#### Scenario: Sending
- **WHEN** a person sends a loop prompt with the default template on an idle Codex tab
- **THEN** one message holding the loop instruction and the template's text is sent to the tab, and the dialog reports it sent once the transcript records it

#### Scenario: Not confirmed
- **WHEN** the tab's transcript does not record the message
- **THEN** the dialog reports it as not confirmed and does not send it again
