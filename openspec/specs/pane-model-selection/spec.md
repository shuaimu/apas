# pane-model-selection Specification

## Purpose

Defines pane model selection as a launch-time decision and prevents direct model reconfiguration from an existing pane's Overview card.
## Requirements
### Requirement: Existing pane cards do not offer model selection
The web interface SHALL NOT render a dedicated model selector on an existing pane card, whatever the pane's provider, stored model value or retained `managed` marker. A terminal pane card SHALL show no agent, backend or model control. A retained structured pane card SHALL keep its combined agent frontend/API backend control, which SHALL name the combination the pane runs, including DeepSeek 4.1 Flash for a pane on the DeepSeek backend.

#### Scenario: Managed Claude pane has a specific model
- **WHEN** the Overview displays a Claude pane that still carries the retained `managed` marker and was launched with a specific model
- **THEN** the pane card does not display a model selector
- **AND** the pane's stored model remains unchanged

#### Scenario: Unmanaged Claude pane uses the default model
- **WHEN** the Overview displays a Claude pane using its default model
- **THEN** the pane card does not display a model selector

#### Scenario: Pane uses another provider
- **WHEN** the Overview displays a pane whose provider is not Claude
- **THEN** the pane card does not display a model selector

#### Scenario: Terminal pane
- **WHEN** the Overview displays a terminal pane
- **THEN** its card is labelled Terminal
- **AND** it shows no agent, backend or model control

#### Scenario: Retained structured pane uses the DeepSeek backend
- **WHEN** the Overview displays a retained structured Claude pane whose model is `deepseek-flash` or the older `deepseek-v4-flash`
- **THEN** its combined agent/backend control names Claude / DeepSeek 4.1 Flash

### Requirement: Pane-card interactions do not directly change the model
Displaying or interacting with an existing pane card SHALL NOT expose an action that directly replaces or clears that pane's stored model. The combined agent frontend/API backend control on a retained structured pane SHALL list another combination only when the effective project policy allows its structured launch profile. The supported catalog holds terminal profiles only, so under a known policy the control lists the current combination alone. The server SHALL reject a request to switch a structured pane to a combination the policy does not allow.

#### Scenario: User interacts with the pane card controls
- **WHEN** the user uses controls on an existing pane card without changing its agent frontend/API backend
- **THEN** no direct model-update action is sent
- **AND** the pane retains its launch model

#### Scenario: User uses the agent frontend/API backend selector
- **WHEN** the user opens the combined control on a retained structured pane under a known project policy
- **THEN** only the pane's current combination is listed

#### Scenario: Stale client requests a structured switch
- **WHEN** a client asks to switch a structured pane to another structured agent, backend or model combination
- **THEN** the server rejects the request as disabled by policy
- **AND** the pane keeps its provider and model

### Requirement: Model selection remains available before launch
Pane-creation interfaces SHALL continue to offer their supported provider and model choices before a new pane is launched.

#### Scenario: User creates a pane
- **WHEN** the user opens a supported pane-creation interface
- **THEN** the available launch-time provider and model choices remain selectable

### Requirement: New terminal panes may select DeepSeek as the Claude backend
The new-tab and mobile create-pane interfaces SHALL offer DeepSeek 4.1 Flash as a model choice of the Claude terminal. Creating such a pane SHALL send the claude frontend with the `deepseek-flash` model, spawn the Claude Code TUI with the DeepSeek environment overrides, and restore the same model on restart. The choice SHALL be offered only when the `terminal:claude:deepseek:deepseek-flash` launch profile is allowed by effective project policy.

#### Scenario: Policy permits the DeepSeek terminal profile
- **WHEN** the effective project policy includes `terminal:claude:deepseek:deepseek-flash`
- **THEN** the new-tab menu and mobile create-pane sheet offer Claude / DeepSeek 4.1 Flash
- **AND** creating it launches the Claude terminal with the DeepSeek backend environment

#### Scenario: Policy does not permit the DeepSeek terminal profile
- **WHEN** the effective project policy lacks `terminal:claude:deepseek:deepseek-flash`
- **THEN** the choice is not offered in any new-pane interface
- **AND** a stale request for it is rejected by policy enforcement

#### Scenario: A DeepSeek terminal pane restarts
- **WHEN** the project CLI restores an existing terminal pane carrying the DeepSeek Flash model
- **THEN** the pane relaunches with the same model's environment overrides

#### Scenario: DeepSeek backend is not configured
- **WHEN** a user creates a DeepSeek terminal pane and no DeepSeek API key is configured on the host
- **THEN** the pane reports the missing-key error and fails closed rather than launching Claude against its official backend

