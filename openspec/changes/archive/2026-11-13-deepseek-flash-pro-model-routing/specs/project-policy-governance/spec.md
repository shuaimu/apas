## ADDED Requirements

### Requirement: DeepSeek is governed by one terminal launch profile
The supported launch-profile catalog SHALL identify DeepSeek as the single profile `terminal:claude:deepseek:deepseek-flash`. Effective project policy SHALL authorize it independently of the official Claude terminal profile, although both use the Claude runtime. When the server normalizes stored policy, an allowlist entry `terminal:claude:deepseek:deepseek-v4-flash` or `agent:claude:deepseek:deepseek-v4-flash` SHALL be carried forward to the current profile. A stored DeepSeek Pro profile SHALL be removed and SHALL NOT be replaced by another profile.

#### Scenario: Administrator reviews DeepSeek capabilities
- **WHEN** an administrator opens a launch-policy editor
- **THEN** DeepSeek 4.1 Flash Terminal appears as one allowlist capability
- **AND** it does not ask for a separate API key

#### Scenario: Policy permits the DeepSeek profile
- **WHEN** the effective project policy contains `terminal:claude:deepseek:deepseek-flash`
- **THEN** new-pane interfaces offer the DeepSeek terminal choice
- **AND** a launch request for it is authorized

#### Scenario: Policy lacks the DeepSeek profile
- **WHEN** the effective project policy contains the official Claude terminal profile but not the DeepSeek profile
- **THEN** the DeepSeek choice is not offered
- **AND** a stale request for it is rejected

#### Scenario: Stored allowlist names the older Flash key
- **WHEN** the server normalizes a stored allowlist that contains `terminal:claude:deepseek:deepseek-v4-flash` or `agent:claude:deepseek:deepseek-v4-flash`
- **THEN** the allowlist contains `terminal:claude:deepseek:deepseek-flash` in its place

#### Scenario: Stored allowlist names only DeepSeek Pro
- **WHEN** the server normalizes a stored allowlist whose only entry is a DeepSeek Pro profile
- **THEN** the result is an explicit empty allowlist
- **AND** no other profile is granted in its place
