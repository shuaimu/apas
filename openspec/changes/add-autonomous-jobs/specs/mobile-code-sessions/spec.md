## MODIFIED Requirements

### Requirement: The mobile application provides a code-only APAS experience
The system SHALL provide supported iOS and Android clients whose primary navigation is limited to APAS coding projects, sessions, persistent autonomous workers, work requiring attention, earlier execution results, and account settings. Workers and Attention SHALL have discoverable entry points rather than notification-only routes. The mobile application SHALL NOT expose a general-purpose AI chat surface or imply that agents execute on the phone. Workers and interactive sessions SHALL retain distinct approval and execution semantics; earlier finite jobs SHALL not retain a separate creation product.

#### Scenario: User opens the mobile application
- **WHEN** an authenticated user launches the mobile application
- **THEN** the application opens a code-session home showing work relevant to that user
- **AND** exposes Workers and Attention navigation without a general chat inbox or consumer-chat composer

#### Scenario: User opens the application on a tablet
- **WHEN** the application runs on a supported tablet layout
- **THEN** it can show additional session or job navigation beside the active detail view
- **AND** preserves the same capabilities and authorization behavior as the phone layout

#### Scenario: User switches from a session to autonomous work
- **WHEN** the user opens Workers from primary navigation
- **THEN** the client displays authorized persistent workers independently of current session or pane selection
- **AND** the interactive session remains available without being converted into a job worker
