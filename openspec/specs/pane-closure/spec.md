# pane-closure Specification

## Purpose

Defines what closing a pane means for the process behind it. Closure is the one
operation that is unambiguously a termination: every other way a pane can stop
being reachable — transport loss, CLI replacement, project suspension — is
either recoverable or bounded, and those are specified elsewhere. This
capability exists so that "the tab is gone" and "the agent is gone" cannot drift
apart, for any pane kind.

## Requirements

### Requirement: Closing a pane terminates the process behind it

Closing a pane SHALL terminate that pane's provider process and its
descendants, regardless of pane kind, provider, or how the process was hosted.
Closure SHALL be treated as a termination rather than a detach: it SHALL NOT
enter an adoption grace period, and it SHALL leave no runtime that a later CLI
can adopt.

Termination SHALL target the pane's process group rather than only its
immediate child, because interactive providers spawn shells, language servers,
and tool subprocesses that would otherwise survive their parent. The system
SHALL escalate from a graceful signal to an unconditional one after a bounded
delay, and SHALL reap the terminated process so no zombie remains.

The system SHALL NOT signal a process group it does not own.

#### Scenario: Closing a structured agent pane

- **WHEN** a user closes a pane whose provider runs as a CLI-owned child process
- **THEN** the pane's process group receives a graceful termination signal, an
  unconditional one after the grace period elapses, and is reaped

#### Scenario: Closing a terminal pane owned by the CLI's own PTY

- **WHEN** a user closes a terminal pane whose provider is attached to a PTY the
  CLI allocated
- **THEN** the provider process is killed and reaped, and the PTY reader stops
  without reporting the resulting end-of-file as a crash

#### Scenario: Closing a terminal pane backed by a persistent runtime

- **WHEN** a user closes a terminal pane whose provider is owned by a persistent
  runtime rather than by the project CLI
- **THEN** the runtime terminates the provider process tree, its hosting session
  is removed, and its local runtime state is deleted
- **AND** no adoption grace period applies, because the close was an explicit
  instruction rather than an unexpected loss of the controller

#### Scenario: A pane the CLI does not own is left alone

- **WHEN** the recorded process is not the leader of its own process group
- **THEN** the system declines to signal that group rather than risk signalling
  the CLI's own

### Requirement: A closed pane does not come back

Closing a pane SHALL remove it from the project's persisted pane list, so that a
later CLI start neither restores the pane nor launches a replacement provider
for it. The server SHALL discard any presentation retained for the closed pane
rather than serving it to a client that attaches afterwards.

#### Scenario: Project CLI restarts after a pane was closed

- **WHEN** a project CLI starts after a pane was closed
- **THEN** the closed pane is absent from the restored panes and no provider
  process is started on its behalf

#### Scenario: Client attaches after a terminal pane was closed

- **WHEN** a client attaches to a session in which a terminal pane was closed
- **THEN** the server does not serve retained output or lifecycle for that pane

### Requirement: Closure cleans up pane-scoped resources it owns

Closing a pane SHALL release the resources the pane alone held: its input
channel, its pause and stop state, and its entry in the project's live pane
metadata. Closure SHALL NOT discard resources that outlive a pane unless the
caller explicitly asked for it — in particular, a pane's isolated worktree SHALL
be removed only when the close request carries a cleanup instruction, so that
unmerged work is never destroyed by closing its tab.

#### Scenario: Close without a cleanup instruction

- **WHEN** a pane holding an isolated worktree is closed with no cleanup action
- **THEN** the provider process is terminated and the worktree is left on disk

#### Scenario: Close with an explicit cleanup instruction

- **WHEN** a pane holding an isolated worktree is closed with a cleanup action
- **THEN** the worktree is cleaned up according to that action

### Requirement: Closure is initiated from any surface with the same effect

Pane closure SHALL have the same process consequences whether it was requested
from the web workspace or performed locally on the machine running the project
CLI. A surface SHALL NOT be able to remove a pane from view while leaving its
provider process running.

#### Scenario: Pane closed from the web

- **WHEN** a user closes a pane from the web workspace
- **THEN** the project CLI performs the same termination and cleanup it performs
  for a locally closed pane
