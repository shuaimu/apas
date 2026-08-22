## ADDED Requirements

### Requirement: DeepSeek model variants are independently governed
The supported launch-profile catalog SHALL identify DeepSeek Pro as `agent:claude:deepseek:deepseek-v4-pro` and DeepSeek Flash as `agent:claude:deepseek:deepseek-v4-flash`. Effective project policy SHALL authorize each variant independently even though both share one backend credential. New deployment policy state SHALL include both supported variants, while an existing persisted allowlist SHALL only gain Flash through an explicit authorized policy update rather than an automatic widening migration.

#### Scenario: Administrator reviews DeepSeek capabilities
- **WHEN** an administrator opens a launch-policy editor
- **THEN** Pro and Flash appear as separate allowlist capabilities under Claude/DeepSeek
- **AND** neither capability asks for a separate API key

#### Scenario: Policy permits only Pro
- **WHEN** the effective project policy contains the Pro launch profile but not the Flash launch profile
- **THEN** users may launch or switch to Pro
- **AND** Flash is not offered and a stale Flash request is rejected

#### Scenario: Policy permits only Flash
- **WHEN** the effective project policy contains the Flash launch profile but not the Pro launch profile
- **THEN** users may launch or switch to Flash
- **AND** Pro is not offered and a stale Pro request is rejected

#### Scenario: Existing allowlist predates Flash support
- **WHEN** the server upgrades a persisted explicit allowlist that contains Pro but not Flash
- **THEN** the upgrade does not silently add Flash to that allowlist
- **AND** an authorized administrator can explicitly add Flash through the normal policy update path

### Requirement: DeepSeek terminal panes are governed by their own launch profiles
The supported launch-profile catalog SHALL identify the Claude-hosted DeepSeek terminal choices as `terminal:claude:deepseek:deepseek-v4-pro` and `terminal:claude:deepseek:deepseek-v4-flash`. Effective project policy SHALL authorize each terminal variant independently of the structured-pane variants that share the same model identity.

#### Scenario: Policy permits the DeepSeek terminal profile
- **WHEN** the effective project policy contains a DeepSeek terminal launch profile
- **THEN** new-pane interfaces offer that terminal variant
- **AND** a launch request for it is authorized

#### Scenario: Policy permits only the structured variant
- **WHEN** the effective project policy contains an agent DeepSeek profile but not the matching terminal profile
- **THEN** new-pane interfaces do not offer the terminal variant
- **AND** a terminal launch request for it is rejected
