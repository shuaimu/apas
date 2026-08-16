## 1. Session screen chrome

- [x] 1.1 Move the pane list and the add-pane control into the top row, right of the back control, keeping it horizontally scrollable
- [x] 1.2 Reduce project name, host, and status to the single line beneath, and remove the separate steer-pane row
- [x] 1.3 Remove the Account control from the session screen and drop the callback that fed it
- [x] 1.4 Move Raw terminal and Summary to the composer row as icon controls, keeping their disabled states and the pane they act on
- [x] 1.5 Tests: panes are selectable from the top row, the account control is gone, and the two moved controls still act on the selected pane

## 2. Manage this project

- [x] 2.1 Add a Manage control where the account control was, opening project management for the current session
- [x] 2.2 Show the permitted tab types as an allow list derived from the stored deny list, so an unrestricted project shows everything permitted
- [x] 2.3 Let a user who may manage the project change the set, writing back the deny list through the existing project-flags path without disturbing the other flags
- [x] 2.4 Render it read-only for everyone else
- [x] 2.5 Tests: an unrestricted project shows all types permitted; clearing one writes the complementary deny list and preserves the other flags; a non-manager cannot change it

## 3. Show the ceiling, not just the project's own list

- [x] 3.1 Derive whether the effective cluster policy permits each tab type at all, matching any allowed launch profile for that kind and provider
- [x] 3.2 Present a type the cluster policy forbids as unavailable rather than permitted, not togglable by anyone, and say the restriction is not the project's
- [x] 3.3 Tests: a type outside the cluster policy reads as unavailable and cannot be toggled by an owner; a type inside it stays owner-togglable

## 4. Verification

- [x] 4.1 Web lint, type-check, and test suite clean
- [ ] 4.2 Live on the mobile browser: open a project, switch panes from the top row, open the raw terminal and summary from the composer, and restrict a tab type as the owner
