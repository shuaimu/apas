## MODIFIED Requirements

### Requirement: The mobile application provides a code-only APAS experience
The system SHALL provide supported iOS and Android clients whose primary navigation is limited to APAS coding projects, sessions, autonomous jobs, work requiring attention, and account settings. Jobs and Attention SHALL have discoverable navigation entry points rather than relying on notification-only routes. The mobile application SHALL NOT expose a general-purpose AI chat surface or imply that coding agents execute locally on the phone. Autonomous jobs and interactive sessions SHALL retain distinct creation and execution semantics.

#### Scenario: User opens the mobile application
- **WHEN** an authenticated user launches the mobile application
- **THEN** the application opens a code-session home showing work relevant to that user
- **AND** exposes Jobs and Attention navigation without presenting a general chat inbox or consumer-chat composer

#### Scenario: User opens the application on a tablet
- **WHEN** the application runs on a supported tablet layout
- **THEN** it can show additional session or job navigation beside the active detail view
- **AND** preserves the same capabilities and authorization behavior as the phone layout

#### Scenario: User switches from a session to autonomous work
- **WHEN** the user opens Jobs from the primary navigation
- **THEN** the client displays authorized durable jobs independently of the current session or pane selection
- **AND** the interactive session remains available without being converted into a job worker
