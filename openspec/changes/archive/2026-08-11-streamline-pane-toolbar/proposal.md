## Why

The pane toolbar is crowded with controls that duplicate capabilities now available in provider terminals, while the terminal-specific view switch sits lower in a separate content row. Simplifying and regrouping these controls makes the active pane's primary view and usage information easier to find, especially on narrow screens.

## What Changes

- Remove the Timeline/Chat toolbar action and its alternate timeline rendering so non-terminal panes consistently show the standard conversation view.
- Move the Terminal/Conversation switch into the shared pane toolbar, immediately before the active provider's usage status, while preserving its per-pane selection and view behavior.
- Remove the inline model and reasoning-effort selectors from the pane toolbar; users change those settings through the provider terminal instead.
- Keep provider switching, persisted pane model/effort data, and the existing server/CLI update contracts unchanged.
- Update web tests to cover the simplified toolbar, terminal view switching, placement, and absence of the removed controls.

## Capabilities

### New Capabilities

- `pane-toolbar`: Defines which pane-level controls appear in the web toolbar and how terminal view selection is positioned and behaves.

### Modified Capabilities

None.

## Impact

- Primarily affects `packages/web/src/components/tabs/TabbedView.tsx`, the terminal view toggle presentation, and related component tests.
- Timeline extraction code may become unused and can be removed if no other consumer remains.
- No WebSocket message, server, CLI, stored pane schema, or external dependency changes are required.
