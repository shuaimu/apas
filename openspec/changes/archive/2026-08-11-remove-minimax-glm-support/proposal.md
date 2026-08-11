## Why

MiniMax and GLM currently require dedicated credential, runtime-bridge, usage-monitoring, protocol, policy, and web UI paths even though APAS no longer intends to offer those backends. Removing them reduces the supported surface area and prevents users or persisted policy from continuing to launch configurations the system no longer supports.

## What Changes

- **BREAKING** Remove MiniMax and GLM from every pane, managed-team, provider/model, backend-switch, and launch-policy choice; all other currently supported choices, including Claude, Codex, DeepSeek, OpenCode, Cursor Agent, and terminal panes, remain available.
- **BREAKING** Reject new, restart, resume, model-switch, and team launch requests that name a MiniMax or GLM provider, backend, model, or retired launch-profile key.
- Remove MiniMax/GLM API-key and machine-configuration controls, daemon configuration messages, runtime environment injection, usage polling/caching/reporting, and provider-specific status presentation.
- Remove MiniMax/GLM profiles from the server-owned supported-profile registry and migrate cluster-default and project-override allowlists so retired keys cannot remain effectively allowed.
- Preserve bounded legacy decoding for existing `.apas` files and mixed-version messages only where needed to avoid crashes: affected panes are shown as unsupported and remain stopped/non-relaunchable, with no silent remapping to another provider.
- Gracefully interrupt and stop any MiniMax or GLM pane that is still running when an upgraded project host applies the retirement; supported panes in the same project are left intact.
- Stop reading or transmitting legacy MiniMax/GLM credentials. Existing keys left in older local configuration files are inert and can be removed manually; the migration does not log, expose, or copy them.
- Remove provider-specific feature documentation and tests, add retirement/migration/compatibility coverage, and verify no selectable, configurable, or operational MiniMax/GLM path remains; retirement errors and historical unsupported-state labels may still name the retired provider.

## Capabilities

### New Capabilities

- `provider-support`: Defines the supported APAS provider/backend catalog, complete MiniMax/GLM retirement behavior, policy cleanup, and safe handling of legacy persisted values.

### Modified Capabilities

None. There are no existing main OpenSpec capabilities in this planning root to modify.

## Impact

- Affects shared provider and WebSocket message types, stable launch-profile keys, CLI configuration and process spawning, daemon machine state, server routing/session state, usage-limit collection, and policy migration.
- Affects web provider/model selectors, managed-team setup, pane controls, machine configuration, usage displays, tests, and operator/developer documentation.
- Existing MiniMax/GLM panes become unsupported historical configuration and cannot be launched again; other panes in the same project remain usable.
- Mixed-version clients may still send retired values, so the upgraded server and CLI must reject those requests explicitly without panicking or launching a fallback process.
