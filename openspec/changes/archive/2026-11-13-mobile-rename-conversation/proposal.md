## Why

The mobile conversation view identifies each agent by its pane label, but the
only way to change that label is currently on desktop. Mobile users need to be
able to give the conversation they are already viewing a useful name without
leaving the session.

## What Changes

- Add a Rename conversation action to the selected pane's existing More menu.
- Open a mobile rename editor prefilled with the selected pane's current label.
- Save a trimmed, non-empty label through the existing durable pane-label
  update path so the header, pane selector, desktop, and later sessions agree.
- Allow canceling without changing the label and prevent renaming when no real
  pane is selected.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `mobile-code-sessions`: an authorized mobile user can rename the currently
  selected pane conversation from its More actions.

## Impact

- `packages/web/src/components/mobile/MobileSessionActivity.tsx`: More-menu
  action and rename editor state/UI.
- `packages/web/src/components/mobile/MobileSessionActivity.test.tsx`: mobile
  interaction, validation, cancel, and selected-pane targeting coverage.
- No wire, server, CLI, persistence, or dependency changes: the feature reuses
  the existing `updatePaneLabel` store action and acknowledgement/retry flow.
