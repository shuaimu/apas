## 1. Report per-agent state

- [x] 1.1 Add a per-pane summary to the mobile payload — identity, label, kind, provider, and whether it is working — defaulted so an older server parses
- [x] 1.2 Populate it from the session manager's pane roster and pane statuses, the same source the session-level flag uses
- [x] 1.3 Tests: a session with a mix reports each pane's state; the session-level flag agrees with the panes; a session with no panes reports none

## 2. List idle agents on mobile

- [x] 2.1 Rename the view to "Idle sessions" and render one row per idle pane, naming its project and host
- [x] 2.2 Include idle panes from projects that are otherwise working
- [x] 2.3 Treat absent per-pane detail as unknown rather than idle, so an older server does not fill the list with everything
- [x] 2.4 Say so plainly when nothing is idle
- [x] 2.5 Tests: mixed project lists only its idle panes, an all-working project contributes none, absent detail contributes none, and the empty state renders

## 3. Open the agent, not just the project

- [x] 3.1 Move the remembered-pane helpers out of the session screen so the home can write what the session screen reads
- [x] 3.2 Remember the tapped pane before navigating, and confirm the session screen honours it
- [x] 3.3 Tests: tapping a row records that pane for that session and opens the session

## 4. Distinguish usage-limited agents

- [x] 4.1 Carry an explicit usage-limited result and reset metadata without treating a 100 percent included meter as blocking when extra usage remains available
- [x] 4.2 Keep usage-limited panes out of ordinary idle rows and render them in a Usage limited subsection beneath Idle sessions
- [x] 4.3 Tests: limited panes follow idle rows with reset metadata and become non-limited after reset

## 5. Rank idle agents by recency

- [x] 5.1 Add the pane idle-transition timestamp to the shared/mobile summary with backward-compatible defaults
- [x] 5.2 Reuse the desktop recent-idle ordering so mobile lists known timestamps newest first and legacy panes afterward
- [x] 5.3 Tests: transition timestamps, replay preservation, newest-first mobile ordering, and legacy fallback

## 6. Preserve the source list across a conversation

- [x] 6.1 Keep the selected mobile home list in the mounted page shell so opening and closing a conversation does not reset it
- [x] 6.2 Regression test: enter a conversation from Idle sessions, go back, and remain on Idle sessions
- [x] 6.3 Reconcile the conversation's working indicator with authoritative idle snapshots and reset live status before same-session reattachment
- [x] 6.4 Regression tests: a refreshed idle snapshot and same-session reattachment both clear a stale working indicator

## 7. Verification

- [x] 7.1 Workspace tests and clippy clean; web lint, type-check, and tests clean
- [ ] 7.2 Live on the mobile browser against a project with a working pane and an idle one
