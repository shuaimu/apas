## ADDED Requirements

### Requirement: Mobile task launch never offers or accepts Pi
The mobile task-launch catalog SHALL NOT include `terminal:pi:official:default`, also when a stored or cached effective policy still lists that profile. A mobile task launch that names the Pi profile, or that resolves to Pi, SHALL be refused with "Pi is no longer supported. Use OMP instead.", whatever capabilities the connected project CLI advertises, and SHALL create no pane. Routing a mobile task SHALL NOT require or read a Pi terminal capability. An OMP mobile task SHALL still require a project CLI that advertises the current mobile task-launch capability and the OMP terminal capability.

#### Scenario: A policy still lists the Pi profile
- **WHEN** the server builds the mobile task-launch catalog from an effective policy that lists `terminal:pi:official:default`
- **THEN** the catalog contains no Pi profile
- **AND** the other allowed profiles are listed as before

#### Scenario: An older mobile build submits the Pi profile
- **WHEN** a mobile build that holds a profile list from before the retirement submits a launch with `terminal:pi:official:default`
- **THEN** the server rejects the launch with "Pi is no longer supported. Use OMP instead."
- **AND** no pane request is routed and the operation is not reported as successful

#### Scenario: A stale submission names another profile that is no longer allowed
- **WHEN** a mobile build submits a profile that is not retired and that the effective policy no longer permits
- **THEN** the server rejects the launch as no longer allowed, asking the user to choose another profile

#### Scenario: OMP mobile launch on an older project host
- **WHEN** an authorized mobile user selects OMP but the connected project CLI does not advertise OMP terminal capability
- **THEN** the server rejects the launch with an update-and-reconnect error, as before

## REMOVED Requirements

### Requirement: Mobile task launch supports capable Pi project hosts
**Reason**: Pi was retired as a provider on 2026-11-13. The Pi terminal capability `terminal_pi_v1` is deleted, so no project host can be "capable" of Pi.
**Migration**: Launch an OMP task instead. An older mobile build that still shows Pi is answered with "Pi is no longer supported. Use OMP instead."; see "Mobile task launch never offers or accepts Pi".
