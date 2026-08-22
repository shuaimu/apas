## Context

See `proposal.md` for motivation. The current launch registry, web provider options, and CLI environment builder encode one canonical DeepSeek model, `deepseek-v4-pro`. Provider and model already travel through the shared pane messages, and any DeepSeek model causes the Claude child to receive the configured endpoint and API key. Existing policy is an exact launch-profile allowlist and intentionally never widens persisted administrator choices during an upgrade.

The existing pane-model-selection contract removes generic model controls from pane cards but preserves a combined agent frontend/API backend control. This change extends that combined choice only for the two supported DeepSeek variants; it does not reintroduce arbitrary model text or a toolbar model selector.

## Goals / Non-Goals

**Goals:**

- Represent Pro and Flash as primary-model variants of one Claude/DeepSeek backend.
- Keep one API key and endpoint per machine.
- Preserve Pro as the compatibility default and use Flash for Claude Code small/Haiku and subagent routing.
- Make launch and switching behavior policy-filtered and fail closed at both server and CLI boundaries.
- Preserve existing Pro panes and existing policy non-widening guarantees.

**Non-Goals:**

- Adding a DeepSeek terminal-pane provider.
- Adding arbitrary model-name entry, automatic cost routing, or per-turn automatic Pro escalation.
- Changing DeepSeek thinking-mode or reasoning-effort controls.
- Creating a second DeepSeek key, endpoint, machine profile, or wire protocol field.
- Changing generic model-selection behavior for official Claude, Codex, OpenCode, or Cursor panes.

## Decisions

### Use canonical API IDs in pane state and policy keys

Pane metadata and launch profiles will store `deepseek-v4-pro` or `deepseek-v4-flash`. Pro remains the value behind the existing generic DeepSeek choice, so current pane rosters and labels keep their meaning. The runtime may add Claude Code's documented `[1m]` selector where required, but that transport-specific suffix will not become a policy identity or persisted pane model.

Alternative considered: store Claude aliases such as Opus and Sonnet. Rejected because aliases obscure the actual billed DeepSeek model and make policy enforcement depend on upstream mapping behavior.

### Use one backend option with two nested model choices

The web provider catalog will expose DeepSeek Pro and Flash beneath Claude/DeepSeek. The retained Overview combined selector and policy surfaces will use the same shared option definitions, labels, reverse mapping, and policy filter. New user work remains terminal-only, so this change will not revive a structured-pane creation surface. The controls will not expose a second credential profile or an arbitrary model selector.

Alternative considered: duplicate `Claude / DeepSeek Pro` and `Claude / DeepSeek Flash` as unrelated top-level profiles. Rejected because endpoint, credential, usage account, and runtime are identical.

### Pin the primary model and map secondary work to Flash

For a DeepSeek pane the CLI will always set the primary model explicitly from pane metadata. It will also configure Claude Code's small/Haiku and subagent model variables to `deepseek-v4-flash`. Pro-primary panes will use DeepSeek's documented Pro selector for their main request; Flash-primary panes will use Flash. Claude model aliases will be assigned deterministically so no unsupported name can reach DeepSeek and be silently mapped by the upstream API.

Alternative considered: set every Claude model variable to the selected primary model. Rejected because that makes Pro subagents unnecessarily expensive and does not follow DeepSeek's recommended mixed routing.

### Treat Flash/Pro changes as process-level switches

Model selection is supplied through child-process environment, so a running structured Claude process cannot safely adopt the other DeepSeek model through APAS's official-Claude live flag path. A confirmed switch will interrupt the current child, generate a fresh provider session ID, respawn with the new environment, and retain only APAS's visible transcript. Persistence and server pane state are updated before the replacement is launched.

Alternative considered: reuse the provider session across the model switch. Rejected because it risks backend session incompatibility and implies context continuity APAS cannot guarantee.

### Add Flash without automatically widening stored policy

The supported registry and fresh in-code default will contain both profile keys. Existing deployment, cluster, and project allowlists remain unchanged until an administrator explicitly adds Flash. The production deployment step will add Flash only to the requested default and currently eligible project rows through the normal authorized policy path, preserving intentionally narrow terminal-only projects.

Alternative considered: a startup migration that appends Flash wherever Pro appears. Rejected because policy migrations must not silently grant a newly introduced billed capability.

## Risks / Trade-offs

- [Claude Code changes accepted custom-model environment semantics] → Cover every emitted environment variable with unit tests and run a bounded real bridge smoke test before deployment.
- [A stale client can submit an arbitrary DeepSeek-looking model] → Keep the canonical registry exact and enforce policy independently on server and project host; do not accept substring matching as authorization.
- [Switching loses provider prompt context] → Require confirmation, state the consequence, allocate a fresh provider session, and retain the visible APAS transcript.
- [Pro startup can incur high context/title-generation cost] → Route small/title/subagent aliases to Flash and include model/cost attribution in the smoke-test acceptance evidence.
- [Existing policy rows do not show Flash after code deployment] → Treat explicit production policy updates and verification as deployment tasks rather than weakening non-widening migration behavior.
- [Duplicated web option tables drift] → Centralize Pro/Flash constants and option metadata in `providerOptions.ts`, then make Overview and launch surfaces consume them with cross-language drift tests.

## Migration Plan

1. Ship the registry, CLI routing, UI, and tests while keeping Pro as the existing default.
2. Verify fresh/default policy serialization includes both variants and persisted explicit allowlists remain unchanged.
3. Build and deploy server, web, and CLI together so all boundaries recognize Flash before policy enables it.
4. Through the authorized policy path, add Flash to the production deployment default and to project overrides that already permit DeepSeek model work; do not alter terminal-only overrides.
5. Restart/reconnect clients, verify both choices appear only where allowed, and run one bounded Flash smoke request plus the existing Pro check without exposing the key.
6. Roll back binaries/web and restore the pre-change policy allowlists if model routing or policy enforcement fails; pane metadata remains readable because it already carries an optional model string.
