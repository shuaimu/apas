# APAS - Autonomous Programming Agent System

APAS is an autonomous coding agent that wraps Claude Code CLI and runs in a continuous loop to work through tasks defined in a TODO.md file.

## Features

- **Autonomous Mode**: Runs Claude Code in a dead loop, continuously working through tasks
- **Customizable Prompts**: Define your workflow in the `.apas` config file
- **Web Dashboard**: Monitor and observe Claude's work in real-time via web UI
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
apas
```

This will:
1. Create a `.apas` file in your project if it doesn't exist
2. Connect to the APAS server for web monitoring
3. Start Claude Code in autonomous mode with the default workflow

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

If no `prompt` is specified, the default 7-step workflow is used:

1. Pick a task from TODO.md
2. Analyze and break down if needed
3. Execute the task with a plan
4. Run tests and fix failures
5. Prepare for commit (check for unsafe code)
6. Git commit and push
7. Loop back to step 1

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
