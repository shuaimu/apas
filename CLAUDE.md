# APAS - Autonomous Programming Agent System

> Canonical contributor/agent runbook. Keep architecture, local development,
> and workflow guidance here. `agent.md` is a generated pointer to this file;
> `claude.md` and `AGENTS.md` are deployment-only notes.

APAS runs coding agents against a project, from a browser or a phone. The CLI
owns local panes and worktrees, the server brokers project/session state, and
the web UI exposes the pane tabs, conversation and terminal views, the project
overview, and diff/PR handoff surfaces.

A project is a directory with an `.apas` file; the work happens in **panes**,
each hosting one agent. A pane is created, talked to, rebooted and closed by a
person — there is no orchestration layer above it.

**Managed team mode was removed.** Four roles (Manager, Tech Lead, Developer,
Reviewer) used to coordinate through `project_goal.md`, `team-todo.md` and
`.apas-team.jsonl`, dispatching work to each other through an MCP server. It was
the largest feature in the CLI and, at removal, had zero managed panes anywhere
in the deployment. What it cost everything else was a second way to run a
provider: a second spawn path, its own status and review plumbing, and a policy
field carried at three levels. If you find a reference to a team role, the
scratchpad, the TODO queue or `apas mcp-server`, it is stale — say so rather
than reviving it.

Two things that look like team mode and are not:

- **`role` / `goal` / `backstory` on a pane** — identity metadata anyone can set
  from the role modal, composed into the pane's system prompt by
  `pane_identity::compose_system_prompt`.
- **The "Start bot" deadloop** — a pane repeating a prompt on an interval. It
  predates team mode and outlives it.

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

The CLI keeps project-local state in `.apas`, plus optional per-pane worktrees.
The server caches and broadcasts machine, session and pane state. The web UI
lets a person open panes, talk to them, inspect their diffs, and hand off to a
PR.

## Project Structure

```
apas/
├── crates/
│   ├── client-cli/      # CLI binary (apas)
│   │   ├── src/
│   │   │   ├── main.rs        # CLI entry point and config commands
│   │   │   ├── config.rs      # User/machine config and supported backend settings
│   │   │   ├── project.rs     # .apas project metadata
│   │   │   ├── pane_identity.rs # A pane's role/goal/backstory as a system prompt
│   │   │   ├── claude_session_hook.rs # SessionStart hook: which transcript a pane writes
│   │   │   ├── worktree.rs    # Isolated worktree creation/diff/cleanup
│   │   │   ├── claude.rs      # Claude process wrapper
│   │   │   ├── terminal_pane.rs # Pty host for kind:"terminal" panes (portable-pty)
│   │   │   └── mode/
│   │   │       ├── dual_pane.rs # Pane runtime: panes, deadloops, watchers
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
│   │   │       ├── ws_cli.rs  # CLI WebSocket handler, pane/session replay
│   │   │       └── ws_web.rs  # Web WebSocket handler, Overview/machine actions
│   │   └── Cargo.toml
│   │
│   └── shared/          # Shared types between CLI and server
│       ├── src/
│       │   ├── lib.rs
│       │   └── messages.rs  # Shared WebSocket/machine message types
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
│       │       └── mobileSelectedPane.ts # Which pane a session screen opens on
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
  "auto_approve_todos": false,
  "auto_merge_prs": false,
  "panes": [
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

`auto_approve_todos`, `auto_merge_prs` and `disallowed_tab_types` are
project-level policy flags, settable by the project owner or the operator of the
cluster hosting it. `disallowed_tab_types` restricts which tab types users may
create (see "Tab-type policy"); the other two were read by the team loop and are
now inert, kept so an older `.apas` still parses.

`managed: true` on a pane is likewise vestigial. It marked a pane as belonging
to the managed team rather than to you, and it used to gate real behaviour:
refusing PR creation, exempting the pane from the retired-agent-kind rule,
forcing Claude panes to `effort: max`, and gating relaunch on team availability.
Two boot-time migrations even *created* it, promoting any pane whose role
mentioned "manager", "tech lead" or "reviewer" — so naming a pane "reviewer" in
the role modal silently made it managed. None of that remains; a test asserts
loading a project never marks a pane managed.

New work is created as a terminal pane. `kind` defaults to `"agent"` when absent
solely for compatibility, so `.apas` files written before terminal panes existed
keep loading unchanged — see "Terminal panes" under Key Concepts. A `.apas` may
also still carry `team_enabled`, a `role` like `"team manager"`, or
`managed: true`; all three are ignored, and such a pane loads as an ordinary
pane.

## Message Types

Key message types in `crates/shared/src/messages.rs`:

- **CliToServer**: Register, SessionStart, StreamMessage, UserInput,
  Heartbeat, ProjectFlagsChanged, TerminalOutput, TerminalExited, machine
  config/status updates.
- **ServerToCli**: Registered, SessionAssigned, Input, Signal,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  TerminalInput, TerminalResize, pane/worktree/suggestion actions.
- **WebToServer**: Authenticate, ListCliClients, AttachSession, Input,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  TerminalInput, TerminalResize, TerminalAttach, machine/provider config
  actions.
- **ServerToWeb**: Authenticated, CliClients, SessionMessages, StreamMessage,
  UserInput, ProjectFlagsChanged, Machines, PaneDiff, TerminalOutput,
  TerminalSnapshot, TerminalExited.

The `Terminal*` family is the pty byte channel for `kind: "terminal"` panes and
is deliberately separate from `Output` / `StreamMessage` — see "Terminal panes"
under Key Concepts for why.

`WebToServer::UpdateProjectFlags` carries the project policy flags from the web
to the server. The server **rejects the whole message from anyone who is neither the
project owner nor the operator of the cluster hosting it**
(`ws_web::can_manage_project_settings`) — this is the only authority gate in the
WebSocket layer, everything else there authorizes on session *access* alone.
It then forwards `ServerToCli::UpdateProjectFlags` to the CLI for `.apas`
persistence; the CLI emits `CliToServer::ProjectFlagsChanged`, and the server
broadcasts `ServerToWeb::ProjectFlagsChanged`. The CLI also re-broadcasts the
flags from `.apas` every 5s, so a web client attaching mid-session hydrates
without asking.

## Data Storage

- **SQLite** (`data/apas.db`): Users, CLI clients, sessions metadata
- **JSONL files** (`data/sessions/{id}/messages.jsonl`): Chat messages per session
- **Worktrees** (`.apas-worktrees/pane-<id>/`): isolated branches for a pane
  working in isolation

A project that predates the team-mode removal may still hold `project_goal.md`,
`team-todo.md` and `.apas-team.jsonl` on disk. Nothing reads or writes them —
they are left alone rather than deleted, because they are the user's files.

## Development

### Running locally
```bash
# Terminal 1: Server
RUST_LOG=info cargo run -p apas-server

# Terminal 2: Web frontend
cd packages/web && npm run dev

# Terminal 3: CLI (in any project directory). Registers the project and exits;
# start it from the web. `--attach` opens the terminal UI for one already
# running here.
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
  `apas-server` (`127.0.0.1:8080`), along with `/cluster/ /projects/
  /mobile/`; everything else → the Next.js app
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

  **The mirror-image mistake is easier to make: a new API prefix nginx does not
  proxy at all.** Everything unmatched falls through to Next.js, so the route
  answers `200` with the app's HTML instead of JSON, and only the caller
  notices. `/cluster/` shipped this way and broke the entire cluster surface
  until it was added here. **Whenever you add a route prefix to
  `routes/create_router`, add the matching `location` in this file in the same
  change**, and smoke-test it with `curl -s <url> | head -c 40` — an HTML
  doctype where JSON belongs is this bug.
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

# Configure the system administrator BEFORE deploying the server. Without it,
# /admin cannot be entered at all, and this deploy removes every account's
# deployment-wide authority. Keep apas-server.toml mode 0600.
ssh root@apas.mpaxos.com "grep -q '^\[system_admin\]' /opt/apas/apas-server.toml || echo 'MISSING [system_admin] BLOCK'"

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
# Then sign in at https://apas.mpaxos.com/admin with the configured credential
# and rotate it: the surface warns while it is still the bootstrap value, and
# rotation invalidates every token issued against it.
ssh root@apas.mpaxos.com "systemctl is-active apas-web apas-server && journalctl -u apas-web --since '5 minutes ago' -p err --no-pager -q && journalctl -u apas-server --since '5 minutes ago' -p err --no-pager -q"
```

For rollback, move the failed `/opt/apas/web` directory aside, extract the
selected `/opt/apas/backups/web-<timestamp>/web.tgz` under `/opt/apas`, run
`npm ci` and the versioned build from that restored tree, then restart and
repeat every smoke check above. This emergency rollback restores the previous
vulnerable dependency graph; follow it with a corrected patched deployment.

## Key Concepts

1. **Pane Runtime**: The CLI restores panes from `.apas`, runs deadloop or
   interactive processes, and can isolate a pane in a git worktree.
2. **Hybrid Mode** (legacy): Single pane with local terminal + streaming.
3. **Project-based Sessions**: Sessions are identified by project directory
   and `.apas` project metadata.
4. **Stream-JSON**: Uses Claude CLI's `--output-format stream-json` for
   structured Claude output; other providers are bridged through their own
   runtime paths.
5. **Real-time Updates**: WebSocket connections broadcast live messages,
   machine config, and pane diffs.
6. **Pane kinds**: `PaneConfig.kind` picks how a pane hosts its agent, and
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
from a stale browser tab whose menu predates the restriction. It fails **open**
on an unreadable `.apas`: the worst case
is a tab an owner meant to block, whereas failing closed would lock everyone out
of the project entirely.

## Virtual clusters and system administration

These are two different jobs and two different surfaces. They used to be one
page behind one `cluster_role = "admin"` flag, which meant running *your own*
machines required deployment-wide authority over everyone else's projects.

**A virtual cluster is derived, not stored.** Every account operates exactly
one: the machines whose client registered under it, plus the projects hosted in
it. A project is hosted in an account's cluster when the account **owns it** or
when **at least one of its sessions was created under it** (`db::
project_in_user_cluster`). Sessions are the durable evidence of where a project
actually runs — machines live only in memory, and projects carry no machine
column. So a project owned by `soumojit.dalui` that runs on your daemon is in
*your* cluster, and you administer it without being a member of it.

**Belonging to a project is deliberately not hosting it.** Content access is
owner ∨ member ∨ host; administration (lifecycle, stop-runtime, membership,
ownership, policy) is host only. Otherwise anyone you shared a project with
could suspend it. Running it on your own machine *does* make you a host — that
is the point, not a loophole.

The cluster surface is `/machines` (kept at that route so links still work),
available to every active account with no role check: `routes/cluster.rs`
scopes every request through the hosting predicate. `routes/admin.rs` backs the
deployment-wide surface and both call the same DB operations, so the two cannot
drift.

**System administration is a credential, not an account.** One per deployment,
stored in `system_admin_credential` outside the `users` table, seeded from
`[system_admin]` in `apas-server.toml` only when no row exists — so editing the
config later cannot revert a rotation. Its token carries `sub =
"system-admin"`, `token_kind = "system_admin"`, and the credential version;
rotating the password bumps that version and invalidates every outstanding
token. An account token is rejected by `/admin/*` and this token is rejected
everywhere else. No UI can grant it, and `bootstrap_admin_email` is gone —
registration now always requires an invitation, which the system administrator
issues.

**The `/admin` login must stay inline on the page.** nginx proxies the whole
`/admin/` prefix to `apas-server`, so a Next.js route at `/admin/login` would
never be served. The page renders its own form when it holds no token, and
keeps that token in `sessionStorage` under its own key — never in the zustand
store, never in `localStorage`, so it dies with the tab. Nothing in the ordinary
interface links to it.

**`cluster_role` no longer means anything.** Every account migrates to `user`,
and the column plus its wire field survive only so older web and mobile builds
keep parsing identity responses. Two deployment-wide reads went with it: web
machine listings no longer have an all-machines branch, and no account can
revoke another account's mobile device.

**Policy resolves over three levels** — deployment default (system
administrator) → cluster default (`cluster_default_policies`, one row per
account) → project override. The launch-profile allowlist **narrows
monotonically**: each level may only restrict what the level above allows, and
a widening write is rejected rather than silently clamped. It intersects over
*every* hosting cluster, not just the owner's, so your cluster default governs
the foreign-owned projects on your machines; intersection is order-independent,
so a multi-hosted project still has one answer.

`team_available` is **vestigial**. It is still on the wire and in stored policy
so web and mobile builds that predate the team-mode removal keep parsing what
they are sent — the same reason `cluster_role` survives — and it decides
nothing. Do not reintroduce a read of it; a test asserts a policy carrying it
behaves exactly like one without it.

**Audit carries an actor kind and a cluster.** `admin_audit_events` was rebuilt
once (create-copy-drop-rename, guarded by `schema_migrations`) because
`actor_user_id` referenced `users(id)` with `PRAGMA foreign_keys=ON`, which made
a non-account actor unrecordable. `cluster_invitations.created_by` was rebuilt
for the same reason. `project_members.invited_by` keeps its key, so a
system-administrator membership change is attributed there to the project owner
while the audit row records who actually did it. An operator's audit view also
applies the live hosting predicate, so a project hosted in several clusters is
visible to each of them and pre-attribution rows still land where they belong.

## Projects run inside the one instance

A host runs one `apas` process for a user, and the projects run *in it* as
supervised tasks. The daemon used to spawn each project as `apas --headless`
into its own tmux session and then could not see it, which is why running state
was inferred from `/proc` and why a restarted daemon came back owning nothing.
A host is now one `apas` plus one **pane host per terminal pane**.

**Pane hosts are deliberately not merged.** They own the PTYs so a provider
survives the CLI being replaced — which is also what keeps this arrangement
safe, since the blast radius is the supervision layer rather than running
agents.

**Stopping a project sets a flag it observes; it is never an aborted future.**
Aborting would strand roughly thirty threads and every pane child. The flag
ends the project through the same teardown an ordinary stop takes, and the wait
is bounded at 30s so one project that will not stop cannot hold the instance
the others are running in.

**Failure containment rests on unwind.** `panic = "abort"` is not set, so a
panicking project unwinds its own task, is reported as stopped, and leaves the
others alone. A test fails if anyone ever sets it, because that would turn any
project panic into a host-wide outage with nothing else looking different. The
three `process::exit` calls that used to mean "stop this project" are gone;
`run_inner` returns a `ProjectOutcome` and the caller decides.

**Blocking work must stay off the runtime.** Each project's blocking readers
already run on their own threads. New code that blocks in async would stall
every other project, and a two-project test will not show it.

**Two things the process boundary gave for free had to be replaced.** Each
project carries a `project` span so its records are identifiable — the
per-project tmux session and stderr log are gone, and the default `Full` log
format prints the span fields. And `exec` now takes the projects with it, so an
upgrade writes what was running to a manifest in the *runtime* directory
(volatile, so a machine reboot starts nothing nobody asked for; cleared on read
so a crash cannot retry forever) and starts them again afterwards. Pane hosts
survive the `exec`, so a prompt resume lands inside their adoption grace.

**A PATH trap comes with the merge.** The old model passed
`env PATH=<login shell PATH>` on every spawned project's command line, because
a daemon started from a minimal environment cannot find nvm/cargo-installed
providers. In-process, projects inherit the daemon's PATH, so the daemon
applies the login shell PATH to itself before any project starts.

**The rollout is not additive.** An older instance's projects are separate
processes this one cannot supervise, so a starting instance stops the
process-per-project leftovers before starting anything — otherwise one `.apas`
and one set of worktrees get two owners. It finds those by their `-d <path>`;
it cannot find one a person started by running `apas` in a directory before
that became register-and-exit, because those carry no arguments and nothing
distinguishes them from an `apas --attach` in use. Those stay a manual step.

`apas --headless -d <path>` survives as a way to run one project alone for
debugging. Nothing spawns it, and a project with an external headless run is
never given a second owner.

## One instance per user per host

Running `apas` in a project directory **registers that project and exits**. It
does not open a terminal UI and does not start the project — projects are
started from the web. A host runs one APAS instance per user, and by the time
a launch gets there it already exists, because `ensure_daemon_running` started
it.

This is the existing `apas daemon` rule applied to the command people actually
type. `detect_running_daemon` already reads a pid state file, verifies the
process really is `apas`, and deletes a stale record; `apas daemon` already
prints and returns when one is running. The gap was that plain `apas` went
straight into `dual_pane` without asking, so typing it in a directory the
daemon was already running produced two owners of one project, one `.apas`,
and one set of worktrees — `is_headless_running_for` guards only the daemon's
own spawns.

Registration is all a launch needs to do: the daemon reads the shared registry
(`list_registered_projects`) on every heartbeat and reports what it finds, so
the project appears on the Machines page with no IPC and no start request.

**Creating a project from a local directory is still a thing a launch does.**
`apas` in a directory that is not yet a project creates and registers it. This
rule governs how many instances run, not when a project comes into being, and
the web's create flow only clones a repository into a *new* directory.

**Headless workers are exempt.** They are `apas` processes, but the daemon's
children rather than instances a user launched; applying the rule to them
would stop the daemon running more than one project.

`apas --attach` opens the terminal UI for a project already running here. It
is a local view for when the web is unreachable, it renders little beyond pane
names, and it is expected to be removed if nothing uses it.

## The daemon is replaced only when asked

Replacing the daemon used to be nearly free: it owned no long-lived children, so
a **15-minute self-upgrade tick** was a good trade — unattended hosts stayed
current and nothing running noticed. zoo-002 sitting nine versions behind was
the problem it solved.

Projects run **inside** the daemon now. Replacing it is the same act as stopping
every project on the host and starting them again, so nothing does it on a
schedule any more. **The tick is gone**, along with the `stat`-based gate it
needed (`apas_binary_fingerprint`, `newer_installed_version`). Installing a
binary to the shared path no longer propagates on its own.

**A launch does not replace it either.** `apas` in a project directory, and
`apas daemon`, both used to stop an older daemon and start a fresh one. That was
invisible when the daemon owned nothing; now it ends every project on the host —
and by `stop_daemon_process`'s SIGTERM/4s/SIGKILL, so nothing is saved, no
resume manifest is written, and nothing comes back. Whoever types `apas` in a
directory is a bystander to that work. Both paths now report that the running
instance is older and point at the Machines page; `stop_daemon_process` is gone
with its last caller. `plan_launch_daemon` holds the decision so it is testable
away from the spawning, the way `plan_daemon_restart` does.

**The requested restart is the whole upgrade path**, from the Machines list on
desktop and mobile (`WebToServer::RebootDaemon { machine_id }` →
`ServerToDaemon::RebootDaemon`). It is addressed by **machine**, not through a
project: a daemon is per-machine, and a host running nothing still has one worth
restarting. The server authorizes the machine against the requester's own daemon
registrations — the same check `StartMachineProjectCli` uses — and reports an
offline daemon rather than dropping the request.

A requested restart **applies an available update first**, via
`prepare_cli_restart`, so `check_for_update_available` syncs the git repo and
pull/build/install all complete while the current daemon is still serving; a
failure leaves that daemon working rather than the machine with none. It is what
lets a CLI update roll out from a phone instead of an SSH session. A version
that only touches the web frontend or docs skips the rebuild
(`pending_update_needs_rebuild`). Progress past "requested" is deliberately not
reported — the daemon replaces its own process image, so anything further would
have to outlive the process that would report it.

It upgrades only. Equal, older, or unparseable versions replace in place;
an accidental downgrade across a cluster sharing one NFS home would be painful
to unpick.

**The cost of removing the tick is that a host nobody visits keeps its version
forever** — exactly the zoo-002 problem, deliberately reaccepted. What makes it
tolerable is that the machine lists now *show* it: each machine displays the
version its daemon reports, and its restart control reads "Reboot to update"
when that version is behind the newest one the client can see. The old failure
was that nothing surfaced it.

**It re-execs rather than spawn-and-exit**, which matters for three reasons:

- the pid is preserved, so `daemon.json` stays correct and
  `detect_running_daemon` is not briefly fooled into starting a second daemon,
- the session is preserved, so it stays `setsid`-detached,
- destructors do **not** run, so `RegistrationGuard` never withdraws the host
  record or releases project claims. Spawn-and-exit would open a window where a
  peer daemon sees those projects unclaimed and could spawn a duplicate CLI
  against the same `.apas` and worktrees — the race the claim system exists to
  prevent.

**Stopping it stops its projects.** The signal handler is registered with
`ctrlc`'s `termination` feature, so a SIGTERM sets the same shutdown flag a
SIGINT does. Without that only SIGINT is handled, and a `setsid`-detached daemon
essentially never receives one — the teardown that saves each project's pane
roster and ends its agent subtrees would be unreachable, and nothing would look
any different. A test fails if the feature is ever dropped, for the same reason
one fails if `panic = "abort"` is ever set.

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

**Pane hosts are untouched.** They own the PTYs in their own tmux sessions, so
nothing about replacing the daemon's process image reaches them, and a
replacement that lands inside their adoption grace picks the terminals back up.

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

- **claude** — **the provider reports its own transcript.** Each Claude pane is
  spawned with `--settings <pane file>` installing a `SessionStart` hook that
  runs `apas session-hook`; Claude passes it `transcript_path` on stdin, and it
  records that under
  `${XDG_RUNTIME_DIR}/apas/panes/<project>/<pane>/claude-session.json` (0600,
  written temp+rename so a poll cannot read a half-written file). The watcher
  reads that file.

  The deriving path — `--session-id <pane's session_id>` pinned at spawn, and
  `~/.claude/projects/<cwd with / replaced by ->/<session-id>.jsonl` — remains
  as the fallback, and both halves of it are wrong as soon as the user acts.
  Claude Code can move a session into `.claude/worktrees/<name>` and then writes
  under **that** directory's slug; and `/resume` onto another session appends to
  **that session's** file, so the pinned id names a conversation the pane has
  left. What used to cover the gap — follow the newest transcript in the
  directory — cannot tell our pane switching files from an unrelated `claude`
  running in the same directory, and on a live pane it published 607 records of
  someone else's conversation while the pane's own sat unread in a worktree.

  Three things were verified against Claude Code before building on it: the hook
  fires on `startup` **and** `resume` carrying the absolute `transcript_path`;
  `--settings` **merges** with the user's other settings layers rather than
  replacing them, so the pane keeps its owner's model and theme; and the hook
  process **inherits the provider's environment**, which is how `APAS_PANE_RUNTIME`
  identifies the pane. A `claude` a person runs by hand has no such variable, so
  it records nothing and can never be adopted.

  This is deliberately unlike the `record_turn` MCP tool below, which asked the
  *model* to cooperate and was abandoned: a hook is run by the client, not
  chosen by the agent. Never substitute `--continue`: it can select another
  pane's most recent cwd conversation and silently disconnect terminal output
  from APAS's transcript watcher.
- **codex** — cannot be given an APAS-chosen id at creation. On Linux, APAS uses the terminal's
  process group to find the user rollout that its codex process actually has
  open, so multiple panes can share one cwd without sharing status or history.
  The selected path is retained across brief descriptor gaps and changes when
  that process opens a newer user rollout through resume/fork. Its
  `session_meta.id` is then the first verified Codex identity APAS has for the
  pane, so it replaces the provisional pane UUID in `.apas` and future restores
  use `codex resume <id>` directly. A pane written by an older APAS keeps the
  picker until the process-owned rollout reveals its id; `--last` is never used
  because it can select a sibling pane in the same cwd. Other platforms fall
  back to the newest user rollout whose `session_meta.cwd` matches, but that
  ambiguous fallback is not persisted as pane identity.
- **opencode** — OpenCode owns its `ses_*` identifiers, so APAS asks
  `opencode session list --format json` for the newest session whose directory
  exactly matches the pane cwd, then reads it with `opencode export <id>`.
  Like Codex, two panes sharing a cwd are inherently ambiguous; sessions from
  another directory are never selected.

Parsing keeps only real conversation. claude transcripts also carry `mode`,
`ai-title`, `last-prompt` bookkeeping; codex carries `developer` messages (the
harness's own injected context), `reasoning`, and tool calls; OpenCode exports
typed reasoning/tool/synthetic parts. None of those are turns, and rendering
them would be noise. Tool-use-only turns with no text are skipped rather than
recorded blank. In-progress OpenCode assistant messages are held until their
completion timestamp arrives so APAS never advances its cursor over a partial
reply.

**Agent questions appear in the conversation view, and can be answered there.**
The parser drops tool-use-only turns as noise — right for `Bash`, wrong for
`AskUserQuestion`, which has no text either but is the one turn the human has
to act on. Across the transcripts on one machine, 170 questions were recorded
and 169 were being discarded, so a terminal pane could sit blocked on a
question nobody could see. A question turn is now published as the `tool_use`
block it already is, so the web's existing `AskUserQuestionCard` renders it
with no new wire message, storage path, or renderer.

Answering reuses the whole `AnswerQuestion` pipeline agent panes use (web →
server → CLI); only the last hop differs, because a terminal pane has no
stream-json control channel. The CLI writes **keystrokes** to the pty instead:
`↑/↓` to navigate and `Enter` to select, which is the contract the picker
prints in its own footer — verified by driving the real TUI (Claude Code
2.1.233), where digits notably do *not* move the selection. The agent's options
occupy positions 1..N with the TUI's own `Type something` / `Chat about this`
beneath them, so stepping down by an option's index can only land on an option
the agent offered.

**The answer is confirmed by reading, never by writing.** A successful pty
write proves only that bytes were accepted, so the acknowledgement is the
`tool_result` the provider records, republished as `User` + `ToolResult` — the
one variant the server's converter reads for tool results, which is exactly why
ordinary non-assistant turns avoid it. A terminal pane has no structured echo,
so the recorded answer arrives as prose (`The user answered: "Q"="A"`) and is
parsed, letting the card settle on what the agent *took* rather than on what
was clicked. Pending state is derived the same way — a question is open while
its `tool_use` has no `tool_result` — which is what makes a blind write safe:
a stale tab, a retransmit, or a question already answered in the terminal all
send nothing.

**A pane blocked on that question is Pending answer, not Working or Idle.**
The server caches one canonical pane status when `AskUserQuestion` arrives and
reports it separately in pane/session summaries, so project lists, waiting-agent
lists, pane selectors and conversation status bars all agree. Pending answer
does not advance idle recency and takes presentation precedence over a cached
provider usage limit: it names the action the human can take now. Merely routing
answer bytes does not clear it; the matching transcript `tool_result` is the
proof that moves the pane back to Working until the resumed turn completes.

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
shows user/assistant turns plus the questions the agent asks, and sends typed
messages and answers into the same live pty. The terminal stays
mounted-but-hidden behind the conversation view, because unmounting would tear
down the xterm instance and force a re-attach, losing scroll position and focus
on every glance at the transcript.

Each turn is then dressed as the stream message an agent pane would have sent
(`conversation_turn_to_stream_messages`). That is the trick that made this
cheap: no new wire message, no new storage path, no new renderer, and no server
or web change at all. A turn carrying token counts emits a second `Result`
message, because `ws_cli` reads usage only from `extra.usage` on that variant;
`total_cost_usd` stays 0 because the transcript reports tokens, not price.

## Terminal panes (`kind: "terminal"`)

New user-created Claude, Codex, and OpenCode work uses `kind: "terminal"`. The structured
`kind: "agent"` path is retained for managed team roles and historical panes:
the CLI runs the provider headlessly and parses stream-json into structured
events. Missing `kind` still deserializes as `agent` so old `.apas` files remain
readable; that compatibility default is not a creation default.

A **terminal pane** instead allocates a pty (`portable-pty`), execs the
provider's *real interactive TUI*, and streams the raw bytes
to xterm.js in the browser. Nothing is parsed, so nothing has to be kept in
sync with a provider's output format — the point is to reuse the CLI as it
ships.

Only `claude`, `codex`, and `opencode` can host one
(`terminal_pane::terminal_binary_for`); DeepSeek and Cursor Agent have
unverified pty behaviour. OpenCode launches with `--auto`; a fresh mobile task
uses `--prompt <instruction>`, while restoration uses `--continue`.

APAS does not install OpenCode, choose its model provider, or authenticate it.
Install and authenticate OpenCode on every intended project host before enabling
the profile. The default executable is `opencode`; override a nonstandard
installation with `apas config set opencode_path /path/to/opencode`. Existing
explicit cluster/project allowlists remain opt-in and must add
`terminal:opencode:official:default` through the normal policy controls.

The desktop tab bar, mobile browser pane picker, native mobile task launcher,
server authorization, and CLI local add-tab path all enforce this boundary.
Existing agent panes can still run, receive messages, switch models and reboot;
creating another one is rejected. Nothing creates one any more — managed team
roles were the last thing that did, and they are gone with team mode.

**What terminal panes do not get directly.** Fine-grained tool status and plan
review are built on stream-json events, so a terminal pane has neither. Coarse
working/idle state is reconstructed from user turns and provider-confirmed
completion markers in the transcript. `PaneDiff` is *not* in the unavailable
list despite the pane-kind boundary suggesting it should be: it is computed
from git by `compute_pane_diff`, so it works for any pane with a worktree.

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
provider-native CLI prompt; a v1 CLI would otherwise accept the pane and drop
that instruction. OpenCode additionally requires `terminal_opencode_v1`, so a
rolling server/web deployment refuses to route it to an older v2 CLI that can
launch Claude/Codex but not OpenCode. The server asks the user to
update/reconnect instead of pretending an older launch succeeded.

Roll out OpenCode support in this order: server first, web second, then upgrade
and reconnect project CLIs so they advertise `terminal_opencode_v1`. Install and
authenticate OpenCode and opt the intended policies into its profile only after
the host CLI has reconnected.

### Persistent pane hosts and CLI lifecycle

On supported Unix project hosts, each new Claude, Codex, or OpenCode terminal
pane is owned by a hidden `apas pane-host` process in its own project-scoped
tmux session. The replaceable project CLI is only its authenticated controller.
This removes the CLI process from the provider's lifetime: a transport-only
`Reconnect Server` leaves the CLI, pane hosts, PTYs, queues, and structured
turns untouched, while `Reboot CLI` prepares the update first and then adopts
the same hosted terminal processes after `exec`.

The feature is advertised only when Unix sockets, tmux, the installed
`apas pane-host` subcommand, and secure runtime storage all validate. Otherwise
terminal panes keep using the direct PTY implementation and the lifecycle menu
warns that they must restart/resume. Existing direct PTYs are not migrated in
place; after their first restart under a capable CLI they become host-backed.
Structured `kind: "agent"` panes retain their existing restart/resume behavior
and are never described as live-adopted.

Pane-host state is host-local, volatile, and outside the project directory:

- Root: `${XDG_RUNTIME_DIR}/apas/ph`, or `/tmp/apas-<uid>/apas/ph` when
  `XDG_RUNTIME_DIR` is unavailable.
- Project/runtime directories are `0700`; `runtime.json`, `credential`, the
  Unix socket, and reboot `handoff.json` are `0600`.
- `runtime.json` contains identity, protocol, tmux session, and socket paths,
  but no credential or terminal content. The random 256-bit credential is in a
  separate owner-only file and is never sent to providers or the server.
- Raw detached output remains only in the pane-host's bounded in-memory ring;
  it is not written to `.apas`, SQLite, JSONL, or a spool file.

Unexpected controller loss keeps the provider alive for 600 seconds by
default; an authenticated reboot handoff gets 900 seconds. Configure the
bounded values with:

```bash
apas config set pane_host_adoption_grace_seconds 600  # allowed: 30..3600
apas config set pane_host_reboot_grace_seconds 900    # allowed: 60..7200
```

Pane close, provider switch/reboot, project stop/suspension, and project
deletion bypass grace: they tombstone the project, authenticate shutdown where
possible, terminate the provider process group, kill the exact pane-host tmux
session, and remove local runtime files. An unexpected orphan self-terminates
when its lease expires.

Operational inspection must not print `credential` contents. Safe checks are:

```bash
apas config path
find "${XDG_RUNTIME_DIR:-/tmp/apas-$(id -u)}/apas/ph" -name runtime.json -type f -print
tmux -L "apas-<full-project-uuid>" list-sessions
journalctl --user --since '15 minutes ago' | grep -E 'pane-host|lifecycle'
```

Use the web Machines/Admin project stop action for cleanup; the daemon can
enumerate and terminate pane hosts even when the project CLI is absent. If
manual recovery is unavoidable, first identify the exact session from its
owner-only `runtime.json`, then use `tmux -L <socket> kill-session -t <session>`;
never recursively delete a runtime root while a listed host is still alive.

Roll out in this order: server/shared protocol, web lifecycle menu, then CLI
and daemon. Old CLIs keep the legacy reboot control and never receive a
reconnect disguised as reboot. Rollback disables new host creation; already
running compatible hosts should be allowed to close normally or stopped by a
new daemon. Forcing an old CLI/daemon to clean them up interrupts active
terminal turns.
