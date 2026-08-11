## Why

New projects intentionally start with no panes, but the web client currently treats an attached zero-pane project like an unselected project. It returns an inert empty state before rendering the tab bar, Overview, or Add Pane controls, leaving users unable to create the first pane from the canonical web interface.

## What Changes

- Distinguish an attached project with zero panes from the absence of a selected project/session.
- Render the normal project shell, tab bar, and Overview for an attached zero-pane project.
- Keep the Add Pane control available so users can create the project's first pane from the web.
- Reserve the existing no-project empty state for cases where no project/session is selected.
- Add regression coverage for an attached project whose authoritative `paneConfigs` is empty.

## Capabilities

### New Capabilities

- `web-project-workspace`: Defines web workspace behavior for selected projects, including the valid zero-pane state and first-pane creation.

### Modified Capabilities

None.

## Impact

- Web tab/workspace rendering and initial active-view derivation in `packages/web/src/components/tabs/TabbedView.tsx`.
- Web component tests covering empty and zero-pane project states.
- No server protocol, database, CLI metadata, dependency, or deployment changes are expected.
