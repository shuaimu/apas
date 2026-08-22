## 1. Per-agent detail on the web session list

- [x] 1.1 Populate the panes and their working state for the web list, from the same source the session-level flag uses
- [x] 1.2 Share one helper between the web and mobile lists so the two cannot disagree
- [x] 1.3 Carry the detail into the web session type

## 2. The sidebar

- [x] 2.1 Add the switch between projects and idle agents, showing which is active
- [x] 2.2 List one row per idle agent, naming its project and host, including inside working projects
- [x] 2.3 Exclude agents in projects that are not running, and sessions with no detail
- [x] 2.4 Say so plainly when nothing is idle
- [x] 2.5 Tests: mixed project, stopped project, missing detail, empty state, and returning to the projects

## 3. Opening the agent

- [x] 3.1 Carry the named agent through the store, since the remembered tab is keyed by something the caller cannot know before attaching
- [x] 3.2 Honour it once the session is attached and the agent exists, then clear it
- [x] 3.3 Tests: opening a row asks for that session and that agent

## 4. Distinguish unavailable providers from idle agents

- [x] 4.1 Preserve an explicit provider usage-limited result, limiting window, and reset time while respecting enabled extra usage
- [x] 4.2 Keep usage-limited agents out of the ordinary idle rows and list them in a Usage limited subsection beneath Idle sessions
- [x] 4.3 Show a reset-aware usage-limited label on the affected pane and test that ordinary idle rows precede the subsection

## 5. Rank idle agents by recency

- [x] 5.1 Record and transport the pane's idle-transition timestamp, preserving it across repeated idle observations and defaulting it for older payloads
- [x] 5.2 Sort Idle sessions by the shared recent-idle ordering, with known timestamps first and deterministic legacy fallback
- [x] 5.3 Tests: working-to-idle transition, repeated idle, newest-first desktop ordering, and missing timestamps

## 6. Verification

- [x] 6.1 Workspace tests and clippy clean; web lint, type-check, and tests clean
- [ ] 6.2 Live on the desktop against a project with a working agent and an idle one
