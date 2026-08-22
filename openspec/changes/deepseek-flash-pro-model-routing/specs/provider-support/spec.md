## ADDED Requirements

### Requirement: DeepSeek Pro and Flash share one Claude backend configuration
APAS SHALL support `deepseek-v4-pro` and `deepseek-v4-flash` as distinct primary models reached through one Claude runtime, one Anthropic-compatible DeepSeek endpoint, and one machine-level DeepSeek API key. Pro SHALL remain the primary model selected by the generic or existing DeepSeek choice. For either primary model, Claude Code's small/Haiku and subagent model routing SHALL use Flash. APAS SHALL validate the requested model before launch and SHALL NOT rely on the upstream API's fallback for unknown model names.

#### Scenario: User launches the default DeepSeek choice
- **WHEN** a user selects the generic Claude/DeepSeek choice without selecting a model variant
- **THEN** APAS launches Claude Code with DeepSeek Pro as the primary model
- **AND** configures Claude Code's small and subagent model routing to use DeepSeek Flash

#### Scenario: User launches DeepSeek Flash
- **WHEN** a user selects DeepSeek Flash and the effective project policy permits its launch profile
- **THEN** APAS launches Claude Code against the existing DeepSeek endpoint and API key with Flash as the primary model
- **AND** does not create or require a second machine credential profile

#### Scenario: Pro pane delegates small work
- **WHEN** Claude Code running a Pro-primary DeepSeek pane selects its small/Haiku model or starts a Claude Code subagent
- **THEN** that request is routed to DeepSeek Flash
- **AND** the pane's primary model remains Pro

#### Scenario: Requested DeepSeek variant is unsupported
- **WHEN** a stale or modified client requests an unknown DeepSeek model identifier
- **THEN** the server or project host rejects the launch or switch
- **AND** APAS does not silently run Flash or Pro as a fallback
