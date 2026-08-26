## 1. Mobile rename interaction

- [x] 1.1 Add captured rename-target and draft state to the mobile conversation view and connect it to the existing pane-label update action
- [x] 1.2 Add Rename conversation to More actions with online/running/selected-pane availability and a focused, prefilled rename sheet
- [x] 1.3 Trim and save a non-empty draft to the captured pane, while cancel, backdrop dismissal, empty input, and a removed target send nothing

## 2. Regression coverage

- [x] 2.1 Test that renaming updates the selected pane through `updatePaneLabel`, trims the label, and closes the editor
- [x] 2.2 Test prefill, cancellation, empty-label rejection, unavailable states, and protection against renaming a different or removed pane

## 3. Verification

- [x] 3.1 Run the focused mobile conversation tests, full web tests, lint, and production build
