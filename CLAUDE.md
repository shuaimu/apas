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
│   │   │   ├── config.rs      # User/machine config and supported backend settings
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
server = "wss://apas.mpaxos.com"
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

A newly created project has **no panes**. The user opens what they want; a
default Claude pane meant every fresh project immediately spawned an agent
process nobody asked for. Three places used to force one into existence and all
three are gone: the `ProjectMetadata` constructors, an "ensure there is always
at least one pane" backfill at CLI launch, and `migrate_legacy`. That last one
was the real blocker — it runs on *every* load via `get_or_create_project` and
refilled any empty pane list with **two** legacy panes, so "no panes" was not
representable at all. It is now gated on the legacy `deadloop_claude_session_id`
/ `interactive_claude_session_id` fields actually being present, which is what
distinguishes a pre-`panes` file from a new project.

`team_enabled`, `auto_approve_todos`, `auto_merge_prs`, and
`disallowed_tab_types` are project-level policy flags. `team_enabled` gates
managed team mode entirely (see "Team mode is opt-in" below);
`disallowed_tab_types` restricts which tab types users may create (see "Tab-type
policy"); the other two are read by the Tech Lead loop. All are owner/admin-only. Managed pane entries are restored as team roles; new unmanaged
work is created as a terminal pane. `kind` defaults to `"agent"` when absent
solely for compatibility, so `.apas` files written before terminal panes
existed keep loading unchanged — see "Terminal panes" under Key Concepts.

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
- `NEXT_PUBLIC_WS_URL`: WebSocket URL for web frontend (default: `wss://apas.mpaxos.com`)

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
  standard port 80 (`wss://apas.mpaxos.com/ws/web`) instead of the non-standard
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
  the **CLI/daemon** (`wss://apas.mpaxos.com/ws/cli`); do not firewall 8080.
- **Next.js web (apas-web)**: `127.0.0.1:3000` (moved off 80 via a systemd
  drop-in `apas-web.service.d/port.conf` → `Environment=PORT=3000`).
- The web's default WS/API URLs are now **port-less** (`wss://apas.mpaxos.com`,
  `https://apas.mpaxos.com`); `NEXT_PUBLIC_WS_URL`/`NEXT_PUBLIC_API_URL` no
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
# Verify both supported web dependency graphs in separate clean trees before
# deployment. Both audits include development dependencies and fail on low or
# higher severity findings.
(cd packages/web && npm ci && npm run audit:npm)
(cd packages/web && pnpm install --frozen-lockfile && pnpm run audit:pnpm)

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

# Back up the currently deployed web source and build before synchronization.
web_backup_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
ssh root@apas.mpaxos.com "install -d -m 700 /opt/apas/backups/web-${web_backup_stamp} && tar -C /opt/apas --exclude='web/node_modules' -czf /opt/apas/backups/web-${web_backup_stamp}/web.tgz web"

# For web updates. `--delete` is important: without it, files removed
# locally (renamed/retired components) stay on the server and the next
# `npm run build` fails because those files still reference removed
# store actions or types. Always inspect the dry-run before synchronizing.
rsync -avn --delete --exclude 'node_modules' --exclude '.next' --exclude '.apas-version' packages/web/ root@apas.mpaxos.com:/opt/apas/web/
rsync -av --delete --exclude 'node_modules' --exclude '.next' --exclude '.apas-version' packages/web/ root@apas.mpaxos.com:/opt/apas/web/

# Restart apas-web (compute version locally since server lacks git history).
# `npm run build` runs `prebuild` first (`eslint .`, packages/web/eslint.config.mjs),
# which fails the build on React Rules-of-Hooks violations (e.g. a hook after an
# early return -> React error #310 -> whole-app crash). So a lint failure here
# aborts the deploy before restart — fix the reported hook error, don't bypass it.
# `npm ci` installs the exact reviewed package-lock graph, including devDependencies.
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm ci && NEXT_PUBLIC_WEB_UI_VERSION=${web_version} npm run build && systemctl restart apas-web"

# Verify service state, public pages, API health, referenced /_next/static assets,
# WebSocket/terminal attachment from the UI, and recent service errors.
for path in / /login /machines /share /admin /health; do curl -fsSL "https://apas.mpaxos.com${path}" >/dev/null; done
ssh root@apas.mpaxos.com "systemctl is-active apas-web apas-server && journalctl -u apas-web --since '5 minutes ago' -p err --no-pager -q && journalctl -u apas-server --since '5 minutes ago' -p err --no-pager -q"
```

For rollback, move the failed `/opt/apas/web` directory aside, extract the
selected `/opt/apas/backups/web-<timestamp>/web.tgz` under `/opt/apas`, run
`npm ci` and the versioned build from that restored tree, then restart and
repeat every smoke check above. This emergency rollback restores the previous
vulnerable dependency graph; follow it with a corrected patched deployment.

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

The catalog is deliberately **not** every `Provider`. DeepSeek is the Claude
binary against a different backend, so the add-tab menu offers it as a Claude
model. `shared::all_tab_types` and `packages/web/src/lib/tabTypes.ts` must
agree; a test in `shared` reads the TS file and asserts they do.

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

## The daemon upgrades itself

The daemon checks **every 15 minutes** whether the installed `apas` binary is
newer than the one it is running, and re-execs into it if so. Its own interval,
not the 10s heartbeat: an upgrade is never urgent, and tying it to the heartbeat
exercised the version path 360x more often than anything benefited from.

Before this, `ensure_daemon_running` was the only upgrade path, and it runs on
*interactive CLI startup* — so a node nobody logs into keeps its daemon
forever. zoo-002 sat nine versions behind for exactly that reason.

**Cost is one `stat` per tick.** The binary's (length, mtime) is the gate;
`apas --version` is spawned only when that changes, since an install writes a
new file rather than editing one in place.

**It re-execs rather than spawn-and-exit**, which matters for three reasons:

- the pid is preserved, so `daemon.json` stays correct and
  `detect_running_daemon` is not briefly fooled into starting a second daemon,
- the session is preserved, so it stays `setsid`-detached,
- destructors do **not** run, so `RegistrationGuard` never withdraws the host
  record or releases project claims. Spawn-and-exit would open a window where a
  peer daemon sees those projects unclaimed and could spawn a duplicate CLI
  against the same `.apas` and worktrees — the race the claim system exists to
  prevent.

**Claims are reconciled at daemon startup.** `reconcile_running_claims` claims
every project this host is already running, because claims are otherwise only
taken in `start_project` — so a restarted or self-upgraded daemon would come
back owning nothing while its project CLIs kept running. During that gap a peer
sees the projects unclaimed, and its `is_headless_running_for` only reads its
*own* `/proc`, so a `StartProjectCli` there would spawn a second CLI against the
same `.apas` and worktrees. A peer holding the claim for something running here
is logged, not seized: that means two CLIs are already live for one project,
which is exactly what the operator needs to see.

Relatedly, `claim_project` adopts the **current pid** when refreshing a claim
owned by this hostname. A claim written by a previous daemon carries that
daemon's pid, and `refresh_own_claims` only refreshes claims whose pid matches
the running process — so keeping the old pid left the claim un-refreshed, stale
within `STALE_AFTER_SECS`, and free for a peer to take while we were actively
running the project.

**Headless project CLIs are untouched.** They are owned by a per-project
`tmux:server`, not by the daemon — the daemon has no long-lived children — so
nothing about replacing the daemon's process image reaches them. And because
`exec` preserves the pid, even a direct child would survive.

Their *reported* state survives too: `snapshot_projects` derives `is_running`
from `headless_pid_for()`, which scans `/proc` for a headless CLI with that
`-d <path>`, rather than from the daemon's in-memory `sessions` map. So the map
being reset by the re-exec costs nothing — the web still shows them running. The
map self-heals on the next `StartProjectCli`, which re-checks `/proc` before
spawning and therefore cannot double-start a project that is already up.

It upgrades only. Equal, older, or unparseable versions do nothing — equal would
re-exec every tick forever, and an accidental downgrade across a cluster sharing
one NFS home would be painful to unpick.

## Terminal-pane history: read the provider's transcript

An agent pane is *observed* — the CLI parses its stream-json and knows every
turn without cooperation. A terminal pane hosts the provider's real TUI on a
pty, so there is nothing structured to parse, which is why terminal panes had
no history and no usage counters.

**Self-reporting via MCP was tried first and does not work.** A `record_turn`
tool was added and the requirement stated in the MCP server's `initialize`
instructions. Tested against both providers: each connects to the server and
*will* call the tool when told to directly, but neither acts on the
`initialize` instructions — an ordinary task ("what is 17 times 23?") recorded
nothing at all. Those instructions are advisory and both clients treat them as
such. Do not reach for that mechanism again expecting a guarantee.

So the CLI reads the transcript each provider already writes. It needs no
cooperation, cannot be skipped, and carries token usage the agent would
otherwise have had to volunteer.

**Locating the file differs by provider, and so does the confidence:**

- **claude** — initially spawned with `--session-id <pane's session_id>`, which
  APAS already mints per pane, and restored with `--resume <pane's session_id>`
  so it keeps writing that same transcript. The path is exact:
  `~/.claude/projects/<cwd with / replaced by ->/<session-id>.jsonl`. Verified
  against a live run, not assumed. Never substitute `--continue` here: it can
  select another pane's most recent cwd conversation and silently disconnect
  terminal output from APAS's transcript watcher.
- **codex** — has no equivalent flag. Its rollout files record `cwd` and a
  start timestamp in `session_meta`, so a pane is matched to the newest rollout
  in its own directory. That is a heuristic; two codex panes in the same
  directory could in principle be confused.

Parsing keeps only real conversation. claude transcripts also carry `mode`,
`ai-title`, `last-prompt` bookkeeping; codex carries `developer` messages (the
harness's own injected context), `reasoning`, and tool calls. None of those are
turns, and rendering them would be noise. Tool-use-only turns with no text are
skipped rather than recorded blank.

**The conversation view is writable, and that is the point on mobile.** An
xterm TUI on a phone is close to unusable — no modifier keys, tiny hit targets,
scrolling that fights the page — so the conversation view plus its text box is
the practical way to drive an agent from one. Text goes straight into the pty
via the same `TerminalInput` path the xterm view uses; the TUI cannot tell the
difference. **MCP is not and cannot be involved**: it is agent-pull, so a tool
server can answer a call but can never push a turn into a live conversation.
Multi-line text is sent as a bracketed paste (`ESC[200~ … ESC[201~`) so the TUI
takes it atomically instead of treating the first newline as submit; single-line
text is sent bare, since a TUI that never enabled DECSET 2004 would otherwise
show the wrapper as literal keystrokes. The carriage return is a separate write.
The caveat is that it is sent **blind** — unlike the terminal view you cannot
see whether the agent is mid-turn or sitting in a menu, so the UI says where the
live state is.

**The web can render a terminal pane either way.** A per-pane toggle switches
between the live pty (xterm.js) and the same structured conversation view an
agent pane gets — the captured turns arrive as ordinary pane messages, so
`MessagePane` renders them with no special casing. The two are not equivalent
and the UI says so: the terminal is live and interactive, while the
conversation is a *reading* of the transcript that lags by up to one poll,
shows only user/assistant turns, and cannot be typed into. The terminal stays
mounted-but-hidden behind the conversation view, because unmounting would tear
down the xterm instance and force a re-attach, losing scroll position and focus
on every glance at the transcript.

Each turn is then dressed as the stream message an agent pane would have sent
(`conversation_turn_to_stream_messages`). That is the trick that made this
cheap: no new wire message, no new storage path, no new renderer, and no server
or web change at all. A turn carrying token counts emits a second `Result`
message, because `ws_cli` reads usage only from `extra.usage` on that variant;
`total_cost_usd` stays 0 because the transcript reports tokens, not price.

## Team-mode MCP server (`apas mcp-server`)

Phase 3.1 shipped delegation as `.apas-team.jsonl` tag conventions driven from
bash (see `docs/dev/3.1-delegation-via-scratchpad.md`, which left the MCP path
open as a follow-up). That follow-up is now in: `crates/client-cli/src/mcp.rs`
exposes the same protocol as typed MCP tools.

**The scratchpad file is still the source of truth.** Every tool writes through
`scratchpad` / `team_todo` / `manager` and lands on disk in the shape it always
had — so the CLI's watcher still *observes* writes rather than trusting agents
to self-report, delegations stay visible in the Overview scratchpad, and team state
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

Provider wiring reuses existing channels: Claude and DeepSeek use
`--mcp-config`; Codex uses `-c mcp_servers.apas.*`, the same `-c` override that
already carries `model_reasoning_effort`. OpenCode/Cursor Agent get no flags —
unverified, and a bad flag breaks the spawn outright.

**Concurrency.** `team-todo.md` mutations are a load → mutate → save cycle, and
every pane runs its own server process (rmcp also serves calls concurrently
within one). Without a lock, two panes mutating at the same instant both read
the old file and the second save silently discards the first — a lost update
with no error either side, reproducible on the first concurrent call. All
mutating tools therefore hold an advisory `flock` on `.apas-team.lock` for the
whole cycle; it releases on fd close, so a killed pane cannot wedge the project.

## Terminal panes (`kind: "terminal"`)

New user-created Claude and Codex work uses `kind: "terminal"`. The structured
`kind: "agent"` path is retained for managed team roles and historical panes:
the CLI runs the provider headlessly and parses stream-json into structured
events. Missing `kind` still deserializes as `agent` so old `.apas` files remain
readable; that compatibility default is not a creation default.

A **terminal pane** instead allocates a pty (`portable-pty`), execs the
provider's *real interactive TUI*, and streams the raw bytes
to xterm.js in the browser. Nothing is parsed, so nothing has to be kept in
sync with a provider's output format — the point is to reuse the CLI as it
ships.

Only `claude` and `codex` can host one (`terminal_pane::terminal_binary_for`);
DeepSeek, OpenCode, and Cursor Agent have unverified pty behaviour.

The desktop tab bar, mobile browser pane picker, native mobile task launcher,
server authorization, and CLI local add-tab path all enforce this boundary.
Existing unmanaged agent panes can still run, receive messages, switch models,
and reboot; creating another one is rejected. Managed team panes continue to
use `agent` because delegation, diffs, plan review, and status depend on its
structured stream.

**What terminal panes deliberately do not get.** Every other team-mode
integration is built on stream-json events, so a terminal pane still has no
pane status, no `PaneDiff`, and no plan review. It is never a Tech Lead
delegation target and is forced `managed: false`.

**Conversation history and usage** are recovered by reading the provider's
own transcript — see below.

**Transport.** Raw pty bytes never touch `CliToServer::Output` or
`StreamMessage` — those persist into `messages.jsonl` as chat records, and
ANSI would both bloat the store and break the message renderer. They ride
dedicated `Terminal*` messages, base64-encoded because a pty read splits
both UTF-8 sequences and escape sequences:

- `CliToServer::TerminalOutput` / `TerminalExited` / `TerminalState`
- `ServerToWeb::TerminalOutput` / `TerminalSnapshot` / `TerminalExited` /
  `TerminalState`
- `WebToServer::TerminalInput` / `TerminalResize` / `TerminalAttach`
- `ServerToCli::TerminalInput` / `TerminalResize`

The server keeps a bounded in-memory state entry per `(session, pane)` with
scrollback bytes, the newest sequence, truncation, PTY instance UUID,
lifecycle (`unknown`, `running`, `disconnected`, or `exited`), and optional
exit status. `TerminalAttach` is answered from that entry, including when it
has lifecycle but no bytes, so reattach paints immediately and a process that
exited before producing output still gets an accurate banner.

**PTY lifetime is not WebSocket lifetime.** A transport-only CLI disconnect
changes confirmed-running terminal entries to `disconnected` but retains their
bounded presentation. When the same APAS process reconnects, it reports every
configured terminal before draining queued output. Each spawned PTY has a UUID:
a running report for the same UUID restores `running` without clearing bytes,
while a different UUID replaces the entry and starts with fresh presentation.
Output and exit events from an older UUID are ignored. An already-`exited`
entry and its status stay exited across later transport cleanup. Explicit pane
removal still deletes the entry.

Retention is deliberately non-durable. Terminal bytes and lifecycle remain in
the session manager only, obey `TERMINAL_SCROLLBACK_MAX_BYTES`, and are never
written to SQLite or `messages.jsonl`; a server restart loses them and state is
`unknown` until the CLI reconciles again.

On the web side, frames bypass zustand entirely (`lib/terminalBus.ts`): a
full-screen TUI repaints many times a second, and storing chunks in state
would re-render every subscriber per frame. `TerminalPane` tracks the current
PTY UUID and last rendered sequence locally. Same-instance snapshots at an
already-rendered sequence update lifecycle without duplicating output;
cumulative snapshots that cover missed frames and replacement UUIDs reset
xterm before replay. Snapshot/live lifecycle is authoritative for the
disconnected, unknown, and exited banners, including empty snapshots.

**Rolling deployment.** Deploy the server first, then web, then CLI. New fields
are optional/defaulted, so a new server accepts legacy output and exit frames,
and a new web treats a legacy snapshot as `unknown`. Metadata-less output is
still rendered but never proves that a retained process is running across a
reconnect. During rollback, an older server may ignore the new `TerminalState`
variant while continuing to relay the backward-compatible output/exit frames;
continuity then degrades to unknown/disconnected rather than a false confirmed
running state.

Native mobile launch advertises `mobile_task_launch_v2`. The version bump is
intentional: v2 creates a terminal pane and passes the first instruction as a
positional CLI prompt; a v1 CLI would otherwise accept the pane and drop that
instruction. The server therefore asks the user to update/reconnect the CLI
instead of pretending an older launch succeeded.

**Lifetime.** The pty is a child of the CLI process, so a terminal pane dies
when `apas` restarts; the restore path re-execs with the provider's own resume
flow (`claude --resume <pane session id>`, `codex resume`). Claude's pane
session id is APAS-visible and remains pinned across that restore.
