## ADDED Requirements

### Requirement: Mobile presents machines beside coding sessions

The mobile browser home SHALL let a user move between their coding sessions and the machines those sessions run on, without leaving it. The native application does not list machines. Machines SHALL be reachable in the same way the session filters are.

#### Scenario: A user switches to machines

- **WHEN** a user selects machines on the mobile browser home
- **THEN** the machines they can reach are listed in place of the session list

#### Scenario: A user switches back to sessions

- **WHEN** a user selects a session filter again
- **THEN** their coding sessions are listed as before
