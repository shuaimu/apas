## MODIFIED Requirements

### Requirement: Existing pane cards do not offer generic model selection
The web interface SHALL NOT render a dedicated generic model selector on an existing pane card, regardless of whether the pane is managed or unmanaged, its provider, or its stored model value. The existing combined agent frontend/API backend control SHALL, however, present policy-allowed DeepSeek Pro and Flash variants beneath the single Claude/DeepSeek backend for structured DeepSeek panes.

#### Scenario: Managed Claude pane has a specific official model
- **WHEN** the Overview displays a managed Claude pane using the official Anthropic backend with a specific model
- **THEN** the pane card does not display a model selector
- **AND** the pane's stored model remains unchanged

#### Scenario: Unmanaged Claude pane uses the default model
- **WHEN** the Overview displays an unmanaged Claude pane using its default model
- **THEN** the pane card does not display a model selector

#### Scenario: Pane uses another provider
- **WHEN** the Overview displays a pane whose provider is not Claude/DeepSeek
- **THEN** the pane card does not display a model selector

#### Scenario: Structured pane uses the DeepSeek backend
- **WHEN** the Overview displays an existing structured Claude/DeepSeek pane
- **THEN** its combined agent/backend control identifies the active Pro or Flash variant
- **AND** offers only the DeepSeek variants allowed by effective project policy

### Requirement: Pane-card interactions only change a model through a supported combined choice
Displaying or interacting with an existing pane card SHALL NOT expose an action that directly replaces or clears that pane's stored model outside the combined agent frontend/API backend/model control. Other pane-card actions SHALL retain their existing behavior. Selecting a different allowed DeepSeek variant in that combined control SHALL persist the requested model and restart the structured provider process with a fresh provider session.

#### Scenario: User interacts with the pane card controls
- **WHEN** the user uses controls on an existing pane card without changing its combined agent/backend/model choice
- **THEN** no direct model-update action is sent
- **AND** the pane retains its launch model

#### Scenario: User changes agent frontend or API backend
- **WHEN** the user selects a different supported agent frontend or API backend on an existing pane
- **THEN** the existing agent/backend switching behavior is preserved

#### Scenario: User changes a DeepSeek model variant
- **WHEN** the user changes an existing structured DeepSeek pane from Pro to Flash or from Flash to Pro
- **THEN** APAS warns that the current turn will be interrupted and the provider process will start with fresh prompt context
- **AND** after confirmation persists the new model, creates a fresh provider session, and retains the visible APAS transcript

### Requirement: DeepSeek variants remain available in retained structured-pane controls
The retained structured-pane Overview control and launch-policy catalog SHALL expose supported DeepSeek model choices. DeepSeek Pro and Flash SHALL appear as model variants of one Claude/DeepSeek backend rather than as separate credential or provider profiles. This capability SHALL NOT restore creation of structured agent panes; new user-created work remains terminal-only.

#### Scenario: User reviews a historical structured pane
- **WHEN** the user opens the Overview for an existing structured pane
- **THEN** the applicable combined agent/backend/model choices remain selectable

#### Scenario: User creates new work
- **WHEN** the user opens a supported pane-creation interface
- **THEN** APAS continues to offer only supported terminal-pane providers
- **AND** does not add DeepSeek as a standalone terminal provider

#### Scenario: User reviews DeepSeek launch choices
- **WHEN** both DeepSeek launch profiles are allowed by effective project policy
- **THEN** the interface presents Pro and Flash under one Claude/DeepSeek backend
- **AND** the generic/default DeepSeek choice resolves to Pro

## ADDED Requirements

### Requirement: New terminal panes may select DeepSeek as the Claude backend
The new-tab and mobile create-pane interfaces SHALL offer DeepSeek Pro and Flash as model variants of the Claude terminal beneath the single Claude/DeepSeek backend. Creating such a pane SHALL send the claude frontend with the deepseek model, spawn the Claude Code TUI with the DeepSeek environment overrides, and restore the same model on restart. Each variant SHALL be offered only when its own `terminal:claude:deepseek:*` launch profile is allowed by effective project policy.

#### Scenario: Policy permits a DeepSeek terminal variant
- **WHEN** the effective project policy includes `terminal:claude:deepseek:deepseek-v4-pro` or `terminal:claude:deepseek:deepseek-v4-flash`
- **THEN** the new-tab menu and mobile create-pane sheet offer that variant as a Claude terminal choice
- **AND** creating it launches the Claude terminal with the matching DeepSeek backend environment

#### Scenario: Policy does not permit a DeepSeek terminal variant
- **WHEN** the effective project policy lacks a DeepSeek terminal profile
- **THEN** that variant is not offered in any new-pane interface
- **AND** a stale request for it is rejected by policy enforcement

#### Scenario: A DeepSeek terminal pane restarts
- **WHEN** the project CLI restores an existing terminal pane carrying a DeepSeek model
- **THEN** the pane relaunches with the same model's environment overrides

#### Scenario: DeepSeek backend is not configured
- **WHEN** a user creates a DeepSeek terminal pane and no DeepSeek API key is configured on the host
- **THEN** the pane reports the missing-key error and fails closed rather than launching Claude against its official backend
