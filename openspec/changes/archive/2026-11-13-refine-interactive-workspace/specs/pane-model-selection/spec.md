## MODIFIED Requirements

### Requirement: Existing pane cards do not offer model selection
The web interface SHALL NOT render a dedicated model selector on an existing pane card, whatever the pane's provider, stored model value or retained `managed` marker. A terminal pane card SHALL name what the tab runs, by the provider catalog's name for it, as a label, and SHALL show no agent, backend or model control. A retained structured pane card SHALL keep its combined agent frontend/API backend control, which SHALL name the combination the pane runs, including DeepSeek 4.1 Flash for a pane on the DeepSeek backend.

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
- **WHEN** the Overview displays an OMP terminal pane and a Claude terminal pane on the DeepSeek backend
- **THEN** their cards are labelled "OMP" and "Claude / DeepSeek 4.1 Flash"
- **AND** they show no agent, backend or model control

#### Scenario: Retained structured pane uses the DeepSeek backend
- **WHEN** the Overview displays a retained structured Claude pane whose model is `deepseek-flash` or the older `deepseek-v4-flash`
- **THEN** its combined agent/backend control names Claude / DeepSeek 4.1 Flash
