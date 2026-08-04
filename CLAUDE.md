# APAS - Autonomous Programming Agent System

> Canonical team-mode contributor/agent runbook. Keep architecture, local
> development, and team-role workflow guidance here. `agent.md` is a generated
> pointer to this file; `claude.md` and `AGENTS.md` are deployment-only notes.

APAS runs a local autonomous programming team around a project. The CLI owns
local panes and worktrees, the server brokers project/session state, and the
web UI exposes the Overview, Manager chat, Team TODO queue, pane tabs, and
diff/PR handoff surfaces.

The current v3 operating model has four managed roles:

- **Manager** chats with the human and keeps `project_goal.md` current.
- **Tech Lead** converts the project goal into approved work in
  `team-todo.md`, dispatches developers/reviewers through
  `.apas-team.jsonl`, and tracks worker-opened PRs.
- **Developer** panes implement approved subtasks in isolated worktrees,
  publish `kind: "diff"` records, and open PRs after Reviewer approval.
- **Reviewer** panes evaluate worker diffs and publish `approves:<pane>` or
  `rejects:<pane>` review records.

## Architecture

```
┌─────────────────────┐    WebSocket     ┌─────────────────┐    WebSocket    ┌─────────────────────┐
│ CLI client          │ ◄───────────────►│ apas-server     │◄───────────────►│ Web frontend        │
│ (apas binary)       │                  │                 │                 │ (Next.js Overview)  │
└─────────────────────┘                  └─────────────────┘                 └─────────────────────┘
        │                                         │
        │                                         ▼
        │                                ┌─────────────────┐
        │                                │ SQLite + JSONL  │
        │                                │ data/sessions/  │
        │                                └─────────────────┘
        ▼
┌─────────────────────┐
│ Pane processes      │
│ Claude/Codex/etc.   │
│ worktrees + prompts │
└─────────────────────┘
```

The CLI keeps project-local files such as `.apas`, `project_goal.md`,
`team-todo.md`, `.apas-team.jsonl`, and optional worker worktrees. The server
caches and broadcasts machine, session, TODO, suggestion, and project-goal
state. The web UI lets the human manage goals, approve proposed TODOs,
inspect panes, and review PR links/status.

## Project Structure

```
apas/
├── crates/
│   ├── client-cli/      # CLI binary (apas)
│   │   ├── src/
│   │   │   ├── main.rs        # CLI entry point and config commands
│   │   │   ├── config.rs      # User/machine config, provider API keys
│   │   │   ├── project.rs     # .apas project metadata
│   │   │   ├── role.rs        # Built-in Manager/Tech Lead/Developer/Reviewer prompts
│   │   │   ├── team_todo.rs   # team-todo.md parsing/state helpers
│   │   │   ├── manager.rs     # project_goal.md read/write helpers
│   │   │   ├── worktree.rs    # Isolated worktree creation/diff/cleanup
│   │   │   ├── claude.rs      # Claude process wrapper
│   │   │   ├── terminal_pane.rs # Pty host for kind:"terminal" panes (portable-pty)
│   │   │   └── mode/
│   │   │       ├── dual_pane.rs # Default managed panes, deadloops, team files
│   │   │       ├── hybrid.rs    # Legacy single-pane local CLI + streaming
│   │   │       ├── local.rs     # Offline mode
│   │   │       └── remote.rs    # Remote-only mode
│   │   └── Cargo.toml
│   │
│   ├── server/          # WebSocket server (apas-server)
│   │   ├── src/
│   │   │   ├── main.rs      # Server entry point
│   │   │   ├── state.rs     # AppState with DB, sessions, storage
│   │   │   ├── storage.rs   # File-based message storage (JSONL)
│   │   │   ├── db/          # SQLite database
│   │   │   ├── session/     # Session manager
│   │   │   └── routes/
│   │   │       ├── ws_cli.rs  # CLI WebSocket handler, project-goal/TODO replay
│   │   │       └── ws_web.rs  # Web WebSocket handler, Overview/machine actions
│   │   └── Cargo.toml
│   │
│   └── shared/          # Shared types between CLI and server
│       ├── src/
│       │   ├── lib.rs
│       │   └── messages.rs  # Shared WebSocket/team/machine message types
│       └── Cargo.toml
│
├── packages/
│   └── web/             # Next.js web frontend
│       ├── src/
│       │   ├── app/
│       │   │   ├── layout.tsx
│       │   │   └── page.tsx
│       │   ├── components/
│       │   │   ├── overview/         # Manager/goal/TODO/pane-grid control surface
│       │   │   ├── tabs/             # Pane tabs, task bars, diff modal
│       │   │   │   └── TerminalPane.tsx  # xterm.js view for kind:"terminal" panes
│       │   │   ├── chat/             # Message display
│       │   │   ├── code/             # Code blocks
│       │   │   └── tools/            # Tool cards
│       │   └── lib/
│       │       ├── store.ts          # Zustand state, WebSocket message handling
│       │       ├── terminalBus.ts    # Pty frame fan-out (deliberately NOT zustand)
│       │       └── roleTemplates.ts  # Web-spawned managed-role prompt templates
│       └── package.json
│
├── data/                # Runtime data (created at runtime)
│   ├── apas.db          # SQLite database
│   └── sessions/        # Message storage
│       └── {session-id}/
│           └── messages.jsonl
│
├── Cargo.toml           # Workspace root
└── CLAUDE.md            # This file
```

## Build Commands

```bash
# Build all Rust crates
cargo build

# Build specific crate
cargo build -p apas          # CLI
cargo build -p apas-server   # Server
cargo build -p shared        # Shared types

# Run server
cargo run -p apas-server

# Run CLI (in a project directory)
cargo run -p apas

# Run CLI in offline mode (no server)
cargo run -p apas -- --offline

# Web frontend (from packages/web/)
npm install
npm run dev
```

### CLI build target (static musl)

The shipped CLI is a **static musl** binary so a self-update rebuild never
leaves `apas` depending on the build machine's glibc version. `install.sh` and
the self-updater (`crates/client-cli/src/update.rs`) build it with
`--target <arch>-unknown-linux-musl`; the server and plain `cargo build` stay
on the host's glibc target (musl is deliberately **not** a global
`build.target`, which would drag the server's bundled SQLite/ring through musl
too). The CLI's TLS is rustls (not native-tls/OpenSSL) so nothing but ring's
small C shim needs the musl toolchain.

Building the CLI as it ships needs the musl target plus a musl C compiler:

```bash
rustup target add x86_64-unknown-linux-musl   # (aarch64-… on ARM)
sudo apt-get install -y musl-tools            # provides musl-gcc + musl-dev

cargo build --release --target x86_64-unknown-linux-musl -p apas
```

`.cargo/config.toml` points ring's C build at `musl-gcc` for the musl targets.
If the musl toolchain is missing, `install.sh` and the self-updater fall back
to a glibc build so an update can never brick the binary.

## Configuration

### CLI Config
Located at `~/.config/apas/config.toml`:
```toml
[remote]
server = "ws://apas.mpaxos.com:8080"
token = "your-token"

[local]
claude_path = "claude"
```

### Project Identification
Each project directory gets a `.apas` file with project metadata and restored
pane state:
```json
{
  "id": "uuid",
  "name": "project-name",
  "created_at": "2024-01-01T00:00:00Z",
  "team_enabled": false,
  "auto_approve_todos": false,
  "auto_merge_prs": false,
  "panes": [
    {
      "pane_id": 151,
      "role": "team manager",
      "mode": "interactive",
      "managed": true
    },
    {
      "pane_id": 178,
      "role": "tech lead",
      "mode": "deadloop",
      "managed": true
    },
    {
      "pane_id": 568,
      "role": "developer",
      "mode": "deadloop",
      "managed": true,
      "worktree_path": null
    },
    {
      "pane_id": 440,
      "role": null,
      "mode": "interactive",
      "managed": false
    },
    {
      "pane_id": 612,
      "role": null,
      "mode": "interactive",
      "kind": "terminal",
      "provider": "codex",
      "managed": false
    }
  ]
}
```

`team_enabled`, `auto_approve_todos`, `auto_merge_prs`, and
`disallowed_tab_types` are project-level policy flags. `team_enabled` gates
managed team mode entirely (see "Team mode is opt-in" below);
`disallowed_tab_types` restricts which tab types users may create (see "Tab-type
policy"); the other two are read by the Tech Lead loop. All are owner/admin-only. Managed pane entries are restored as team roles; unmanaged
interactive panes can coexist with the team. `kind` defaults to `"agent"` when
absent, so `.apas` files written before terminal panes existed keep loading
unchanged — see "Terminal panes" under Key Concepts.

## Message Types

Key message types in `crates/shared/src/messages.rs`:

- **CliToServer**: Register, SessionStart, StreamMessage, UserInput,
  Heartbeat, ProjectGoalChanged, ProjectFlagsChanged, TeamTodoChanged,
  SuggestedWorkersChanged, TerminalOutput, TerminalExited, machine
  config/status updates.
- **ServerToCli**: Registered, SessionAssigned, Input, Signal,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  TerminalInput, TerminalResize, pane/worktree/suggestion actions.
- **WebToServer**: Authenticate, ListCliClients, AttachSession, Input,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  TerminalInput, TerminalResize, TerminalAttach, machine/provider config
  actions.
- **ServerToWeb**: Authenticated, CliClients, SessionMessages, StreamMessage,
  UserInput, ProjectGoalChanged, ProjectFlagsChanged, TeamTodoChanged,
  SuggestedWorkersChanged, Machines, PaneDiff, TerminalOutput,
  TerminalSnapshot, TerminalExited.

The `Terminal*` family is the pty byte channel for `kind: "terminal"` panes and
is deliberately separate from `Output` / `StreamMessage` — see "Terminal panes"
under Key Concepts for why.

`WebToServer::UpdateProjectFlags` carries the project policy flags
(`team_enabled`, `auto_approve_todos`, `auto_merge_prs`) from the web to the
server. The server **rejects the whole message from anyone below admin**
(`ws_web::can_manage_project_settings`) — this is the only role gate in the
WebSocket layer, everything else there authorizes on session *access* alone.
It then forwards `ServerToCli::UpdateProjectFlags` to the CLI for `.apas`
persistence; the CLI emits `CliToServer::ProjectFlagsChanged`, and the server
broadcasts `ServerToWeb::ProjectFlagsChanged`. The CLI also re-broadcasts the
flags from `.apas` every 5s, so a web client attaching mid-session hydrates
without asking. Behavioral safeguards for the autonomy flags are documented in
the role prompts and README.

## Data Storage

- **SQLite** (`data/apas.db`): Users, CLI clients, sessions metadata
- **JSONL files** (`data/sessions/{id}/messages.jsonl`): Chat messages per session
- **Project files** (`project_goal.md`, `team-todo.md`, `.apas-team.jsonl`):
  team-mode goal, work queue, and append-only cross-pane scratchpad
- **Worktrees** (`.apas-worktrees/pane-<id>/`): isolated branches for managed
  Developer panes

## Team-mode Operations

The v3 team loop is driven by project-local files:

- `project_goal.md` is the high-level goal. The Manager updates it from human
  chat, and the Tech Lead reads it before proposing work.
- `team-todo.md` contains Global TODOs plus per-pane subtasks. Tech-Lead
  proposals start as `status: proposed, origin: tech-lead`; the human
  approves/rejects them in the Overview Team TODO panel.
- `.apas-team.jsonl` is the append-only scratchpad. Tech Lead uses
  `kind: "delegation"` with `delegate-to:<pane>` and `task:TODO-NNN` tags;
  Developers publish `kind: "diff"`; Reviewers publish `kind: "review"` with
  `approves:<pane>` or `rejects:<pane>`; Developers publish
  `kind: "decision"` with `pr-opened` after opening a PR.
- Worker PRs are opened by the Developer pane after Reviewer approval. The
  human merges or closes the PR; Tech Lead tracks PR state and dispatches PR
  comments back to the owning Developer.

For contributor work, keep task scopes narrow. If a TODO says docs-only or
names specific files, do not combine it with adjacent team-mode cleanup.

## Development

### Running locally
```bash
# Terminal 1: Server
RUST_LOG=info cargo run -p apas-server

# Terminal 2: Web frontend
cd packages/web && npm run dev

# Terminal 3: CLI (in any project directory)
cargo run -p apas
```

### Environment Variables
- `RUST_LOG`: Logging level (e.g., `info`, `debug`)
- `NEXT_PUBLIC_WS_URL`: WebSocket URL for web frontend (default: `ws://apas.mpaxos.com:8080`)

## Deployment

### Production Server

The APAS server and web UI are deployed on an LXC container:

- **Host**: `apas.mpaxos.com` (130.245.173.82)
- **SSH**: `ssh root@apas.mpaxos.com`
- **Edge**: **nginx** on port **80** reverse-proxies everything. The config is
  version-controlled at **`deploy/nginx-apas.conf`** and deployed to
  `/etc/nginx/conf.d/apas.conf` — edit the repo copy, `scp` it, `nginx -t`,
  then `systemctl reload nginx`. `/ws/ /auth/ /admin/ /share/ /health` →
  `apas-server` (`127.0.0.1:8080`); everything else → the Next.js app
  (`127.0.0.1:3000`). This exists so the web's WebSocket + HTTP API ride the
  standard port 80 (`ws://apas.mpaxos.com/ws/web`) instead of the non-standard
  `:8080`, which mobile carriers/Wi-Fi block — that broke mobile entirely.

  **A proxied `location /X/` shadows the page at `/X`.** nginx answers a
  request for a proxied prefix location *minus* its trailing slash with a 301
  that appends one. So `/share` was redirected to `/share/`, which then matched
  the API prefix and 404ed on `apas-server` — every share invite was dead
  (`/share?code=...`), and the whole `/admin` page with it, while the
  `/share/*` API endpoints looked perfectly healthy the entire time. The fix is
  an exact-match location, which outranks the prefix and is exempt from the
  redirect:

  ```nginx
  location = /share { proxy_pass http://127.0.0.1:3000; ... }
  ```

  `/auth`, `/admin`, and `/share` all have one. **Add another whenever you add
  an API prefix that shares a name with a Next.js page**, and check
  `curl -sL -o /dev/null -w '%{http_code} %{num_redirects}' <url>` — a page
  answering `200 1` instead of `200 0` is this bug.
- **apas-server**: `127.0.0.1:8080` — still also bound publicly on `:8080` for
  the **CLI/daemon** (`ws://apas.mpaxos.com:8080/ws/cli`); do not firewall 8080.
- **Next.js web (apas-web)**: `127.0.0.1:3000` (moved off 80 via a systemd
  drop-in `apas-web.service.d/port.conf` → `Environment=PORT=3000`).
- The web's default WS/API URLs are now **port-less** (`ws://apas.mpaxos.com`,
  `http://apas.mpaxos.com`); `NEXT_PUBLIC_WS_URL`/`NEXT_PUBLIC_API_URL` no
  longer need to be set at build time.

#### Directory Structure on Server
```
/opt/apas/
├── apas-server         # Server binary
├── data/
│   ├── apas.db         # SQLite database
│   └── sessions/       # Message storage
└── web/                # Next.js web frontend
```

#### Systemd Services
```bash
# Check status
systemctl status apas-server
systemctl status apas-web

# Restart services
systemctl restart apas-server
systemctl restart apas-web

# nginx edge proxy (port 80). After editing /etc/nginx/conf.d/apas.conf:
nginx -t && systemctl reload nginx

# View logs
journalctl -u apas-server -f
journalctl -u apas-web -f
journalctl -u nginx -f
```

#### Deploying Updates

```bash
# Build locally
# cargo build -p apas-server --release
cargo build --release

# Stop server
ssh root@apas.mpaxos.com "systemctl stop apas-server"

# Copy binary to server
scp target/release/apas-server root@apas.mpaxos.com:/opt/apas/

sleep 1

# Restart server
ssh root@apas.mpaxos.com "systemctl restart apas-server"

sleep 1

# For web updates. `--delete` is important: without it, files removed
# locally (renamed/retired components) stay on the server and the next
# `npm run build` fails because those files still reference removed
# store actions or types. Use `-n` (dry-run) first if you're not sure
# what will get cleaned up.
rsync -av --delete --exclude 'node_modules' --exclude '.next' --exclude '.apas-version' packages/web/ root@apas.mpaxos.com:/opt/apas/web/

# Restart apas-web (compute version locally since server lacks git history).
# `npm run build` runs `prebuild` first (`eslint .`, packages/web/eslint.config.mjs),
# which fails the build on React Rules-of-Hooks violations (e.g. a hook after an
# early return -> React error #310 -> whole-app crash). So a lint failure here
# aborts the deploy before restart — fix the reported hook error, don't bypass it.
# Requires devDependencies installed (the plain `npm install` above does this).
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm install && NEXT_PUBLIC_WEB_UI_VERSION=${web_version} npm run build && systemctl restart apas-web"
```

## Key Concepts

1. **Managed Team Mode** (opt-in, off by default): Manager, Tech Lead,
   Developer, and Reviewer panes coordinate through `project_goal.md`,
   `team-todo.md`, and `.apas-team.jsonl`. Gated on `team_enabled` — see
   "Team mode is opt-in" below.
2. **Dual-Pane Runtime**: The CLI restores panes from `.apas`, runs deadloop
   or interactive processes, and can isolate Developer panes in git
   worktrees.
3. **Hybrid Mode** (legacy): Single pane with local terminal + streaming.
4. **Project-based Sessions**: Sessions are identified by project directory
   and `.apas` project metadata.
5. **Stream-JSON**: Uses Claude CLI's `--output-format stream-json` for
   structured Claude output; other providers are bridged through their own
   runtime paths.
6. **Real-time Updates**: WebSocket connections broadcast live messages,
   project-goal changes, Team TODO state, machine config, and pane diffs.
7. **Pane kinds**: `PaneConfig.kind` picks how a pane hosts its agent, and
   is orthogonal to `provider` (which binary) and `mode` (how autonomous).
   See "Terminal panes" below.

## Tab-type policy (`disallowed_tab_types`)

A project's owner or admin can restrict which tab types users may create. A
"tab type" is a **pane kind plus a provider** — `agent:claude`,
`terminal:codex` (`shared::tab_type_key`). Neither half alone identifies one: a
claude agent tab and a claude terminal tab are different capabilities, since the
terminal runs the real TUI with permission prompts bypassed.

Stored as a **deny** list, presented in the UI as an allow list. An allow list
on the wire would make an absent field mean "nothing permitted", so every
project predating the feature would refuse to open any tab. Empty deny list =
everything allowed = existing projects unaffected. It also means a provider
added later is permitted until an owner says otherwise, rather than vanishing
from their menu.

The catalog is deliberately **not** every `Provider`. MiniMax, GLM and DeepSeek
are the claude binary against a different backend, so the add-tab menu offers
them as claude *models* — a `agent:minimax` key would be a checkbox that
silently does nothing, because those tabs arrive as `provider: claude`.
Restricting by model would be a separate, larger feature. `shared::all_tab_types`
and `packages/web/src/lib/tabTypes.ts` must agree; a test in `shared` reads the
TS file and asserts they do.

Enforced in the CLI (`tab_type_allowed_for`), which re-reads `.apas` on every
`AddPane` — the web only hides menu entries, and the same message can arrive
from a stale browser tab whose menu predates the restriction. Unlike
`team_enabled_for`, this fails **open** on an unreadable `.apas`: the worst case
is a tab an owner meant to block, whereas failing closed would lock everyone out
of the project entirely.

**Managed team panes are exempt.** The Tech Lead spawns those from role
templates, and an owner restricting *user* tab types has not asked to break
their own team.

## Team mode is opt-in (`team_enabled`)

Managed team mode is **off for every project** until someone turns it on, and
only a project's **owner or admin** can turn it on or off. Users the project was
shared with can work in it but cannot change this.

Why off by default: enabling team mode spawns four autonomous panes that read
the repo, write worktrees, and open PRs. That should never be something a
project arrives with — it should be a decision someone made.

The flag lives in `.apas` as `team_enabled` and is `#[serde(default)]`, so a
`.apas` written before the field existed reads as **off**. Upgrading therefore
switches team mode off on existing projects too; their panes are untouched but
the team surfaces disappear until an owner opts back in. That is deliberate, not
a migration gap.

Three enforcement points, because each covers a different failure:

- **Server** (`ws_web::can_manage_project_settings`) — the actual permission
  check. Reuses `share::ProjectRole::can_manage_access` rather than re-deriving
  the owner/admin boundary, so the WS and HTTP paths cannot drift. Fails closed
  on an unknown user or a failed lookup.
- **CLI** (`team_enabled_for`) — refuses `ServerToCli::StartTeam` while the flag
  is false, re-reading `.apas` at the point of use. `.apas` is the source of
  truth, and a cached `true` would let a `StartTeam` that raced the toggle spawn
  the panes an owner just disabled. Also fails closed on an unreadable `.apas`.
- **Web** (`lib/projectRole.ts`) — decides what to render, nothing more. Hides
  the team surfaces and renders the settings read-only for a plain user.

Turning team mode **off stops a running team**: the CLI's `stop_managed_team`
pauses every managed deadloop and interrupts every managed pane's in-flight
turn, the same end state as the Overview's "Stop team" button. It pauses before
interrupting, where the web does the reverse — between an interrupt and the
pause landing, a sibling pane's write can wake the loop for one more iteration.
Unmanaged side chats are never touched.

## Self-reported history for terminal panes (`record_turn`)

An agent pane is *observed*: the CLI parses its stream-json and knows every turn
without cooperation. A terminal pane hosts the provider's real TUI on a pty, so
there is nothing structured to parse — which is why it had no history and no
usage.

The agent now reports its own turns through its per-pane MCP server. Each call
appends to `.apas-conversations/pane-<id>.jsonl` (one file per pane, so appends
need no cross-process lock — exactly one MCP server exists per pane, unlike
`team-todo.md`). A CLI tailer forwards new turns to the server.

**The turn is dressed as the stream message an agent pane would have sent**
(`conversation_turn_to_stream_messages`). That is the whole trick: no new wire
message, no new storage path, no new renderer. The server persists it to the
same `messages.jsonl`, the web renders it with the same components, and usage
accounting bills the same pane — none of which needed changing. A turn carrying
token counts emits a second `Result` message, because `ws_cli` reads usage only
from `extra.usage` on that variant; `total_cost_usd` stays 0 since a
self-reporting agent cannot know what it was billed.

**Two properties follow from self-report, and both are accepted trade-offs:**

1. *History is only as complete as the agent's cooperation.* Nothing enforces
   the call. The MCP server states the requirement in its `initialize`
   instructions — for a terminal pane that is the **only** channel available,
   since it execs the provider's TUI and there is no system prompt of ours to
   append to. If that text stops asking, history silently stops being written
   and nothing else fails, which is why a test asserts it still does.
2. *The content is whatever the agent says.* `pane_id` is stamped server-side
   from `--pane-id`, so a pane cannot forge history for **another** pane, but
   within its own pane this records what the agent claims, not what happened.

The alternative — tailing the provider's own transcript
(`~/.claude/projects/**.jsonl`, `~/.codex/sessions/**/rollout-*.jsonl`, both of
which carry full turns *and* token usage) — would be complete and need no
cooperation, but only ever works for providers whose format we track. Self-report
was chosen for provider-agnostic coverage.

## Team-mode MCP server (`apas mcp-server`)

Phase 3.1 shipped delegation as `.apas-team.jsonl` tag conventions driven from
bash (see `docs/dev/3.1-delegation-via-scratchpad.md`, which left the MCP path
open as a follow-up). That follow-up is now in: `crates/client-cli/src/mcp.rs`
exposes the same protocol as typed MCP tools.

**The scratchpad file is still the source of truth.** Every tool writes through
`scratchpad` / `team_todo` / `manager` and lands on disk in the shape it always
had — so the CLI's watcher still *observes* writes rather than trusting agents
to self-report, delegations stay visible in the web Team modal, and team state
survives a machine loss (after the 2026-08-02 NFS crash the scratchpad was the
only durable artifact of it — the server persists none of this).

Tools: `publish_record`, `delegate`, `read_records`, `read_team_todo`,
`propose_todo`, `update_todo_status`, `read_project_goal`,
`write_project_goal`, `list_panes`. Schemas are derived from Rust argument
structs via `schemars`, so the advertised schema cannot drift from the code.

**One server per pane.** The CLI spawns `apas mcp-server --project-dir <root>
--pane-id <n>` as a stdio child of each pane, so `pane_id` is stamped
server-side: an agent can neither publish as another pane nor forget to
identify itself. Note `--project-dir` is the **project root**, never a pane's
worktree — worktrees contain no `team-todo.md` / `.apas-team.jsonl`.

Provider wiring reuses existing channels: claude (and the MiniMax/GLM/DeepSeek
variants, which are the claude binary behind different env) via `--mcp-config`;
codex via `-c mcp_servers.apas.*`, the same `-c` override that already carries
`model_reasoning_effort`. opencode/cursor-agent get no flags — unverified, and a
bad flag breaks the spawn outright.

**Concurrency.** `team-todo.md` mutations are a load → mutate → save cycle, and
every pane runs its own server process (rmcp also serves calls concurrently
within one). Without a lock, two panes mutating at the same instant both read
the old file and the second save silently discards the first — a lost update
with no error either side, reproducible on the first concurrent call. All
mutating tools therefore hold an advisory `flock` on `.apas-team.lock` for the
whole cycle; it releases on fd close, so a killed pane cannot wedge the project.

## Terminal panes (`kind: "terminal"`)

Most panes are `kind: "agent"` (the default, and what every pane written
before this field existed deserializes as): the CLI runs the provider
headlessly and parses stream-json into structured events.

A **terminal pane** instead allocates a pty (`portable-pty`), execs the
provider's *real interactive TUI*, and streams the raw bytes
to xterm.js in the browser. Nothing is parsed, so nothing has to be kept in
sync with a provider's output format — the point is to reuse the CLI as it
ships.

Only `claude` and `codex` can host one (`terminal_pane::terminal_binary_for`);
the MiniMax/GLM/DeepSeek variants are the claude binary behind different env,
and opencode/cursor-agent have unverified pty behaviour.

**What terminal panes deliberately do not get.** Every other team-mode
integration is built on stream-json events, so a terminal pane still has no
pane status, no `PaneDiff`, and no plan review. It is never a Tech Lead
delegation target and is forced `managed: false`.

**Conversation history and usage now arrive by self-report** — see below.

**Transport.** Raw pty bytes never touch `CliToServer::Output` or
`StreamMessage` — those persist into `messages.jsonl` as chat records, and
ANSI would both bloat the store and break the message renderer. They ride
dedicated `Terminal*` messages, base64-encoded because a pty read splits
both UTF-8 sequences and escape sequences:

- `CliToServer::TerminalOutput` / `TerminalExited`
- `ServerToWeb::TerminalOutput` / `TerminalSnapshot` / `TerminalExited`
- `WebToServer::TerminalInput` / `TerminalResize` / `TerminalAttach`
- `ServerToCli::TerminalInput` / `TerminalResize`

The server keeps a bounded in-memory scrollback ring per `(session, pane)`
(`TERMINAL_SCROLLBACK_MAX_BYTES`, never written to disk) and answers
`TerminalAttach` from it, so reattach paints instantly and works while the
CLI is mid-reconnect. The ring is dropped when the pane is removed and when
the CLI disconnects — those ptys died with it, and replaying their last
frame would look like a live terminal.

On the web side, frames bypass zustand entirely (`lib/terminalBus.ts`): a
full-screen TUI repaints many times a second, and storing chunks in state
would re-render every subscriber per frame.

**Lifetime.** The pty is a child of the CLI process, so a terminal pane dies
when `apas` restarts; the restore path re-execs with the provider's own
continue flag (`claude --continue`, `codex resume`) as the closest available
substitute. There is no apas-visible session id to resume a TUI.
