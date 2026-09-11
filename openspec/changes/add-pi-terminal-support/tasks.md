## 1. Shared Catalog, Capability, and Protocol

- [x] 1.1 Add the `Pi` provider variant with serde name `pi` and include it in the terminal tab-type catalog (`terminal:pi`).
- [x] 1.2 Add the `terminal:pi:official:default` launch profile ("Pi Terminal") to the supported-profile catalog and the launch-profile key mapping.
- [x] 1.3 Add the `terminal_pi_v1` CLI capability constant and advertise it from the project host registration.
- [x] 1.4 Regenerate `packages/protocol` schemas and generated types, and update shared catalog/profile contract tests.

## 2. Pi Terminal Hosting and Configuration

- [x] 2.1 Add Pi to the terminal-host allowlist and resolve launches through the configured `pi_path`.
- [x] 2.2 Add `pi_path` to local configuration, its CLI `config set/get/list` handling, and the provider display-name and config-key mappings.
- [x] 2.3 Build fresh-launch arguments using Pi's exact session identity plus a positional initial instruction, and restore arguments that open the exact retained session file with pinned-identity fallback.
- [x] 2.4 Add terminal argument and missing-binary regression tests that preserve existing Claude, Codex, and OpenCode behavior.

## 3. Transcript Recovery and Usage

- [x] 3.1 Locate the pane's exact Pi session file under the sessions root, honoring configuration-directory and session-directory overrides, and tolerate its absence before the first completed turn.
- [x] 3.2 Parse Pi session entries into conversation turns: user and completed assistant text only, excluding thinking, tool results, shell execution, extension entries, compaction, branch summaries, labels, and model/thinking bookkeeping.
- [x] 3.3 Derive completion state from the recorded assistant stop reason and carry model, token, and cost usage onto the pane's turn records.
- [x] 3.4 Integrate Pi into the terminal transcript watcher with per-pane pinned-source identity and no directory-recency fallback.
- [x] 3.5 Add session fixtures and tests covering extension entries, active-branch selection, tool continuations, usage/cost, and same-directory pane isolation, including a fixture captured from an installed Pi release.

## 4. Server and Mobile Authorization

- [x] 4.1 Gate new Pi terminal launches on the `terminal_pi_v1` capability in the server authorization path.
- [x] 4.2 Gate mobile Pi task launch on the same capability without affecting Claude, Codex, or OpenCode routing.
- [x] 4.3 Add server authorization tests for capable and incapable project hosts.

## 5. Web and Mobile Catalogs

- [x] 5.1 Add Pi to the web provider union, normalization, launch-profile key logic, tab-type catalog, and provider option groups.
- [x] 5.2 Add Pi to the desktop add-tab menu, pane provider selector, and mobile launch parsing and labels with effective-policy filtering.
- [x] 5.3 Update web catalog, menu, and mobile launch tests.

## 6. oh-my-pi Package Support

- [x] 6.1 Document installing and authenticating Pi and installing `oh-my-pi` globally, including the project-trust boundary for project-local installs and diagnosing with the `/oh-my-pi` command inside a Pi pane.
- [x] 6.2 Verify the published `oh-my-pi` package declares and loads its Pi extension entry in a real Pi process, and that a Pi session written with the extension present parses as standard conversation.
- [ ] 6.3 On a host with an authenticated Pi, exercise the oh-my-pi orchestrator prompt and specialist subagents end to end inside an APAS Pi pane, confirming their session files never confuse pane history.

## 7. Documentation and Verification

- [x] 7.1 Document Pi installation, `pi_path` configuration, exact-session behavior, the unfollowed in-TUI session-switch limitation, policy opt-in, and rolling deployment order.
- [x] 7.2 Verify formatting and generated protocol contracts are stable.
- [x] 7.3 Run the complete shared, CLI, server, web, protocol, terminal-web, and native mobile test, lint, and typecheck suites.
- [x] 7.4 Validate the Pi command shapes against an installed current Pi release, and build the production web bundle.
