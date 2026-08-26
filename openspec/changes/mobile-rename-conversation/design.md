## Context

The responsive mobile conversation screen owns its selected pane and presents
pane-scoped operations in a More bottom sheet. Desktop already renames pane
labels through `updatePaneLabel`, which optimistically updates `paneConfigs`,
sends `update_pane_label`, and retains an unacknowledged rename for reconnect.
See `proposal.md` for motivation and the delta spec for observable behavior.

## Goals / Non-Goals

**Goals:**

- Put rename beside the other actions for the selected mobile conversation.
- Target the pane that was selected when the editor opened.
- Reuse the existing optimistic, acknowledged, retryable label path.
- Follow the established mobile bottom-sheet interaction and accessibility
  patterns.

**Non-Goals:**

- Rename projects, sessions, transcript titles, or provider conversations.
- Add a new wire message, server route, persistence model, or offline mutation.
- Change desktop rename behavior.

## Decisions

### Open a dedicated rename sheet from More actions

The More sheet gains a `Rename conversation` row. Selecting it closes More and
opens a focused sheet with one labelled text input and Cancel/Save actions. The
input is prefilled, focused, and selected so a phone user can replace the name
without manually clearing it.

An inline field inside More was considered, but it would mix navigation and
editing states, crowd the keyboard-height viewport, and make backdrop dismissal
ambiguous. A dedicated sheet matches the existing reboot and close flows.

### Capture a rename target rather than reading selection at save time

Opening the editor stores the selected `PaneConfig` as its target and seeds the
draft from that pane's display label. Saving uses the captured pane id. If that
pane disappears before save, the editor closes without sending a rename.

Reading `selectedPaneId` only when Save is pressed was considered, but a pane
list refresh or selection change underneath the sheet could otherwise rename a
different agent than the one named in the dialog.

### Delegate persistence and convergence to `updatePaneLabel`

Save trims the draft, refuses an empty result, and calls the existing store
action once. Its optimistic update immediately changes the top pane selector
and dialog/header labels; its existing acknowledgement and reconnect queue own
eventual persistence. Rename is unavailable while disconnected or while the
project is stopped, consistent with mobile mutation rules.

A component-specific WebSocket send was rejected because it would duplicate
the retry and acknowledgement behavior and could reintroduce lost renames.

## Risks / Trade-offs

- [The optimistic label can briefly precede CLI persistence] → Reuse the
  existing pending-label acknowledgement and retry mechanism.
- [The software keyboard reduces sheet height] → Keep the editor to one input
  and two actions, with safe-area padding and no nested scroll content.
- [The selected pane can be removed while editing] → Verify the captured pane
  still exists before saving and dismiss without mutation if it does not.

## Migration Plan

Deploy the web application only. No data migration or rollout ordering is
required because all transport and persistence behavior already exists.
Rollback restores the previous web build and leaves any labels already saved
fully compatible.
