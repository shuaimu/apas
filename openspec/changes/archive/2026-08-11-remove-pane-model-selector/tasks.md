## 1. Remove Post-Launch Model Selection

- [x] 1.1 Remove the dedicated Claude model dropdown from Overview pane cards for both managed and unmanaged panes.
- [x] 1.2 Delete pane-card-only model options, normalization, conditional rendering, and comments that become unused.
- [x] 1.3 Preserve the existing agent frontend/API backend selector, stored pane model values, and all launch-time provider/model selection interfaces.

## 2. Test and Verify

- [x] 2.1 Replace pane-card model-mutation tests with coverage proving that managed and unmanaged cards render no model selector and do not directly update stored models.
- [x] 2.2 Retain and run coverage for agent/backend switching and pane-creation model choices to guard the intentionally preserved behavior.
- [x] 2.3 Run the focused PaneGrid tests and web lint, resolving any regressions introduced by the removal.
