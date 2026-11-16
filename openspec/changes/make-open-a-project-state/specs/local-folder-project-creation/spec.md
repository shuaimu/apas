## MODIFIED Requirements

### Requirement: Successful registration is ready for an explicit project start

The system SHALL report success only after both local registration and the server's canonical project identity, hosting placement, and effective policy are available to the authorization that opening the project applies. Finalization SHALL apply current account, project-access, and lifecycle checks, preserve any existing owner and policy override, and SHALL NOT create a runtime session or start a project as a substitute for registration.

#### Scenario: Start a newly registered ordinary directory
- **WHEN** registration of a new non-Git directory completes successfully and the owner explicitly opens the project
- **THEN** the authorization of an open recognizes the registered project and its hosting placement
- **AND** the resulting authorized workspace contains zero panes until the user creates one

#### Scenario: Server finalization is rejected
- **WHEN** local registration finishes but the server rejects the identity because of current authorization or project lifecycle state
- **THEN** the web reports failure rather than successful creation
- **AND** existing ownership and access remain unchanged and no local directory or retained metadata is deleted as compensation

### Requirement: Registration does not start a project or its agents

Successful local-folder registration SHALL make the project discoverable through the selected machine's ordinary project inventory. It SHALL NOT open the project, start it on any machine, create default panes, or launch providers. The web SHALL explain that opening the project is a separate operation and SHALL provide a success action leading to the selected owner's machine inventory. Registering a currently running project SHALL leave its runtime and panes undisturbed.

#### Scenario: Register a stopped project with saved panes
- **WHEN** a directory with saved pane configuration is registered
- **THEN** it appears as registered without starting those panes
- **AND** the user can open it explicitly, which starts those panes

#### Scenario: Register a running project again
- **WHEN** an owner registers a directory whose project is already running
- **THEN** the existing runtime remains running without restarting or adding providers

#### Scenario: Discover a folder added from the sidebar or mobile home
- **WHEN** local registration succeeds for a project that has no session yet
- **THEN** the project appears in desktop sidebar/rail and mobile All projects as registered and closed
- **AND** the entry and completion view offer navigation to the owning machine inventory, where the project can be opened
- **AND** no session is fabricated and no project or agent is started merely to display the entry

#### Scenario: Deduplicate registration and later session reports
- **WHEN** several owned machines report the same stable project ID, or a real session later arrives for that ID
- **THEN** machine registration contributes at most one project-list entry
- **AND** a real session replaces the sessionless representation while retaining its normal attachment and pane-status behavior

#### Scenario: Shared compute alone does not add sessionless projects
- **WHEN** a shared-cluster member receives machine inventory without a corresponding authorized project session
- **THEN** that inventory alone does not create a project entry or a session attachment target
