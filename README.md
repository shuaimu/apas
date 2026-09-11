# APAS - Autonomous Programming Agent System

APAS runs coding agents against your projects, from a browser or a phone.

A project is any directory with an `.apas` file. The work happens in **panes**,
each hosting one agent. You create a pane, talk to it, reboot it, and close it.
There is no orchestration layer above that: a pane does what you ask it to, and
nothing dispatches work between panes on its own.

The CLI owns the panes and worktrees on your machine, a server brokers project
and session state, and the web UI gives you the pane tabs, the conversation and
terminal views, the project overview, and the diff and pull-request handoffs.

> **Managed team mode was removed.** Four coordinating roles (Manager, Tech
> Lead, Developer, Reviewer) used to pass work to each other through
> `project_goal.md`, `team-todo.md`, and `.apas-team.jsonl`. If you find a
> reference to those files, to a team role, to the TODO queue, or to
> `apas mcp-server` in an old checkout or an old doc, it is stale. Some `.apas`
> files still carry `team_enabled` or `managed: true`; both are ignored and load
> as ordinary panes.

## Features

- **Terminal panes**: each pane runs a provider's real interactive TUI on a pty
  and streams it to the browser, so you get the CLI exactly as it ships
- **Claude, Codex, OpenCode, and Pi**, plus Claude against a DeepSeek backend
- **Read it as a conversation**: a per-pane toggle swaps the live terminal for a
  structured transcript, recovered from the provider's own session files
- **Drive it from a phone**: type into the conversation view and answer an
  agent's questions there, which is the practical way to work when an xterm is
  not
- **Isolated worktrees**: give a pane its own git branch and review its diff
- **Your own cluster**: register machines, start and stop projects, and share
  compute with collaborators
- **Live updates**: pane state, diffs, and terminal output stream over WebSockets

## Installation

### Quick install

```bash
curl -sSL https://raw.githubusercontent.com/shuaimu/apas/master/install.sh | bash
```

This clones and builds from source into `~/.local/bin/`, installing Rust via
rustup if it is missing.

The CLI ships as a **static musl** binary so that a self-update never leaves it
depending on the build machine's glibc. Building it the way it ships needs the
musl target and a musl C compiler:

```bash
rustup target add x86_64-unknown-linux-musl   # aarch64-… on ARM
sudo apt-get install -y musl-tools

cargo build --release --target x86_64-unknown-linux-musl -p apas
```

If the musl toolchain is missing, both `install.sh` and the self-updater fall
back to a glibc build, so an update can never brick the binary.

### Update

```bash
apas update
```

A launch also checks for a newer version and installs it before starting.

A **daemon**, though, is replaced only when you ask. It now hosts every project
on the machine as a supervised task, so replacing it means stopping all of them
and starting them again. Use **Reboot to update** on the Machines page, which
applies any pending update first and then restarts. Each machine shows the
version its daemon reports, so a host that has fallen behind is visible rather
than silent.

## Getting started

```bash
# 1. Point the CLI at a server and sign in.
apas config set server wss://apas.mpaxos.com
apas login

# 2. Register a project. This exits immediately; it does not start anything.
cd /path/to/your/project
apas

# 3. Open the web UI, find the project on the Machines page, and start it.
#    Then add a pane and talk to it.
```

Running `apas` in a directory **registers the project and exits**. It does not
open a terminal UI, and it does not start the project. A host runs **one APAS
instance per user**, and projects are started from the web. Running `apas` in a
directory that is not yet a project creates and registers it.

`apas --attach` opens a local terminal UI for a project already running on the
host. It is the fallback for when the web is unreachable and shows little beyond
pane names.

### Accounts

Registration needs either an **invitation**, which an administrator issues and
which works with any address, or an email address in a domain the deployment
has opened for self-signup. The signup page states which applies.

## Key concepts

### Panes

A pane hosts one agent. `PaneConfig.kind` decides how, and is independent of
`provider` (which binary) and `mode` (how autonomous).

New panes are **terminal panes** (`kind: "terminal"`). APAS allocates a pty,
execs the provider's real interactive TUI, and streams the raw bytes to xterm.js
in the browser. Nothing is parsed, so nothing has to keep pace with a provider's
output format.

`kind: "agent"` is the older structured path, where the CLI runs the provider
headlessly and parses its stream-JSON. It is kept only for panes that already
exist. Nothing creates one any more.

Only Claude, Codex, OpenCode, and Pi can host a terminal pane. DeepSeek runs
the Claude binary against an Anthropic-compatible endpoint. APAS does not
install or authenticate these tools; install and log into them on each host
first.

### Terminal panes have history

An agent pane is observed directly, because the CLI parses its output. A
terminal pane has nothing structured to parse, so APAS reads the transcript each
provider already writes:

- **Claude** reports its own transcript through a `SessionStart` hook, so the
  pane is never guessing which conversation it is in.
- **Codex** is located by the terminal's process group, so several panes can
  share one directory without sharing history.
- **OpenCode** is asked for its session list and export.
- **Pi** pins its exact session id at launch and is read back from that session
  file, so sibling panes and subagents sharing a directory never mix.

That transcript is what gives a terminal pane its conversation view, its token
counts, and its working/idle state. Questions a Claude agent asks appear there
and can be answered there, and typed messages go straight into the live pty. Pi
questions stay in the terminal view: its `ask` tool comes from the oh-my-pi
extension and its picker has not been verified, so APAS does not write answers
into it blindly.

### Pi and oh-my-pi

Install and authenticate Pi on the host (`npm install -g
@earendil-works/pi-coding-agent`, or the installer at
[pi.dev](https://pi.dev)). Override a nonstandard installation with
`apas config set pi_path /path/to/pi`, then allow `terminal:pi:official:default`
through cluster or project policy.

The [oh-my-pi](https://github.com/can1357/oh-my-pi) orchestration layer is a Pi
package, not a separate agent. Install it into Pi **globally**:

```bash
pi install npm:oh-my-pi
```

A global install means APAS panes never hit Pi's project-trust prompt. If you
install it project-locally (`-l`), answer `/trust` once in the pane's terminal
view; APAS deliberately never trusts repository-controlled extensions on your
behalf. Run `/oh-my-pi doctor` inside a Pi pane to diagnose an install — the
package's published shell launcher is broken (it imports TypeScript sources the
tarball does not ship), so the slash command is the supported entry point.

### Worktrees and diff review

A pane can be given its own git worktree and branch, under
`.apas-worktrees/pane-<id>/`, so it works in isolation. When it has one, a
**Diff** control appears in its header showing the unified diff against the
project's main branch, split into per-file sections and refreshed as the pane's
HEAD moves. From there you can **merge and close** the pane, or **discard** it
along with its branch. Closing a pane that owns a worktree offers the same
choice: leave the branch alone, merge it, or discard everything.

Diffs are computed from git, so they work for any pane with a worktree,
terminal panes included.

**Plan review** is the older per-pane policy that holds an agent's tool calls
until you approve them, set from the pane's role modal. It is built on the
structured event stream, so it applies only to legacy `agent` panes; a terminal
pane runs the provider's own permission prompts instead.

### Pane hosts

On Unix hosts, each terminal pane is owned by a hidden `apas pane-host` process
in its own tmux session, and the project CLI is only its authenticated
controller. This takes the provider's lifetime out of the CLI's hands: a
transport reconnect leaves everything running, and a CLI reboot re-adopts the
same live terminals afterward.

### Projects run inside one instance

A host runs one `apas` process per user, with the projects running inside it as
supervised tasks, plus one pane host per terminal pane. Stopping a project sets
a flag it observes rather than aborting it, and a project that panics unwinds
its own task without taking the others down.

### Clusters and administration

These are two separate jobs with two separate surfaces.

**Your virtual cluster** is `/machines`, available to every account with no role
check. It is derived, not stored: the machines your client registered, plus the
projects hosted on them. A project is hosted in your cluster if you own it or if
one of its sessions was created under it, so a project someone else owns but
runs on your machine is yours to administer. You can share that cluster with
another account, scoped to specific machines.

**System administration** is `/admin`. It is a credential, not an account: one
per deployment, stored outside the users table, with its own login. No UI can
grant it, and its token authorizes nothing else.

Belonging to a project is deliberately not the same as hosting it. Content
access is owner, member, or host; administration is host only.

## Configuration

### CLI config

`~/.config/apas/config.toml`:

```toml
[remote]
server = "wss://apas.mpaxos.com"
token = "your-token"

[local]
claude_path = "claude"
codex_path = "codex"
opencode_path = "opencode"
pi_path = "pi"
```

Set values with `apas config set KEY VALUE`. Useful keys include `server`,
`claude_path`, `codex_path`, `opencode_path`, `pi_path`, `deepseek_api_base_url`,
`deepseek_api_key`, `daemon_roots`, and the pane-host grace periods
`pane_host_adoption_grace_seconds` and `pane_host_reboot_grace_seconds`.

### Project file

Each project directory gets an `.apas` file holding its identity and its
restored panes:

```json
{
  "id": "uuid",
  "name": "project-name",
  "created_at": "2026-01-01T00:00:00Z",
  "disallowed_tab_types": [],
  "panes": [
    {
      "pane_id": 555,
      "label": "Claude 2",
      "kind": "terminal",
      "provider": "claude",
      "mode": "interactive"
    },
    {
      "pane_id": 920,
      "label": "Codex 2",
      "kind": "terminal",
      "provider": "codex",
      "mode": "interactive"
    }
  ]
}
```

A new project has **no panes**; you open what you want. A missing `kind` still
loads as `agent` so that files written before terminal panes existed keep
working.

`disallowed_tab_types` restricts which tab types users may create, where a tab
type is a pane kind plus a provider (`terminal:claude`, `terminal:codex`,
`terminal:opencode`, `terminal:pi`). It is stored as a deny list, so an empty
value means everything is allowed and a provider added later is permitted until
an owner says otherwise. The CLI enforces it by re-reading `.apas` on every
request, because the web only hides menu entries.

`auto_approve_todos` and `auto_merge_prs` may still appear in older files. They
were read by the team loop and now do nothing.

## CLI reference

```bash
apas                     # Register the project in this directory, then exit
apas --attach            # Open the local terminal UI for a running project
apas --offline           # Run without a server
apas -d /path/to/dir     # Act on another directory
apas --headless          # Run one project without a TUI (debugging)

apas login               # Sign in to the server
apas whoami              # Show login status
apas logout              # Sign out

apas config show         # Show configuration
apas config set KEY VAL  # Set a configuration value
apas update              # Check for updates and install
apas daemon              # Run the per-machine daemon
apas worktree            # Manage per-pane isolated git worktrees
```

## Architecture

```
┌─────────────────────┐    WebSocket     ┌─────────────────┐    WebSocket    ┌─────────────────────┐
│ CLI client          │ ◄───────────────►│ apas-server     │◄───────────────►│ Web frontend        │
│ (apas binary)       │                  │                 │                 │ (Next.js)           │
└─────────────────────┘                  └─────────────────┘                 └─────────────────────┘
        │                                         │
        │                                         ▼
        │                                ┌─────────────────┐
        │                                │ SQLite + JSONL  │
        │                                └─────────────────┘
        ▼
┌─────────────────────┐
│ Pane hosts (tmux)   │
│ Claude/Codex/…      │
│ pty + worktrees     │
└─────────────────────┘
```

- **APAS CLI** owns local panes, pane identity, isolated worktrees, and the
  `.apas` file, and supervises the pane hosts that hold the pty for each
  terminal pane.
- **APAS server** brokers project, session, and machine state between CLI and
  web clients, and keeps a bounded in-memory buffer of terminal output per pane.
  Terminal bytes are deliberately never persisted.
- **Web UI** provides the pane tabs, terminal and conversation views, diffs, the
  project overview, and the cluster and administration surfaces.

## Development

### Project structure

```
apas/
├── crates/
│   ├── client-cli/           # The apas binary
│   │   └── src/
│   │       ├── main.rs               # Entry point and config commands
│   │       ├── project.rs            # .apas project metadata
│   │       ├── pane_identity.rs      # Role/goal/backstory as a system prompt
│   │       ├── terminal_pane.rs      # pty host for terminal panes
│   │       ├── pane_host.rs          # Persistent per-pane host process
│   │       ├── claude_session_hook.rs# Which transcript a Claude pane writes
│   │       ├── worktree.rs           # Isolated worktree create/diff/cleanup
│   │       └── mode/dual_pane.rs     # Pane runtime: panes, deadloops, watchers
│   ├── server/               # Rust/Axum WebSocket + HTTP server
│   │   └── src/routes/               # ws_cli, ws_web, ws_daemon, auth, admin, cluster
│   └── shared/               # Wire types shared by CLI and server
├── packages/
│   ├── web/                  # Next.js web UI
│   ├── mobile/               # Mobile companion app
│   ├── protocol/             # Shared protocol definitions
│   └── terminal-web/         # Terminal rendering support
├── docs/                     # Design and operations notes
├── deploy/                   # nginx config and deployment assets
└── install.sh
```

### Building

```bash
cargo build                  # Everything
cargo build -p apas          # CLI only
cargo build -p apas-server   # Server only
```

### Running locally

```bash
# Terminal 1: server
RUST_LOG=info cargo run -p apas-server

# Terminal 2: web UI
cd packages/web && npm install && npm run dev

# Terminal 3: register a project
cargo run -p apas
```

### Tests

```bash
cargo test                                  # Rust
cd packages/web && npm test                 # Web
```

If Rust tests fail with `pool timed out while waiting for an open connection`,
the temp directory is out of space. The suite builds SQLite databases under
`TMPDIR`; point it somewhere with room:

```bash
TMPDIR=/var/tmp cargo test -p apas-server
```

## Further reading

**[CLAUDE.md](CLAUDE.md) is the canonical contributor and agent runbook**, with
the architecture, deployment procedure, and the reasoning behind decisions that
are easy to undo by accident. Read it before changing anything here.

- [docs/shared-clusters.md](docs/shared-clusters.md) — sharing compute: the
  trust boundary, role matrix, and rollout order
- [docs/cluster-administration.md](docs/cluster-administration.md) — cluster and
  deployment administration
- [docs/pane-work-summaries.md](docs/pane-work-summaries.md) — scope, privacy,
  retention, and rollback for pane work summaries
- [docs/one-instance-per-host.md](docs/one-instance-per-host.md) — why a host
  runs a single instance
- [docs/mobile-development-and-operations.md](docs/mobile-development-and-operations.md)
  and [docs/mobile-threat-model.md](docs/mobile-threat-model.md) — the mobile app

## License

MIT
