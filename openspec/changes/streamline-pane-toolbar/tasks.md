## 1. Relocate Terminal View Controls

- [x] 1.1 Refactor terminal pane rendering so `TabbedView` owns the active session/pane view mode and passes that controlled mode to the terminal body without unmounting the live terminal.
- [x] 1.2 Render the compact Terminal/Conversation switch in the shared toolbar only for terminal panes, immediately before provider usage, and remove its former pane-content row.
- [x] 1.3 Preserve per-session/per-pane view restoration and the existing Conversation view content and terminal-input behavior across tab changes.

## 2. Simplify Pane Toolbar and Views

- [x] 2.1 Remove the Timeline/Chat toolbar action, per-pane timeline state, and alternate timeline rendering so non-terminal panes always use the standard conversation view.
- [x] 2.2 Confirm timeline extraction has no remaining consumers, then remove its orphaned component, utility, and dedicated tests if unused.
- [x] 2.3 Remove the inline model and reasoning-effort selectors and any UI-only option/normalization code that becomes unused, while retaining provider switching, persisted pane values, backend update contracts, and saved effort behavior for bot startup.
- [x] 2.4 Clean up obsolete imports, store subscriptions, comments, and toolbar layout descriptions introduced by the removed controls.

## 3. Test and Verify

- [x] 3.1 Extend terminal-pane component tests to verify toolbar placement before Codex usage, switching in both directions, retention of the mounted terminal instance, and restoration of independent pane preferences.
- [x] 3.2 Add or update toolbar tests to verify the terminal switch is absent for non-terminal panes, Timeline/Chat and model/effort controls are absent, and applicable provider and pane actions remain.
- [x] 3.3 Run the focused Vitest suites, the full web test suite, and web lint; resolve all regressions caused by the cleanup.
