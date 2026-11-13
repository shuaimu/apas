## ADDED Requirements

### Requirement: DeepSeek is one model reached through the Claude runtime
APAS SHALL support DeepSeek as one model, `deepseek-flash`, labelled DeepSeek 4.1 Flash. It SHALL be reached through the Claude runtime, one Anthropic-compatible DeepSeek endpoint and one machine-level DeepSeek API key. The generic DeepSeek choice SHALL resolve to that model. The older identifier `deepseek-v4-flash` SHALL be accepted and stored as `deepseek-flash`. Every Claude Code model slot, including the small, Sonnet, Opus and subagent slots, SHALL resolve to `deepseek-flash`. DeepSeek Pro (`deepseek-v4-pro`) is withdrawn. APAS SHALL NOT offer or launch it and SHALL NOT substitute another model for it. APAS SHALL validate the requested model before launch and SHALL NOT rely on the upstream API's fallback for unknown model names.

#### Scenario: User launches the default DeepSeek choice
- **WHEN** a user selects the Claude / DeepSeek choice without naming a model
- **THEN** APAS launches Claude Code with `deepseek-flash` as the primary model
- **AND** Claude Code's small and subagent model routing also uses `deepseek-flash`

#### Scenario: A pane stored under the older Flash identifier
- **WHEN** a pane whose stored model is `deepseek-v4-flash` is launched or restored
- **THEN** APAS launches it with `deepseek-flash` against the same endpoint and API key

#### Scenario: An existing pane uses DeepSeek Pro
- **WHEN** a pane whose stored model is `deepseek-v4-pro` is displayed or relaunched
- **THEN** the pane is shown as an unsupported provider and keeps its history
- **AND** the launch fails with a message that the model was withdrawn
- **AND** no other model is started in its place

#### Scenario: Requested DeepSeek model is unknown
- **WHEN** a stale or modified client requests an unknown DeepSeek model identifier
- **THEN** the server or project host rejects the launch or switch
- **AND** APAS does not silently run another model as a fallback
