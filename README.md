# APAS - Autonomous Programming Agent System

APAS runs an autonomous programming team around your project. A Manager pane talks with the human and keeps `project_goal.md` current, a Tech Lead turns that goal into structured work in `team-todo.md`, and worker/reviewer panes implement changes in branches and open PRs for human review.

## Features

- **Team Mode**: Coordinate Manager, Tech Lead, Reviewer, and worker panes from one project
- **Shared Project State**: Keep goals in `project_goal.md` and work queues in `team-todo.md`
- **PR-Based Review**: Workers publish diffs, wait for Reviewer approval, then open pull requests for the human to merge
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
apas config set server ws://apas.mpaxos.com:8080

# start apas
apas
```

This will:
1. Create a `.apas` file in your project if it doesn't exist
2. Connect to the APAS server for web monitoring
3. Start the local CLI session and expose the project in the web Overview
4. Spawn or reconnect the default team panes so the Manager, Tech Lead, Reviewer, and workers can coordinate

### Team Mode

The Overview is the main control surface for team-mode projects:

1. Start or open the Manager pane.
2. Describe the project goal, or ask the Manager to scan the repo and draft one. The Manager keeps `project_goal.md` in sync.
3. Start the Tech Lead. It reads `project_goal.md` and `team-todo.md`, proposes Global TODOs, and dispatches approved work to worker panes.
4. Approve or reject proposed Global TODOs from the Overview TODO panel.
5. Review worker PRs on GitHub. Workers wait for Reviewer approval before opening PRs, and they wait for the human to merge them.

### Configuration

The `.apas` file in your project directory contains:

```json
{
  "id": "uuid-of-your-project",
  "name": "project-name",
  "created_at": "timestamp",
  "prompt": "Your custom prompt here (optional)"
}
```

If no custom `prompt` is specified, APAS uses the built-in team-mode prompts for the default Manager, Tech Lead, Reviewer, and Developer panes. You can customize pane roles, goals, backstories, and prompts in `.apas` as your workflow matures.

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

- **APAS CLI**: Wraps Claude Code, sends structured output to server
- **APAS Server**: Routes messages between CLI and web clients
- **Web UI**: Displays real-time Claude output and project status

## Development

### Project Structure

```
apas/
├── crates/
│   ├── client-cli/    # APAS CLI (apas binary)
│   ├── server/        # APAS server
│   └── shared/        # Shared types and messages
├── packages/
│   └── web/           # Next.js web dashboard
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
