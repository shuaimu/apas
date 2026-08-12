# APAS - Autonomous Programming Agent System

APAS runs an autonomous programming team around your project. A Manager pane talks with the human and keeps `project_goal.md` current, a Tech Lead turns that goal into structured work in `team-todo.md`, and worker/reviewer panes implement changes in branches and open PRs for human review.

## Features

- **Team Mode**: Coordinate Manager, Tech Lead, Reviewer, and worker panes from one project
- **Shared Project State**: Keep goals in `project_goal.md` and work queues in `team-todo.md`
- **PR-Based Review**: Workers publish diffs, wait for Reviewer approval, then open pull requests while the Tech Lead tracks PR state and routes comments
- **Web Dashboard**: Use the Overview to inspect panes, manage TODOs, and observe work in real time
- **Customizable Prompts**: Define role prompts and workflow behavior in the `.apas` config file
- **Auto-Updates**: CLI automatically checks for updates on startup

## Installation

### Quick Install

```bash
curl -sSL https://raw.githubusercontent.com/shuaimu/apas/master/install.sh | bash
```

This will clone and build from source, installing to `~/.local/bin/`. Requires Rust (will install via rustup if not present).

### Manual Build

```bash
git clone https://github.com/shuaimu/apas.git
cd apas
cargo build --release -p apas
cp target/release/apas ~/.local/bin/
```

### Update

```bash
apas update
```

This rebuilds from the latest source. The CLI also checks for updates every 24 hours and notifies you if a new version is available.

## Usage

### Basic Usage

Navigate to your project directory and run:

```bash
# setup ws service or your service
apas config set server wss://apas.mpaxos.com

# start apas
apas
```

This will:
1. Create a `.apas` file in your project if it doesn't exist
2. Connect to the APAS server for web monitoring
3. Start the local CLI session and expose the project in the web Overview
4. Leave team launch under your control: use the Overview **Team setup** card to pick provider/model choices and click **Start team**

### Team Mode

The Overview is the main control surface for team-mode projects:

1. Open the **Team setup** card.
2. Pick the provider/model for each managed role: Manager, Tech Lead, Developer, and Reviewer.
3. Click **Start team** to launch the selected managed panes.
4. Describe the project goal in the Manager pane, or ask the Manager to scan the repo and draft one. The Manager keeps `project_goal.md` in sync.
5. Let the Tech Lead read `project_goal.md` and `team-todo.md`, propose Global TODOs, and dispatch approved work to worker panes.
6. Approve or reject proposed Global TODOs from the Overview TODO panel.
7. Review and merge worker PRs on GitHub. Workers wait for Reviewer approval before opening PRs; after a PR opens, the Tech Lead tracks merge or close state and routes PR comments back to the owning worker.

#### Tech Lead autonomy

The Overview includes opt-in Tech Lead autonomy toggles backed by `.apas`.
Both default to manual control: `auto_approve_todos: false` and
`auto_merge_prs: false`. The Tech Lead re-reads these flags each loop.
`auto_approve_todos` lets it approve its own proposed Global TODOs;
`auto_merge_prs` remains gated by Reviewer approval, mergeability, and
green/non-stale CI state. Leaving both off is safest; enabling either
trades review latency for more autonomous execution.

### Configuration

The `.apas` file in your project directory contains:

```json
{
  "id": "uuid-of-your-project",
  "name": "project-name",
  "created_at": "timestamp",
  "auto_approve_todos": false,
  "auto_merge_prs": false,
  "prompt": "Your custom prompt here (optional)"
}
```

If no custom `prompt` is specified, APAS uses the built-in team-mode prompts for the default Manager, Tech Lead, Reviewer, and Developer panes. You can customize pane roles, goals, backstories, prompts, and Tech Lead autonomy flags in `.apas` as your workflow matures.

### CLI Options

```bash
apas --help              # Show help
apas --version           # Show version
apas update              # Check for updates
apas config show         # Show configuration
apas config set KEY VAL  # Set configuration value
apas --offline           # Run in offline mode (no server)
apas -d /path/to/dir     # Specify working directory
```

Pane work summaries are available on desktop, responsive mobile web, and the
native app when the isolated CLI feature is enabled. See
[docs/pane-work-summaries.md](docs/pane-work-summaries.md) for scope, privacy,
retention, enablement, and rollback guidance.

## Architecture

```
+------------------+     +--------------+     +-----------------+
|   Claude Code    | <-- |  APAS CLI    | --> |  APAS Server   |
| (runs locally)   |     | (Rust)       |     | (Rust/Axum)    |
+------------------+     +--------------+     +-----------------+
                                                      |
                                                      v
                                              +-----------------+
                                              |   Web UI        |
                                              | (Next.js)       |
                                              +-----------------+
```

- **APAS CLI**: Owns local panes, role prompts, isolated worktrees,
  and the team-mode files `project_goal.md`, `team-todo.md`, and
  `.apas-team.jsonl`.
- **APAS Server**: Routes project/session state, pane events, TODO
  updates, and PR/status records between CLI clients and web clients.
- **Web UI**: Provides the Overview team controls for starting roles,
  approving TODOs, reviewing suggested workers, and tracking worker PR
  handoffs and status.

## Development

### Project Structure

```
apas/
├── crates/
│   ├── client-cli/    # APAS CLI: panes, role prompts, worktrees, team files
│   │   └── src/
│   │       ├── role.rs             # Built-in Manager/Tech Lead/worker prompts
│   │       ├── team_todo.rs        # team-todo.md parsing and state changes
│   │       └── mode/dual_pane.rs   # Pane spawning, Start team, team loop wiring
│   ├── server/        # Rust/Axum server for project/session state and routing
│   └── shared/        # Shared wire types and messages
├── packages/
│   └── web/           # Next.js web dashboard
│       └── src/
│           ├── components/overview/ # Team setup, TODOs, suggestions, PR views
│           └── lib/store.ts         # Websocket store and Overview actions
└── install.sh         # Installation script
```

### Building

```bash
# Build everything
cargo build

# Build CLI only
cargo build -p apas

# Build server only
cargo build -p apas-server
```

### Running Locally

```bash
# Start server
cargo run -p apas-server

# Start CLI (in another terminal)
cargo run -p apas

# Start web UI (in another terminal)
cd packages/web
npm run dev
```

## License

MIT
