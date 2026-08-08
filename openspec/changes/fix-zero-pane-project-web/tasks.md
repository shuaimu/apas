## 1. Workspace State Handling

- [x] 1.1 Update initial active-view derivation so a selected project with no real pane IDs lands on `OVERVIEW_PANE_ID`, including when switching from a populated project.
- [x] 1.2 Restrict the generic no-project fallback to the absence of a selected session and allow an empty effective pane list to render the normal workspace shell.
- [x] 1.3 Verify the empty `TabBar` path exposes Overview and the existing policy-aware Add Pane action without synthesizing or automatically creating a pane.

## 2. Regression Coverage

- [x] 2.1 Add unit coverage for active-view derivation with an empty pane ID list and for the populated-project-to-zero-pane transition.
- [x] 2.2 Add component coverage showing that an attached session with `paneConfigs: []` renders Overview and the New Tab control instead of the no-project fallback.
- [x] 2.3 Exercise first-pane creation in the zero-pane component test and assert that an allowed selection invokes the existing pane creation action for the selected project.
- [x] 2.4 Add or extend policy coverage to confirm a disallowed launch profile is not offered for the first pane.
- [x] 2.5 Add component coverage confirming that no selected session still renders the fallback and omits Overview and pane creation controls.

## 3. Verification

- [x] 3.1 Run the focused `TabbedView` and `TabBar` test files covering active-view selection, zero-pane rendering, first-pane creation, and launch-policy filtering.
- [x] 3.2 Run the web package type check and lint checks.
- [x] 3.3 Run the web package test suite and confirm existing populated-pane, legacy-message synthesis, and project-switch behavior remain passing.
