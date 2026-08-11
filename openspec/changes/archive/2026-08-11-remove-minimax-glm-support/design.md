## Context

See `proposal.md` for motivation and `specs/provider-support/spec.md` for the behavior contract. MiniMax and GLM currently appear in two forms: explicit legacy `Provider` enum values and Claude-provider panes whose model name selects an Anthropic-compatible MiniMax or GLM backend. Their support is consequently spread across shared launch-profile derivation, CLI process environment construction, machine configuration messages, daemon state, usage polling, server routing, project policy persisted as JSON arrays in SQLite, and web provider helpers and controls.

Existing `.apas` files, cached usage documents, browser state, and mixed-version WebSocket peers can still contain those values after the feature code is removed. The removal must therefore distinguish active support from bounded decoding compatibility. The cluster-policy rollout has also persisted MiniMax/GLM profile keys in cluster defaults and project overrides, so changing only the in-code registry would leave misleading policy state behind.

## Goals / Non-Goals

**Goals:**

- Establish one shared retirement classification used before every server and CLI launch boundary.
- Remove all active MiniMax/GLM configuration, credential, runtime, usage, policy-selection, and UI paths without affecting other providers.
- Keep projects and mixed-version connections recoverable when historical values are encountered.
- Normalize persisted policy idempotently and advance versions only when stored policy changes.
- Stop already-running retired-provider panes while leaving supported panes and project data intact.

**Non-Goals:**

- Removing or redesigning Claude, Codex, DeepSeek, OpenCode, Cursor Agent, terminal panes, or the general provider abstraction.
- Deleting historical conversations, pane records, audit data, or entire user configuration files.
- Automatically translating a retired pane into a supported provider or model.
- Immediately removing every legacy wire discriminator; compatibility tombstones remain for one rollout window and can be deleted in a later cleanup.

## Decisions

### 1. Separate the supported catalog from legacy decoding tombstones

Remove MiniMax and GLM profiles from `supported_launch_profiles` and from every web provider/model catalog. Keep the explicit `Provider::Minimax` and `Provider::Glm` values temporarily as deprecated, non-launchable tombstones so old `.apas` files and wire payloads still deserialize. No supported catalog, default, selector, process builder, or ordinary match branch may treat those variants as executable.

Likewise, retain retired inbound machine-configuration message discriminators only long enough to return an unsupported-operation response without tearing down a mixed-version WebSocket. They are never emitted by upgraded code, and credential fields are discarded without logging or echoing. Machine status fields can be removed from the active `MachineInfo` shape because Serde ignores unknown fields from older senders by default.

This is preferred over deleting enum/message variants immediately, which would convert one retired field into a parse failure and disconnect the whole peer. Replacing the provider enum with a generic `Unknown(String)` variant was considered but would require custom serialization and broaden this change beyond the two known retired values.

### 2. Centralize fail-closed retirement classification

Add shared helpers that classify a request as retired from all forms APAS has historically accepted: explicit provider variants, normalized MiniMax/M2 or GLM model names, backend names, and stable profile-key segments. The server calls the classifier before policy allowlist evaluation or routing. The CLI calls it immediately before every create, restore, restart, resume, reboot, model/backend switch, and managed-team spawn.

Retirement returns a distinct unsupported-provider error. It never falls through to the current default-provider behavior; in particular, web lookup helpers must return an explicit unsupported result instead of mapping an unknown historical value to the first Claude option.

Duplicating only a small presentation classifier in TypeScript is acceptable because the web is not an enforcement boundary. Catalog parity and representative legacy values are tested against the shared Rust rules.

### 3. Quarantine and stop historical panes without rewriting identity

When project metadata is loaded, retired panes remain present with their original provider/model identity for history and operator diagnosis, but they are excluded from spawn/restore queues and reported as unsupported and stopped. The CLI does not rewrite them as Claude or another supported provider.

At startup and when a new effective policy/catalog is accepted, the CLI scans active pane handles. A retired pane is first marked paused/stopping, receives the existing graceful interrupt path, and is then terminated through the normal pane-stop mechanism if it does not exit. Only matching handles are affected. Pane history, worktrees, metadata, and sibling panes are preserved.

Deleting retired pane records was rejected because it would hide history and could orphan worktrees. Allowing already-running panes to continue was also rejected: provider retirement is stronger than an ordinary policy disallow and the completed system must no longer operate those backends.

### 4. Normalize persisted policy in an idempotent database transaction

During server migration, read the cluster-default allowlist and every non-null project override, remove keys whose provider/backend/model segments identify MiniMax or GLM, and preserve all other entries in their existing order. Use the existing monotonic policy-version allocation for each changed row and leave unchanged rows and versions untouched. An override that becomes empty stays an explicit JSON empty array; it is not converted to `NULL`, inheritance, or an allow-all default.

The migration runs transactionally and is naturally idempotent because a second pass finds no retired keys. It records only counts and affected project IDs in migration/audit diagnostics, never credentials or whole configuration payloads. Runtime policy update validation also rejects retired keys so they cannot be reintroduced by an older administrator client.

Changing only the effective-policy calculation was rejected because stale keys would remain visible in storage and could become active again after rollback or later registry changes. Resetting every project to cluster defaults was rejected because it would destroy intentional supported-provider overrides.

### 5. Remove active credential, machine, and usage plumbing end to end

Remove MiniMax/GLM fields from the active CLI config model and config-command allowlist, backend environment construction, daemon machine snapshots and update handling, server machine-state merging/routing, usage provider/cache/polling/reporting, web store types/actions, Machines controls, and Usage views. Existing TOML keys are accepted as unknown input by deserialization and ignored; a later save may naturally omit them, but deployment does not scan or rewrite users' configuration files.

DeepSeek continues using the Claude CLI bridge and therefore its shared bridge primitives must be retained even where a function was originally introduced for GLM. Shared code is renamed around a generic Anthropic-compatible backend where useful, rather than deleting behavior DeepSeek still requires.

### 6. Make unsupported state explicit in the web without restoring controls

Provider catalogs and selectors contain only supported entries. Historical panes whose provider/model classifier is retired render a non-interactive “unsupported provider” state with delete/history access as applicable, but no start, restart, resume, model, or backend control. Machine and usage pages omit retired cards entirely. Incoming legacy machine/usage fields are ignored rather than inserted into Zustand state.

This avoids the current provider-option fallback, which could visually and operationally convert an unknown MiniMax/GLM pane into Claude Official. Hiding the historical pane entirely was rejected because users still need to understand why a saved pane did not restart.

### 7. Verify removal by behavior and catalog boundaries

Replace provider-feature tests with retirement tests rather than merely deleting coverage. Shared tests assert the supported registry and defaults contain no retired keys and the classifier catches legacy forms. Server/CLI tests prove double enforcement and graceful stopping. Migration tests cover mixed, retired-only, and already-clean policies. Web tests cover every launch surface, machine/usage absence, and historical unsupported rendering. Existing tests for all retained providers and terminals remain the regression baseline.

A source-wide ban on the words “MiniMax” and “GLM” is not appropriate because tombstones, migration code, errors, and compatibility tests must name them. Checks instead assert that active catalogs, outbound messages, configuration commands, runtime branches, and user-selectable controls cannot produce them.

## Risks / Trade-offs

- **[Legacy tombstones are mistaken for continued support]** → Keep them deprecated and isolated behind retirement helpers; add tests proving no active registry or spawn branch accepts them, then remove them in a later compatibility cleanup.
- **[A model-name heuristic misses a historical spelling]** → Cover explicit providers, backend/profile segments, known MiniMax/M2 and GLM forms from fixtures, and fail closed for unrecognized profiles under the existing explicit allowlist.
- **[Policy migration accidentally broadens access]** → Filter entries in place, preserve explicit empty overrides, allocate a newer version only on change, and test idempotence.
- **[Stopping a running pane loses an in-flight response]** → Use graceful interrupt before forced termination, preserve all stored history/worktrees, and scope the stop to retired panes only.
- **[Removing shared bridge code breaks DeepSeek]** → Retain and rename generic Anthropic-compatible helpers and keep DeepSeek launch/configuration tests in the required suite.
- **[Older clients keep showing or requesting retired providers during rollout]** → Deploy server rejection first, then upgrade every project-host CLI/daemon, and deploy the web last; the rollout is complete only after hosts advertise the compatible version.

## Migration Plan

1. Back up the SQLite database and current server, CLI distribution, and web build. Inventory connected project-host versions and persisted policies containing retired keys.
2. Deploy the server first. Its startup migration removes retired policy keys and advances changed versions; its routing boundary rejects stale MiniMax/GLM mutations while preserving read-only mixed-version connectivity.
3. Publish and install the compatible CLI/daemon build on every project host. On startup/reconnect it prevents restoration, gracefully stops any active retired pane, and reports unsupported historical panes without touching supported siblings.
4. Deploy the web after server and hosts. Verify launch/team selectors, policy editing, Machines, usage, and pane actions expose no retired operation.
5. Verify database allowlists, effective-policy broadcasts, absence of retired outbound configuration/usage messages, explicit stale-client rejection, and successful launches for every retained provider family.
6. Keep compatibility tombstones for one release. A later separately reviewed cleanup may remove them after fleet/version and persisted-state audits show they are no longer needed.

Rollback restores the database backup and previous server, CLI, and web artifacts together. Because the removal deliberately stops retired panes and policy migration changes allowlists, rollback does not automatically restart compute; operators must explicitly restart any restored provider panes after confirming credentials and policy.
