## Purpose

Allow a machine owner to register an existing directory as an APAS project from the web without cloning a repository or requiring Git, while preserving files, project identity, and access boundaries.

## ADDED Requirements

### Requirement: New-project entry points offer a source choice

The desktop and mobile web new-project flows SHALL offer Clone from GitHub and Existing folder. Existing folder SHALL identify the selected APAS machine and accept a directory path on that machine without requiring clone URL, repository, or branch fields. Switching sources SHALL apply only the active source's validation and request fields. Existing GitHub creation behavior SHALL remain unchanged.

#### Scenario: Choose an existing folder
- **WHEN** a machine owner chooses Existing folder and selects an eligible machine
- **THEN** the form requests a host-local folder path and explains where it is resolved
- **AND** submission does not require GitHub or branch information

#### Scenario: Switch from clone to local registration
- **WHEN** a user switches from a clone form containing an invalid URL to Existing folder
- **THEN** the invalid clone URL neither blocks a valid local-path submission nor appears in its request

#### Scenario: Keep GitHub cloning available
- **WHEN** a user chooses Clone from GitHub
- **THEN** existing clone validation, eligible machine selection, shared-cluster restrictions, and creation behavior continue to apply

### Requirement: Existing directories are registered independently of repository hosting

The selected daemon SHALL accept an existing directory specified by an absolute path or a `~/` path relative to its operating-system user's home. It SHALL resolve the canonical directory on that machine. A GitHub origin, another Git origin, no origin, and no Git repository SHALL all be valid registration inputs. It SHALL NOT create a missing directory, clone, initialize Git, change Git configuration or branches, or modify source files. New project metadata SHALL be limited to APAS's registration needs, with zero configured panes.

#### Scenario: Register a GitHub checkout in place
- **WHEN** an owner registers an existing GitHub checkout
- **THEN** APAS uses that checkout's canonical path rather than creating another clone
- **AND** its tracked files, uncommitted changes, branch, and remotes remain unchanged

#### Scenario: Register a non-GitHub Git directory
- **WHEN** an owner registers a directory whose Git origin is another host or whose repository has no origin
- **THEN** registration succeeds without GitHub validation or a network clone

#### Scenario: Register an ordinary directory
- **WHEN** an owner registers an existing directory without Git metadata
- **THEN** APAS creates its project metadata and registration without creating `.git`
- **AND** existing files remain unchanged

#### Scenario: Resolve a home-relative path with spaces
- **WHEN** an owner submits `~/work/My Project`
- **THEN** the daemon resolves it under its own user's home and treats spaces literally without shell evaluation

### Requirement: Local-folder registration requires machine ownership

Only the active account owning the selected machine SHALL be allowed to request local-folder registration. The server SHALL enforce this independently of the interface and SHALL NOT dispatch an unauthorized path request to the daemon. Cluster membership and project membership alone SHALL NOT grant access to arbitrary existing directories. Existing project ownership, memberships, and policy SHALL NOT be replaced or widened by registration.

#### Scenario: A shared-cluster member supplies a local path
- **WHEN** a cluster member who does not own the machine submits a local-folder request, including a forged request
- **THEN** the server rejects it before filesystem inspection or mutation
- **AND** existing authorized shared GitHub cloning remains available

#### Scenario: An existing project is owned by another account
- **WHEN** a machine owner registers a directory containing an existing APAS identity
- **THEN** local registration does not transfer its canonical ownership, add project membership, or bypass later project-access checks

### Requirement: Existing project identity is preserved and registration is idempotent

A valid existing `.apas` file SHALL retain its identity, name, pane configuration, and project policy. Registering the same canonical directory again, including through a symlink alias or a repeated request, SHALL reuse the existing project and SHALL NOT create another registry entry or reset configuration. A conflicting association of one project identity with another canonical directory, or one directory with another registered identity, SHALL be reported rather than silently relocating or overwriting the registration.

#### Scenario: Add an existing APAS project
- **WHEN** a valid `.apas` file already exists in the selected directory
- **THEN** registration uses its current project identity and preserves its configuration

#### Scenario: Repeat registration through another path spelling
- **WHEN** a directory already registered on the selected machine is submitted again through its real path or a symlink alias
- **THEN** the operation reports success for the same project and canonical path without duplication

#### Scenario: Encounter malformed metadata or an identity collision
- **WHEN** `.apas` is invalid or its identity conflicts with an existing registry association
- **THEN** the operation reports an actionable error
- **AND** it does not replace metadata, reset panes, or change the conflicting registration

#### Scenario: Reject an unsafe metadata path
- **WHEN** the selected directory's `.apas` is a symbolic link or a non-regular file
- **THEN** registration fails without following it for creation or modifying its target
- **AND** directory symlink aliases remain supported through canonical directory resolution

### Requirement: Successful registration is ready for an explicit project start

The system SHALL report success only after both local registration and the server's canonical project identity, hosting placement, and effective policy are available to the existing Start authorization flow. Finalization SHALL apply current account, project-access, and lifecycle checks, preserve any existing owner and policy override, and SHALL NOT create a runtime session or start a project as a substitute for registration.

#### Scenario: Start a newly registered ordinary directory
- **WHEN** registration of a new non-Git directory completes successfully and the owner explicitly chooses Start
- **THEN** normal Start authorization recognizes the registered project and its hosting placement
- **AND** the resulting authorized workspace contains zero panes until the user creates one

#### Scenario: Server finalization is rejected
- **WHEN** local registration finishes but the server rejects the identity because of current authorization or project lifecycle state
- **THEN** the web reports failure rather than successful creation
- **AND** existing ownership and access remain unchanged and no local directory or retained metadata is deleted as compensation

### Requirement: Registration does not start a project or its agents

Successful local-folder registration SHALL make the project discoverable through the selected machine's ordinary project inventory. It SHALL NOT start a stopped project, create default panes, or launch providers. The web SHALL explain that starting the project is a separate operation through existing lifecycle controls and SHALL provide a success action leading to the selected owner's machine inventory. Registering a currently running project SHALL leave its runtime and panes undisturbed.

#### Scenario: Register a stopped project with saved panes
- **WHEN** a directory with saved pane configuration is registered
- **THEN** it appears as registered without starting those panes
- **AND** the user can explicitly start it through the existing project controls

#### Scenario: Register a running project again
- **WHEN** an owner registers a directory whose project is already running
- **THEN** the existing runtime remains running without restarting or adding providers

#### Scenario: Discover a folder added from the sidebar or mobile home
- **WHEN** local registration succeeds for a project that has no session yet
- **THEN** the completion view offers a way to open the selected machine's project inventory and its existing Start control
- **AND** it does not fabricate a session or start agents merely to insert a sidebar row

### Requirement: Completion and failures are correlated and recoverable

The web SHALL show a pending local-registration operation scoped to its request and target machine, then report confirmed success or an actionable failure. Blank or unsupported relative paths, missing paths, non-directories, unreadable or unwritable required metadata, offline daemons, and unsupported daemon versions SHALL not be presented as successful registration. Results from another machine or an unrelated request SHALL NOT complete the operation. Failure SHALL NOT delete existing directory contents or overwrite existing metadata; retry after a partial APAS-only write SHALL reuse that identity.

#### Scenario: Select an invalid directory
- **WHEN** a user submits a missing path, a regular file, or a directory whose required metadata cannot be accessed
- **THEN** the form reports the specific failure and permits correction without changing the user's existing files

#### Scenario: Target an older or disconnected daemon
- **WHEN** the selected daemon is offline or does not advertise local-folder registration support
- **THEN** the operation reports that it needs a connected, updated daemon
- **AND** it is not reinterpreted as a clone request

#### Scenario: Recover from an interrupted registration
- **WHEN** metadata was created but the result was lost or registry persistence failed, and the owner retries
- **THEN** APAS reuses the existing identity and returns the confirmed canonical registration without deleting the directory
