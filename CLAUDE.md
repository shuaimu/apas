# APAS - Claude Code Web Interface

APAS is a web interface for Claude Code that allows you to observe and interact with Claude CLI sessions from a browser.

## Architecture

```
┌─────────────────┐     WebSocket      ┌─────────────────┐     WebSocket     ┌─────────────────┐
│   CLI Client    │ ◄──────────────────►│     Server      │ ◄────────────────►│   Web Frontend  │
│  (apas binary)  │   ws://host:8081    │  (apas-server)  │   ws://host:8081  │   (Next.js)     │
└─────────────────┘                     └─────────────────┘                    └─────────────────┘
        │                                       │
        ▼                                       ▼
┌─────────────────┐                     ┌─────────────────┐
│   Claude CLI    │                     │  SQLite + Files │
│ (stream-json)   │                     │   (data/)       │
└─────────────────┘                     └─────────────────┘
```

## Project Structure

```
apas/
├── crates/
│   ├── client-cli/      # CLI binary (apas)
│   │   ├── src/
│   │   │   ├── main.rs      # Entry point, CLI args
│   │   │   ├── config.rs    # Config file handling
│   │   │   ├── project.rs   # .apas file management
│   │   │   ├── claude.rs    # Claude process wrapper
│   │   │   └── mode/
│   │   │       ├── hybrid.rs  # Default: local CLI + streaming to server
│   │   │       ├── local.rs   # Offline mode
│   │   │       └── remote.rs  # Remote-only mode
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
│   │   │       ├── ws_cli.rs  # CLI WebSocket handler
│   │   │       └── ws_web.rs  # Web WebSocket handler
│   │   └── Cargo.toml
│   │
│   └── shared/          # Shared types between CLI and server
│       ├── src/
│       │   ├── lib.rs
│       │   └── messages.rs  # All WebSocket message types
│       └── Cargo.toml
│
├── packages/
│   └── web/             # Next.js web frontend
│       ├── src/
│       │   ├── app/
│       │   │   ├── layout.tsx
│       │   │   └── page.tsx
│       │   ├── components/
│       │   │   ├── Sidebar.tsx       # Project list
│       │   │   ├── chat/             # Message display
│       │   │   ├── code/             # Code blocks
│       │   │   └── tools/            # Tool cards
│       │   └── lib/
│       │       └── store.ts          # Zustand state management
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
server = "ws://localhost:8081"
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

- **CliToServer**: Register, SessionStart, StreamMessage, UserInput, Heartbeat
- **ServerToCli**: Registered, SessionAssigned, Input, Signal
- **WebToServer**: Authenticate, ListCliClients, AttachSession, Input
- **ServerToWeb**: Authenticated, CliClients, SessionMessages, StreamMessage, UserInput

## Data Storage

- **SQLite** (`data/apas.db`): Users, CLI clients, sessions metadata
- **JSONL files** (`data/sessions/{id}/messages.jsonl`): Chat messages per session

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
- `NEXT_PUBLIC_WS_URL`: WebSocket URL for web frontend (default: `ws://localhost:8081`)

## Key Concepts

1. **Hybrid Mode** (default): CLI runs locally, streams output to server for web observation
2. **Project-based Sessions**: Sessions identified by project directory (`.apas` file)
3. **Stream-JSON**: Uses Claude CLI's `--output-format stream-json` for structured output
4. **Real-time Updates**: WebSocket connections for live message streaming
