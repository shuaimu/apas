## ADDED Requirements

### Requirement: Locating a Codex conversation does not change the user's Codex daemon

When the system queries the user's shared Codex app-server daemon to find which conversation a Codex tab is running, it SHALL introduce itself as a read-only client that Codex does not treat as originating, so that the daemon's process-wide client identity (the User-Agent of the person's own model requests, its originator and its login handling) stays as the person's own Codex set it. It SHALL request only what locating needs and SHALL NOT start, steer or interrupt any turn.

#### Scenario: A locator query between two typed turns
- **WHEN** the system locates a Codex tab's conversation through the shared daemon, and the person then types a message in that tab
- **THEN** the model request for that message carries the same User-Agent as before the query
