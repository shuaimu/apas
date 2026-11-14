## ADDED Requirements

### Requirement: Project Overview presents persistent workers independently of panes
The project Overview SHALL show a project-authorized bounded inventory of persistent workers, including active, idle, waiting, blocked, paused, and hidden workers, with full expandable descriptions, current work, canonical status, host, update time, and next check when applicable. It SHALL offer access to backlog, standing checklist, execution history, and retained earlier job/bot results. No pane, selected execution, aggregate activity flag, or first global listing page SHALL substitute for the project's worker inventory. Existing interactive project panels SHALL remain available as separate sections, which come before the worker inventory on the desktop and in the mobile browser.

#### Scenario: A project has no panes but has an idle worker
- **WHEN** the user opens that project's Overview
- **THEN** the worker and its next check are visible and controllable under current authority
- **AND** the empty pane roster remains empty rather than manufacturing a worker pane

#### Scenario: The worker inventory is paginated
- **WHEN** more workers or historical results exist than the first page contains
- **THEN** the user can explicitly load the remaining authorized pages
- **AND** the UI does not label the loaded subset as the complete project inventory

### Requirement: Worker navigation preserves terminal and project identity
Desktop and mobile-browser project workspaces SHALL expose Overview before typed pane and worker views. The new-tab control (`+`) SHALL create an interactive agent only. Autonomous worker creation SHALL be offered from the project Overview and from the cross-project Workers page, not from the new-tab control. Selecting a worker, checklist, question, or result SHALL preserve the selected project and leave mounted interactive terminals and their scroll/input state intact. Worker identifiers SHALL not be encoded as pane numbers or session identifiers. The selected pane's machine SHALL only be an initial eligible creation hint, never authority to retarget an approved worker.

#### Scenario: A person switches from a terminal to a worker and back
- **WHEN** the person visits Overview, the worker's backlog, and an older result
- **THEN** the existing terminal instance remains mounted and the project remains selected
- **AND** navigation emits no pane add/remove/reboot/input or worker lifecycle command

#### Scenario: A person opens the new-tab control
- **WHEN** the person presses `+` in a project workspace
- **THEN** interactive providers are offered at once, with no choice between an interactive agent and an autonomous worker
- **AND** worker creation remains available from the Overview and the Workers page

#### Scenario: A different machine's pane is selected
- **WHEN** the worker view is reopened from that same project
- **THEN** the worker retains its approved target and identity
- **AND** neither its history/cache scope nor its scheduling placement follows the selected pane

### Requirement: Workspace preference migration does not authorize execution
Account/project-scoped opened/hidden view preferences and legacy Overview selection SHALL migrate into the typed worker/history workspace without overriding explicit navigation. A saved job view SHALL resolve to its exact earlier result rather than an invented worker. Restored cached labels SHALL remain visibly stale until authorized synchronization, and revocation SHALL remove hidden/opened protected labels as well as detail content. Preference restoration SHALL never approve or resume a worker.

#### Scenario: A user selects a pane before preferences finish loading
- **WHEN** a late worker/Overview preference becomes available
- **THEN** the explicit pane selection remains foreground
- **AND** restoring the preference does not change worker lifecycle or attach a different session
