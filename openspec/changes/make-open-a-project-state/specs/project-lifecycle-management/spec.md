## MODIFIED Requirements

### Requirement: The web interface exposes role-appropriate lifecycle controls
The web interface SHALL expose ownership transfer and permanent deletion to the current owner and project departure to ordinary users, with clear role consequences and confirmation before each mutation. Permanent deletion SHALL be a control of its own, named "Delete project…", offered to the owner beside closing in the project's workspace, in the cluster page's panel for that project, and from project access; it SHALL NOT be reachable only from project access, and closing a project SHALL never lead to it. All of these SHALL open one deletion dialog, which SHALL say before anything is sent: that the project is erased from the APAS server with its tabs, history, usage, sharing, invitations and policy, and that this cannot be undone; that the folder and its files stay on every machine and that each such machine keeps the folder registered, so that it stays listed as a registered folder and opening it there creates a new, empty project; and which machines' agents are stopped first, with an offline machine that owns tabs named as one the deletion cannot stop. The dialog SHALL require the user to type the project's name, SHALL send the request once, SHALL show that the deletion is in progress until the server reports it complete, and SHALL claim nothing for a request that was not sent. After a successful operation, every affected client SHALL refresh project roles and accessible-project state so stale owner or member controls are not retained.

#### Scenario: Owner reviews lifecycle actions
- **WHEN** an owner opens project access or project settings
- **THEN** the interface offers transfer only to current eligible project users
- **AND** explains that the owner will become an ordinary user
- **AND** presents project deletion as a distinct irreversible action with project-specific confirmation

#### Scenario: Ordinary user reviews lifecycle actions
- **WHEN** an ordinary project user opens project actions
- **THEN** the interface offers leaving the project
- **AND** does not offer ownership transfer or project deletion

#### Scenario: Roles change in another client
- **WHEN** ownership transfer, member departure, or deletion succeeds in one client while another client has the project open
- **THEN** the other client receives or obtains refreshed authorization state before its next privileged mutation
- **AND** removes controls and content that its current role no longer permits

#### Scenario: Delete project from the workspace
- **WHEN** the owner chooses "Delete project…" in the workspace of a project that runs on zoo-005
- **THEN** the dialog says what is erased, that zoo-005's agents are stopped first, and that the folder stays on zoo-005 and stays listed there as a registered folder
- **AND** nothing is sent until the owner has typed the project's name and confirmed

#### Scenario: Delete project from the cluster page
- **WHEN** the owner selects one of their projects in the cluster page's project list
- **THEN** its panel offers "Delete project…", which opens the same dialog

#### Scenario: The typed name does not match
- **WHEN** the owner has typed something other than the project's name
- **THEN** the dialog's delete button stays disabled and nothing is sent

#### Scenario: A deletion in progress
- **WHEN** the owner has confirmed and the server has accepted the deletion
- **THEN** the dialog says the deletion is in progress until the server reports it complete
- **AND** the request is not sent again

#### Scenario: A tab owner is offline
- **WHEN** the owner opens the deletion dialog for a project of which an offline laptop owns tabs
- **THEN** the dialog says the laptop cannot be stopped now and that the project comes back from it if it reconnects still running the project
