## Context

See `proposal.md` — Why.

APAS already models terminal providers as a pane kind plus a provider: the CLI resolves a binary, builds a provider argument vector, hosts the real TUI on a pty, and recovers history from the provider's own transcript. Pi fits that shape, but two details differ from every existing provider:

- Pi sessions are plain JSONL under `~/.pi/agent/sessions/--<cwd-with-separators-replaced>--/<timestamp>_<uuid>.jsonl`, with a `type:"session"` header carrying `id` and `cwd`, `type:"message"` entries whose assistant messages carry `stopReason` and `usage` (including real USD `cost`), and non-conversation entries for thinking, tool results, shell execution, extension state, compaction, and branching.
- Pi accepts `--session-id <uuid>` to use an exact project session identity, creating it when missing, and `--session <path|id>` to open an existing session. `--session <id>` can prompt to fork when the id resolves in a different project; an absolute path opens directly.

The oh-my-pi package is a Pi extension, not an agent: it peer-depends on `@earendil-works/pi-coding-agent`, is loaded by Pi's extension discovery, and ships only a diagnostic `oh-my-pi doctor|init|version` CLI. Supporting it therefore means supporting Pi and validating oh-my-pi inside Pi panes, not adding a provider.

## Goals / Non-Goals

**Goals:**

- Make Pi a first-class unmanaged terminal provider across desktop and mobile launch surfaces.
- Give each Pi pane exact session identity so history and restoration never depend on directory recency.
- Recover conversation, completion state, token usage, and recorded cost from Pi session files, including with oh-my-pi's extension entries present.
- Fail closed under project policy and mixed-version deployments.
- Document and verify oh-my-pi as an ordinary installed Pi package.

**Non-Goals:**

- Installing, authenticating, or configuring Pi or oh-my-pi on a user's behalf, and choosing a Pi provider or model.
- Automatically trusting project-local Pi extensions or settings.
- Following in-TUI `/new` or `/resume` session switches in this change.
- Reporting subscription usage windows for Pi; Pi exposes per-turn tokens and cost, not a quota window.
- Adding a distinct provider or launch profile for oh-my-pi.

## Decisions

### Pin the exact Pi session identity on fresh launch

A fresh Pi terminal launches with `--session-id <pane conversation id>` and the initial instruction as a positional message. Both the conversation identity APAS already persists and the provider session identity are then the same value, so recovery and restore need no discovery heuristic.

Alternative considered: OpenCode-style newest-session-in-directory discovery. Rejected: multiple Pi panes can share a working directory, and oh-my-pi can spawn additional Pi processes, so the newest file in a directory cannot be attributed to a pane with confidence.

### Restore by exact file, with pinned identity as fallback

The transcript locator resolves the pane's `<uuid>.jsonl` under the Pi sessions root (default `~/.pi/agent/sessions`, honoring `PI_CODING_AGENT_DIR` and session-directory overrides). When found, restore passes `--session <absolute path>`; when it is absent — a session is not written until its first assistant message — restore passes `--session-id <uuid>` again and Pi recreates it under the same identity.

Alternative considered: `--session <id>`. Rejected because an id that resolves in a different project triggers an interactive fork prompt, which would block a browser-driven pane. `--continue` was also rejected: it selects by directory recency, which is the attribution problem pinning removes.

### Do not pass a project-trust or permission override

Pi has no tool permission prompts, so there is no bypass flag to pass. Its `--approve` flag trusts project-local settings and extensions, which are repository-controlled code; APAS must not grant that on the user's behalf. The practical consequence is documented: install pi packages such as oh-my-pi globally so the trust prompt never blocks a pane, or answer the one-time trust prompt in the terminal view.

Alternative considered: `--approve` for a prompt-free mobile experience. Rejected: it converts any repository with a project-local Pi extension into code execution at pane launch.

### Parse Pi session JSONL, excluding extension and bookkeeping entries

The parser keeps `type:"message"` entries with roles `user` and `assistant`. Assistant text comes from `text` content blocks; thinking and tool-call blocks are skipped, as are tool results, shell executions, `custom` and `custom_message` extension entries, compaction and branch summaries, labels, and model/thinking-level changes. `completes_work` derives from `stopReason`: `stop` completes, `toolUse` and non-terminal reasons do not. Usage maps input/output tokens and, when present, cost becomes the pane's recorded `total_cost_usd` (the terminal `TurnRecord` gains an optional cost field; providers without cost keep zero).

Excluding extension entries is what keeps oh-my-pi safe: its orchestrator state is persisted as extension entries and must never render as conversation.

### Keep structured questions out of scope until a picker can be verified
Upstream Pi has no built-in structured question tool; the `ask` tool belongs to the oh-my-pi package, and its picker contract has not been driven against a real TUI. The answer path writes keystrokes into a live pty, so an unverified picker would let a web click answer on the human's behalf. Pi questions are therefore not published as answerable cards in this change — matching OpenCode, which also has no question support. Assistant text that asks a question still appears as ordinary conversation, and the pane's terminal view remains the place to answer it.

### Keep in-TUI session switching out of scope

APAS watches the pinned session file and does not adopt newer files in the same directory. The failure mode this avoids — publishing a sibling pane's or an oh-my-pi subagent's conversation as this pane's history — is exactly why the Claude fallback was replaced with hook-reported transcripts. Consequence: a user who runs `/new` or `/resume` inside the TUI sees that conversation only in the terminal; APAS history continues from the pinned session, and a pane reboot returns to it. Documented as a known limitation.

### Gate Pi behind an explicit CLI capability

The upgraded CLI advertises `terminal_pi_v1`. Server and mobile launch authorization require it when routing a Pi launch, so a server-first rollout cannot persist a Pi pane on an older CLI that would fail to host it. Claude, Codex, and OpenCode routing is untouched.

### Keep existing launch allowlists unchanged

Pi joins the supported-profile catalog and the fresh default policy, but existing explicit cluster and project allowlists are not silently broadened; `cluster_settings` is seeded with `INSERT OR IGNORE` and policy normalization only removes or remaps existing entries. Administrators enable `terminal:pi:official:default` through the normal policy surface.

## Risks / Trade-offs

- **[Pi has no permission system]** → The launch-profile allowlist and the project host's independent re-check are the only controls; the profile is not added to existing allowlists automatically, and no trust bypass is passed.
- **[In-TUI `/new` or `/resume` is not mirrored]** → Pinned sessions are exact and cross-pane-safe; the limitation is documented, and a follow-up change can add switch detection if Pi exposes a reliable signal.
- **[Answering Pi questions from the conversation view is not supported]** → Publishing an unverified question card and writing blind keystrokes could answer on the human's behalf; Pi questions stay in the terminal until a provider-confirmed interface can be driven and verified. A follow-up change can add it with the same confirm-by-transcript rule Claude uses.
- **[Session file exists only after the first completed assistant message]** → Locator and watcher treat absence as "no history yet"; restore recreates the pinned identity rather than adopting another session.
- **[Pi version drift in CLI flags]** → Pin argument shapes with unit tests against the documented CLI interface and keep an actionable spawn error; document a minimum Pi version that supports exact session identity.
- **[Server-first rollout]** → Older CLIs reject Pi routing through the capability gate with an update-and-reconnect error; older servers ignore the additive capability and the hidden web option.

## Migration Plan

1. Deploy the server first: it understands `Provider::Pi`, the catalog entry, and the capability gate, and refuses to route Pi to project CLIs that lack `terminal_pi_v1`.
2. Deploy the web so desktop launch surfaces can display the server-authorized Pi option.
3. Release and reconnect the upgraded project CLI so it advertises `terminal_pi_v1`.
4. On intended hosts: install Pi, authenticate it, optionally `pi install npm:oh-my-pi`, set `pi_path` when nonstandard, and explicitly enable `terminal:pi:official:default` in cluster or project policy.
5. Verify desktop creation, mobile task launch, pty input/output, history, usage, restoration, and oh-my-pi behavior.

Rollback is server/web/CLI compatible: removing the web option prevents new requests; an older server ignores the added capability; persisted Pi panes remain readable but cannot be relaunched by a CLI without terminal support; administrators can disable the profile through policy before rolling binaries back.
