## Why

An existing pane is launched with a model configuration, so presenting a model dropdown on its Overview card incorrectly suggests that its model is an ordinary post-launch setting. Removing that control makes the UI match the pane lifecycle and avoids disruptive or misleading model changes.

## What Changes

- Remove the dedicated Claude model selector from managed and unmanaged pane cards in the Overview.
- Keep each launched pane's stored model unchanged when its card is displayed or used.
- Preserve model selection in pane-creation flows, where the model is chosen before launch.
- Preserve the separate agent frontend/API backend selector and existing backend update contracts; this change removes direct per-pane model selection only.
- Update pane-card tests to assert that no post-launch model selector is rendered while the remaining pane controls still work.

## Capabilities

### New Capabilities

- `pane-model-selection`: Defines model selection as a launch-time choice and prohibits a dedicated model-changing control on an existing pane.

### Modified Capabilities

None.

## Impact

- Affects `packages/web/src/components/overview/PaneGrid.tsx` and its component tests.
- Removes pane-card-only model option and normalization code that becomes unused.
- Does not change pane configuration storage, WebSocket messages, server or CLI behavior, provider/backend switching, or model choices offered while creating a pane.
