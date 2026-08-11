## Context

See `proposal.md` for motivation. New CLI projects deliberately persist and report an empty pane list. `TabbedView` currently derives `effectiveTabs`, then returns its generic empty state whenever that list is empty. That return occurs before `TabBar`, even though `TabBar` already supports an empty `tabs` array by rendering its pinned Overview pseudo-tab and Add Pane action.

The active-view derivation also exits when there are no pane IDs. If a user switches from a populated project to a zero-pane project, the outgoing pane ID can therefore remain selected instead of landing on Overview.

## Goals / Non-Goals

**Goals:**

- Model “no selected session” and “selected session with no panes” as separate UI states.
- Reuse the existing Overview, tab bar, Add Pane flow, and launch-policy filtering.
- Ensure a zero-pane project deterministically lands on Overview, including after switching from another project.
- Cover both sides of the state boundary with component/unit regression tests.

**Non-Goals:**

- Creating a default pane automatically.
- Changing project metadata, server messages, pane persistence, or launch-policy enforcement.
- Redesigning the Overview or Add Pane menu.
- Changing behavior for legacy single-pane sessions that are synthesized from messages.

## Decisions

### Separate session absence from an empty effective tab list

The early fallback will be gated by the absence of a selected `sessionId`, not by `effectiveTabs.length === 0`. A selected session will continue into the normal workspace render path even when `TabBar` receives `tabs={[]}`.

This reuses `TabBar`'s existing pinned Overview and Add Pane behavior and avoids introducing a second first-pane control. The alternative—adding an Add Pane button to the generic empty state—would duplicate policy-aware UI and would still leave Overview inaccessible.

### Treat Overview as the active view when no real pane IDs exist

Initial active-view derivation will return `OVERVIEW_PANE_ID` for a selected project with no effective pane IDs. Project-change tracking must still advance in this case so an active pane ID from the previously selected project cannot leak into the zero-pane workspace.

The Overview pseudo-tab will remain outside the authoritative `paneConfigs`; injecting a synthetic pane config was rejected because it would mix navigation-only state with CLI-owned pane state and could accidentally enter pane message or lifecycle flows.

### Preserve the existing first-pane command path

The empty tab bar will invoke the existing `handleAddTab`/store action. Provider visibility, project policy filtering, request construction, and authoritative `PaneList` reconciliation therefore stay identical for the first and subsequent panes.

### Test the observable state boundary

Component coverage will seed a valid selected/attached session with `paneConfigs: []`, assert that Overview and the New Tab action render, exercise an allowed first-pane selection, and assert that the store action is called. A complementary no-session case will assert that the fallback remains and project-specific controls are absent. Active-tab derivation will also be covered for an empty pane ID list.

## Risks / Trade-offs

- [Risk] The normal shell may render briefly while a selected session is still attaching and has not received its pane list. → Mitigation: pane creation already reports attachment/send failures through the existing store action, while the selected-session shell remains stable as authoritative state arrives.
- [Risk] Changing empty-list active derivation could cause a transient jump to Overview during project switching. → Mitigation: scope the fallback to the selected session lifecycle and test transitions from a populated project to an attached zero-pane project.
- [Risk] Empty Overview children may assume at least one pane. → Mitigation: render the actual Overview in the zero-pane component test and retain existing empty-array tests for its child panels.

## Migration Plan

No data or protocol migration is required. Deploy the web build after tests pass; existing sessions and project metadata remain compatible. Rollback consists of restoring the previous web build, although that restores the zero-pane usability bug.
