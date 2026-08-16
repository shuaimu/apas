## MODIFIED Requirements

### Requirement: Model and provider policy is enforced at launch
The effective project policy SHALL identify which supported agent frontend, API backend, model, and terminal combinations may be launched. Web interfaces SHALL offer only allowed combinations, and the server and project host SHALL independently reject disallowed requests to create a pane or to change an existing pane's combination.

The allowlist SHALL govern which combinations may be brought into existence, not which existing panes may run. Resuming, rebooting, or restarting an existing pane SHALL NOT be refused because its combination is no longer allowed, and such a pane SHALL NOT prevent the project host from being restarted. Restrictions that are not the allowlist are unaffected: a retired backend SHALL still refuse to launch, and managed team panes SHALL still require team mode to be available.

#### Scenario: User launches an allowed model
- **WHEN** a project member selects a combination permitted by the effective project policy
- **THEN** the system allows the pane or team member to launch

#### Scenario: Stale client requests a disallowed model
- **WHEN** a client requests to create a pane with a model/provider combination that the current effective policy disallows
- **THEN** the server or project host rejects the request with a policy-specific error
- **AND** no disallowed process is launched

#### Scenario: Policy changes while a model is running
- **WHEN** an administrator disallows a combination already used by a running pane
- **THEN** the system prevents new panes and backend switches to that combination
- **AND** reports the existing pane as policy-noncompliant without silently terminating it

#### Scenario: Relaunching a pane whose combination is no longer allowed
- **WHEN** a member resumes, reboots, or restarts an existing pane whose combination the current policy disallows
- **THEN** the pane relaunches
- **AND** the request is not refused on account of the allowlist

#### Scenario: Switching an existing pane to a disallowed combination
- **WHEN** a member changes an existing pane's model or provider to a combination the current policy disallows
- **THEN** the request is rejected
- **AND** the pane keeps the combination it had

#### Scenario: A noncompliant pane does not block the project host
- **WHEN** a project host restart is requested for a project containing a pane whose combination is no longer allowed
- **THEN** the restart proceeds

#### Scenario: Retired backends are still refused
- **WHEN** a member resumes or reboots an existing pane whose backend has been retired
- **THEN** the request is still refused

#### Scenario: Managed panes still require team mode
- **WHEN** a member resumes or reboots an existing managed team pane while team mode is unavailable
- **THEN** the request is still refused

#### Scenario: Noncompliance is reported without implying a restriction
- **WHEN** a project contains panes outside the current allowlist
- **THEN** the system may identify them
- **AND** SHALL NOT state that they cannot be relaunched
