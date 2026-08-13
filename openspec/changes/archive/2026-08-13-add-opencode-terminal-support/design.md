## Context

See `proposal.md` for motivation. APAS already carries an `Opencode` provider variant, configuration key, legacy headless launch path, web branding, terminal transport, server-authoritative launch policy, and mobile task-launch orchestration. The missing boundary is verified terminal hosting: current user-created panes are PTY-based, while the pre-existing OpenCode adapter assumes command-line and JSON behavior that no longer matches the supported OpenCode CLI.

The OpenCode CLI owns `ses_*` session IDs and exposes session discovery/export commands instead of a stable transcript path APAS can derive from its pane UUID. Rolling deployment also matters: older APAS project CLIs understand mobile terminal launch but cannot host OpenCode, so the generic mobile capability is insufficient.

## Goals / Non-Goals

**Goals:**

- Make OpenCode a first-class unmanaged terminal provider across desktop and mobile launch surfaces.
- Preserve the conversation-mode experience by recovering completed OpenCode turns and usage.
- Fail closed under project policy and mixed-version deployments.
- Keep retained legacy/headless OpenCode panes decodable and operational.

**Non-Goals:**

- Adding OpenCode to managed team-role MCP wiring or making terminal panes delegation targets.
- Installing, authenticating, or configuring an OpenCode model on behalf of the user.
- Guaranteeing perfect pane/session attribution when multiple OpenCode terminals share the same working directory.
- Persisting raw PTY scrollback beyond the existing in-memory terminal lifecycle.

## Decisions

### Host the real OpenCode TUI through the existing terminal PTY

OpenCode joins the same terminal-host allowlist as Claude and Codex. Fresh launches use OpenCode's initial-prompt option and automatic approval mode; restored panes use its continuation option and do not replay an old prompt. The configured `opencode_path` remains the binary source so installations outside `PATH` continue to work.

Alternative considered: keep OpenCode on the structured conversation-only path. That path is retired for new user panes and would provide a materially different experience from Claude/Codex, so it is retained only for historical/managed compatibility.

### Recover history through directory-scoped session list and export

The watcher queries OpenCode's JSON session list, selects the newest session whose recorded directory exactly equals the pane cwd, and exports that session by its native ID. Parsing emits user messages immediately but withholds assistant messages until OpenCode records completion; it filters internal typed parts and carries model/token metadata through the existing conversation adapter.

Alternative considered: assign the APAS pane UUID with OpenCode's session flag. OpenCode session identifiers are generated in its own `ses_*` namespace, so treating an APAS UUID as an OpenCode session ID is invalid. Reading OpenCode's database directly was also rejected because the supported CLI export interface is less coupled to storage internals.

### Use an explicit provider capability for rolling compatibility

The upgraded CLI advertises `terminal_opencode_v1`. Desktop and mobile OpenCode creation check that capability before routing, while Claude/Codex remain available to older clients through their existing capabilities. This avoids persisting a dead pane on an old CLI during a server-first rollout.

Alternative considered: bump the generic mobile task-launch capability. That would unnecessarily disable working Claude/Codex mobile launch on every older client.

### Keep policy opt-in semantics for existing explicit allowlists

OpenCode is added to the supported-profile catalog and fresh default policy, but existing explicit cluster and project allowlists are not silently broadened. Administrators can enable `terminal:opencode:official:default` through the normal policy surface.

Alternative considered: migrate every existing allowlist to include OpenCode. Because terminal providers run coding agents with automatic permission approval, silently broadening an administrator's explicit allowlist is not appropriate.

### Translate native JSON events for legacy headless panes

The retained `opencode run --format json` path uses the documented automatic permission and continuation options. A dedicated adapter maps text, tool use, final usage/completion, and errors to the shared message model. Tool-call step finishes are not terminal completion boundaries.

Alternative considered: deserialize OpenCode events as Claude stream messages. The formats are not compatible and silently lose visible output and status.

## Risks / Trade-offs

- **[Two OpenCode panes share one cwd]** → Session discovery can attribute the newest session to both panes; document the ambiguity, require exact cwd matching, and never cross project/worktree directories.
- **[Session export polling adds subprocess overhead]** → Poll only terminal OpenCode panes on the existing low-frequency transcript watcher and request a bounded recent session list.
- **[OpenCode CLI changes its supported interface]** → Pin regression tests to documented command shapes and event fixtures, and return actionable spawn/export errors instead of falling back to another provider.
- **[Automatic permission mode increases authority]** → Honor explicit OpenCode deny rules, retain server-authoritative project policy, and do not auto-enable the profile in existing explicit allowlists.
- **[Older CLI receives an unsupported request]** → Gate routing with `terminal_opencode_v1` and require reconnection after upgrade.

## Migration Plan

1. Deploy the server first so it understands the new profile and capability while refusing OpenCode routing to old CLIs.
2. Deploy the web so desktop launch surfaces can display the server-authorized OpenCode option.
3. Release/install the upgraded APAS CLI on project hosts and reconnect it so `terminal_opencode_v1` is advertised.
4. Install and authenticate OpenCode on each intended host, configure `opencode_path` when necessary, and explicitly enable the terminal profile in existing cluster/project policy.
5. Verify desktop creation, mobile task launch, PTY input/output, conversation capture, completion state, and restoration.

Rollback is server/web/CLI compatible: removing the web option prevents new requests; an older server ignores the additive CLI capability; persisted OpenCode panes remain readable as provider values but cannot be relaunched by a CLI without terminal support. Administrators can disable the profile immediately through policy before rolling binaries back.
