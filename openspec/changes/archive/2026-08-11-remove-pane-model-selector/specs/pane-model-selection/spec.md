## Purpose

Defines pane model selection as a launch-time decision and prevents direct model reconfiguration from an existing pane's Overview card.

## ADDED Requirements

### Requirement: Existing pane cards do not offer model selection
The web interface SHALL NOT render a dedicated model selector on an existing pane card, regardless of whether the pane is managed or unmanaged, its provider, or its stored model value.

#### Scenario: Managed Claude pane has a specific model
- **WHEN** the Overview displays a managed Claude pane that was launched with a specific model
- **THEN** the pane card does not display a model selector
- **AND** the pane's stored model remains unchanged

#### Scenario: Unmanaged Claude pane uses the default model
- **WHEN** the Overview displays an unmanaged Claude pane using its default model
- **THEN** the pane card does not display a model selector

#### Scenario: Pane uses another provider
- **WHEN** the Overview displays a pane whose provider is not Claude
- **THEN** the pane card does not display a model selector

### Requirement: Pane-card interactions do not directly change the model
Displaying or interacting with an existing pane card SHALL NOT expose an action that directly replaces or clears that pane's stored model. Other pane-card actions, including the separate agent frontend/API backend selector, SHALL retain their existing behavior.

#### Scenario: User interacts with the pane card controls
- **WHEN** the user uses controls on an existing pane card without changing its agent frontend/API backend
- **THEN** no direct model-update action is sent
- **AND** the pane retains its launch model

#### Scenario: User uses the agent frontend/API backend selector
- **WHEN** the user selects a different supported agent frontend or API backend on an existing pane
- **THEN** the existing agent/backend switching behavior is preserved

### Requirement: Model selection remains available before launch
Pane-creation interfaces SHALL continue to offer their supported provider and model choices before a new pane is launched.

#### Scenario: User creates a pane
- **WHEN** the user opens a supported pane-creation interface
- **THEN** the available launch-time provider and model choices remain selectable
