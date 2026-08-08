## 1. Shared Provider Contract and Catalog

- [x] 1.1 Add shared retirement classifiers for explicit provider values, historical model names, backend names, and stable launch-profile keys, with fixtures for MiniMax, M2, and GLM forms.
- [x] 1.2 Convert `Provider::Minimax` and `Provider::Glm` into documented compatibility tombstones that deserialize but are never returned by an active supported-provider path.
- [x] 1.3 Remove MiniMax and GLM from the supported launch-profile registry and policy defaults while preserving Claude, Codex, DeepSeek, OpenCode, Cursor Agent, and terminal profiles.
- [x] 1.4 Quarantine legacy MiniMax/GLM machine-configuration message variants for inbound rejection only and remove retired backend fields from active machine status payloads.
- [x] 1.5 Add shared serialization, catalog, default-policy, and retirement tests proving legacy values remain readable but cannot be considered supported.

## 2. Persisted Policy and Server Enforcement

- [x] 2.1 Implement an idempotent transactional migration that filters retired profile keys from cluster defaults and non-null project overrides while preserving supported entry order and explicit empty overrides.
- [x] 2.2 Allocate newer monotonic policy versions only for rows changed by provider retirement and publish migration diagnostics without configuration or credential contents.
- [x] 2.3 Reject retired and unknown profile keys in cluster-default and project-override mutation APIs so stale administrator clients cannot restore them.
- [x] 2.4 Apply the retirement classifier before policy evaluation and routing for pane add/start/restart/resume/reboot, model/backend switch, team start, and team-member launch messages.
- [x] 2.5 Remove MiniMax/GLM machine-status merging, configuration forwarding, and usage handling from server session and WebSocket paths while safely rejecting or ignoring legacy inbound mutations.
- [x] 2.6 Add database tests for mixed allowlists, retired-only overrides, unchanged policies, version advancement, idempotence, and effective-policy broadcasts.
- [x] 2.7 Add server authorization/routing tests proving retired requests receive an unsupported-provider error and are never forwarded, while retained provider requests still route.

## 3. CLI and Daemon Runtime Removal

- [x] 3.1 Remove MiniMax/GLM paths, base URLs, API keys, and config-command keys from the active CLI configuration model while verifying older TOML fields are ignored without logging or breaking supported settings.
- [x] 3.2 Remove MiniMax/GLM runtime environment construction, binary/model detection, argument building, labels, backend switches, and provider-specific resume behavior; retain or rename generic Anthropic-compatible helpers required by DeepSeek.
- [x] 3.3 Enforce retirement immediately before every CLI spawn/restore, pane restart/resume/reboot, model switch, team start, and managed-worker addition so an older server cannot bypass host enforcement.
- [x] 3.4 Load historical retired panes without rewriting their identity, exclude them from restore queues, and report them as unsupported and stopped while continuing to load supported siblings.
- [x] 3.5 Reuse the graceful pane-stop path to interrupt and terminate any running retired pane on startup or catalog/policy refresh without deleting history, worktrees, or project metadata.
- [x] 3.6 Remove MiniMax/GLM daemon machine snapshot fields, configuration update handling, backend refreshes, and outbound status messages while retaining DeepSeek machine configuration.
- [x] 3.7 Remove MiniMax/GLM usage fetching, parsing, cache slots, polling, and reporting; ensure legacy cache fields are ignored and supported provider usage remains readable.
- [x] 3.8 Replace CLI/daemon provider-feature tests with fail-closed launch, legacy project loading, selective graceful-stop, inert credential, mixed-version message, and DeepSeek regression coverage.

## 4. Web Catalog, Controls, and Historical State

- [x] 4.1 Remove MiniMax/GLM constants and options from the canonical web provider catalog and change historical provider/model lookup to return an explicit unsupported result instead of the default Claude option.
- [x] 4.2 Remove retired choices from add-pane, pane toolbar, project goal/role launch, team setup, add-worker, and cluster-policy editing surfaces while preserving effective-policy filtering for all retained choices.
- [x] 4.3 Remove MiniMax/GLM machine configuration cards, store actions/types, backend status rendering, credential forms, and related request messages; retain DeepSeek controls.
- [x] 4.4 Remove retired usage-limit store handling and UI rollups and safely ignore legacy machine/usage fields received from older peers.
- [x] 4.5 Render historical MiniMax/GLM panes as non-interactive unsupported panes with history/delete access where applicable and no start, restart, resume, model, backend, or team controls.
- [x] 4.6 Add component and store tests covering every affected selector, policy editor, Machines/usage absence, stale message handling, and historical unsupported-pane presentation.

## 5. Cleanup, Compatibility, and Release Verification

- [x] 5.1 Remove obsolete provider-specific helpers, tests, comments, and feature documentation while retaining clearly marked compatibility tombstones, retirement errors, migration notes, and fixtures.
- [x] 5.2 Audit outbound shared messages, active catalogs, configuration commands, runtime spawn branches, and web controls to prove upgraded components cannot emit or select MiniMax/GLM values.
- [x] 5.3 Run the complete Rust workspace tests, web tests, formatting, Rust Clippy, web lint, production builds, and static musl CLI build; resolve all introduced regressions for retained providers.
- [x] 5.4 Document the breaking behavior, inert legacy credentials, active-pane shutdown, server-to-host-to-web rollout, policy verification queries, fleet-version checks, and coordinated rollback procedure.
- [x] 5.5 Perform staged compatibility verification with an old client against the new server and a new CLI against an old server, confirming explicit rejection without disconnects or fallback launches.
