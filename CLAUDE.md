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
│       │   │   ├── chat/             # Message display
│       │   │   ├── code/             # Code blocks
│       │   │   └── tools/            # Tool cards
│       │   └── lib/
│       │       ├── store.ts          # Zustand state, WebSocket message handling
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
Each project directory gets a `.apas` file with project metadata:
```json
{
  "id": "uuid",
  "name": "project-name",
  "created_at": "2024-01-01T00:00:00Z"
}
```

## Message Types

Key message types in `crates/shared/src/messages.rs`:

- **CliToServer**: Register, SessionStart, StreamMessage, UserInput,
  Heartbeat, ProjectGoalChanged, ProjectFlagsChanged, TeamTodoChanged,
  SuggestedWorkersChanged, machine config/status updates.
- **ServerToCli**: Registered, SessionAssigned, Input, Signal,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  pane/worktree/suggestion actions.
- **WebToServer**: Authenticate, ListCliClients, AttachSession, Input,
  UpdateProjectGoal, UpdateProjectFlags, TodoApproval, AddTodo,
  machine/provider config actions.
- **ServerToWeb**: Authenticated, CliClients, SessionMessages, StreamMessage,
  UserInput, ProjectGoalChanged, ProjectFlagsChanged, TeamTodoChanged,
  SuggestedWorkersChanged, Machines, PaneDiff.

`WebToServer::UpdateProjectFlags` carries the Tech Lead autonomy flags
(`auto_approve_todos` and `auto_merge_prs`) from the web to the server. The
server forwards `ServerToCli::UpdateProjectFlags` to the CLI for `.apas`
persistence; the CLI then emits `CliToServer::ProjectFlagsChanged`, and the
server broadcasts `ServerToWeb::ProjectFlagsChanged`. Behavioral safeguards for
those flags are documented in the role prompts and README.

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
- **Server port**: 8080 (WebSocket: `ws://apas.mpaxos.com:8080`)
- **Web UI port**: 80 (http://apas.mpaxos.com)

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

# View logs
journalctl -u apas-server -f
journalctl -u apas-web -f
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

# Restart apas-web (compute version locally since server lacks git history)
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm install && NEXT_PUBLIC_WEB_UI_VERSION=${web_version} npm run build && systemctl restart apas-web"
```

## Key Concepts

1. **Managed Team Mode** (default): Manager, Tech Lead, Developer, and
   Reviewer panes coordinate through `project_goal.md`, `team-todo.md`, and
   `.apas-team.jsonl`.
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
