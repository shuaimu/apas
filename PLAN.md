# APAS - Claude Code Web/Mobile Wrapper

A tool that wraps Claude Code CLI and exposes it through a web and mobile interface.

## Architecture Overview

```
┌─────────────────┐                              ┌─────────────────┐
│  Web Frontend   │─────────┐          ┌────────│  Client CLI     │
│  (Next.js)      │         │          │        │  (apas)         │
└─────────────────┘         ▼          ▼        │  [Local Mode]   │
                     ┌──────────────────┐       │  [Remote Mode]  │
┌─────────────────┐  │  Backend Server  │       └─────────────────┘
│  Mobile App     │─▶│  (Rust/Axum)     │◀──────┌─────────────────┐
│  (PWA)          │  │                  │       │  Client CLI     │
└─────────────────┘  │  - Routes msgs   │       │  (another       │
                     │  - Auth/Sessions │       │   machine)      │
                     │  - SQLite DB     │       └─────────────────┘
                     └──────────────────┘              ...

Communication: WebSocket (bidirectional)
```

## Components

### 1. Client CLI (`apas`)

A Rust CLI that wraps Claude Code with two operational modes:

#### Local Mode (default)
- Behaves exactly like Claude Code
- Stdin/stdout pass-through to local terminal
- No network connection required

```bash
$ apas                    # Local mode (default)
$ apas --local
$ apas -l
```

#### Remote Mode
- Connects to backend server via WebSocket
- Disables local stdin/stdout
- Streams all I/O to/from backend server
- Supports reconnection and heartbeat

```bash
$ apas --remote
$ apas -r
$ apas --remote --server wss://api.example.com --token <token>
```

#### Configuration
```bash
$ apas config set server wss://api.example.com
$ apas config set token <auth_token>
```

### 2. Backend Server

A Rust server (Axum) that acts as a message broker:

#### Responsibilities
- **WebSocket endpoints**: Separate endpoints for web/mobile clients and CLI clients
- **Session routing**: Maps web sessions to CLI clients
- **Authentication**: JWT-based auth for all connections
- **Persistence**: SQLite for users, sessions, and message history

#### Endpoints
| Endpoint | Purpose |
|----------|---------|
| `GET /health` | Health check |
| `POST /auth/register` | User registration |
| `POST /auth/login` | User login, returns JWT |
| `WS /ws/web` | WebSocket for web/mobile clients |
| `WS /ws/cli` | WebSocket for CLI clients |

### 3. Web Frontend

A Next.js application with conversation-style UI:

#### UI Components
- **MessageList**: Scrollable conversation history
- **UserMessage**: User input bubbles
- **AssistantMessage**: Claude responses with markdown
- **CodeBlock**: Syntax-highlighted code snippets
- **ToolCard**: Collapsible cards showing tool usage (Read, Edit, Bash, etc.)
- **ApprovalPrompt**: Interactive approve/reject for tool permissions
- **InputBox**: Text input with send button

#### Mobile Support
- PWA (Progressive Web App) for mobile browsers
- Responsive design for all screen sizes

## Technology Stack

### Backend (Rust)

| Component | Library |
|-----------|---------|
| Web Framework | Axum 0.7 |
| Async Runtime | Tokio |
| WebSocket | axum + tokio-tungstenite |
| Database | SQLx + SQLite |
| Auth | jsonwebtoken |
| Password Hashing | argon2 |
| Serialization | Serde |
| CLI Parsing | Clap 4 |
| Config | toml + directories |

### Frontend

| Component | Library |
|-----------|---------|
| Framework | Next.js 14 |
| Styling | Tailwind CSS |
| Markdown | react-markdown |
| Code Highlighting | react-syntax-highlighter |
| State Management | Zustand |
| Icons | Lucide React |

## Project Structure

```
apas/
├── crates/
│   ├── server/                  # Backend server
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── config.rs
│   │   │   ├── error.rs
│   │   │   ├── routes/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── ws_web.rs    # WebSocket for web/mobile
│   │   │   │   ├── ws_cli.rs    # WebSocket for CLI clients
│   │   │   │   ├── auth.rs
│   │   │   │   └── health.rs
│   │   │   ├── session/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── router.rs    # Routes messages web <-> cli
│   │   │   │   └── store.rs
│   │   │   └── db/
│   │   │       ├── mod.rs
│   │   │       └── models.rs
│   │   ├── migrations/
│   │   │   └── 001_init.sql
│   │   └── Cargo.toml
│   │
│   ├── client-cli/              # CLI wrapper
│   │   ├── src/
│   │   │   ├── main.rs          # Entry point, arg parsing
│   │   │   ├── config.rs        # Config management
│   │   │   ├── claude.rs        # Claude Code process
│   │   │   ├── mode/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── local.rs     # Local mode
│   │   │   │   └── remote.rs    # Remote mode
│   │   │   └── stream.rs        # I/O abstraction
│   │   └── Cargo.toml
│   │
│   └── shared/                  # Shared types
│       ├── src/
│       │   ├── lib.rs
│       │   └── messages.rs
│       └── Cargo.toml
│
├── packages/
│   └── web/                     # Next.js frontend
│       ├── app/
│       │   ├── layout.tsx
│       │   ├── page.tsx
│       │   └── globals.css
│       ├── components/
│       │   ├── chat/
│       │   │   ├── MessageList.tsx
│       │   │   ├── UserMessage.tsx
│       │   │   ├── AssistantMessage.tsx
│       │   │   └── InputBox.tsx
│       │   ├── code/
│       │   │   ├── CodeBlock.tsx
│       │   │   └── DiffView.tsx
│       │   └── tools/
│       │       ├── ToolCard.tsx
│       │       └── ApprovalPrompt.tsx
│       ├── lib/
│       │   ├── websocket.ts
│       │   ├── messageParser.ts
│       │   └── store.ts
│       └── package.json
│
├── Cargo.toml                   # Workspace root
├── package.json
└── PLAN.md
```

## Message Types

### CLI <-> Server

```rust
// Client CLI -> Server
enum CliToServer {
    Register { token: String },           // CLI registers with auth token
    Output { session_id: Uuid, data: String },  // Claude output
    SessionEnd { session_id: Uuid, reason: String },
    Heartbeat,
}

// Server -> Client CLI
enum ServerToCli {
    Registered { cli_id: Uuid },
    SessionAssigned { session_id: Uuid },
    Input { session_id: Uuid, data: String },   // User input from web
    Signal { session_id: Uuid, signal: String }, // SIGINT, etc.
    Heartbeat,
}
```

### Web <-> Server

```rust
// Web -> Server
enum WebToServer {
    Authenticate { token: String },
    Input { text: String },
    Approve { tool_call_id: String },
    Reject { tool_call_id: String },
}

// Server -> Web
enum ServerToWeb {
    Authenticated { user_id: Uuid },
    Output { content: String, message_type: MessageType },
    SessionStatus { status: SessionStatus },
    Error { message: String },
}

enum MessageType {
    Text,
    Code { language: String },
    ToolUse { tool: String, input: Value },
    ToolResult { tool: String, output: String },
    ApprovalRequest { tool: String, description: String },
}
```

## Database Schema (SQLite)

```sql
-- Users table
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Sessions table
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    cli_client_id TEXT,
    status TEXT DEFAULT 'pending',  -- pending, connected, disconnected
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Messages table (conversation history)
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    role TEXT NOT NULL,             -- user, assistant
    content TEXT NOT NULL,
    message_type TEXT DEFAULT 'text',
    metadata TEXT,                  -- JSON for tool calls, etc.
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- CLI clients table
CREATE TABLE cli_clients (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    name TEXT,
    last_seen DATETIME,
    status TEXT DEFAULT 'offline',  -- online, offline
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

## Implementation Phases

### Phase 1: Foundation
- [ ] Initialize Rust workspace with 3 crates: server, client-cli, shared
- [ ] Define shared message types

### Phase 2: Client CLI
- [ ] Argument parsing with clap (--local, --remote flags)
- [ ] Configuration management (server url, token)
- [ ] Local mode - transparent Claude Code wrapper
- [ ] Claude Code process spawning and I/O handling
- [ ] Remote mode - WebSocket connection to backend
- [ ] Remote mode - stream I/O to/from backend
- [ ] Remote mode - reconnection and heartbeat

### Phase 3: Backend Server
- [ ] Axum server with dual WebSocket endpoints (web + cli)
- [ ] Session routing (web <-> cli mapping)
- [ ] User authentication (JWT)
- [ ] SQLite integration for users/sessions

### Phase 4: Frontend
- [ ] Initialize Next.js frontend
- [ ] Build conversation UI components (MessageList, UserMessage, AssistantMessage)
- [ ] Build CodeBlock component with syntax highlighting
- [ ] Build ToolCard component for tool usage display
- [ ] Build ApprovalPrompt component for tool permissions
- [ ] Connect frontend to backend via WebSocket

### Phase 5: Deployment
- [ ] Docker configuration for backend
- [ ] PWA support for mobile

## Configuration Files

### Server Config (`server.toml`)
```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
path = "./data/apas.db"

[auth]
jwt_secret = "your-secret-key"
token_expiry_hours = 24
```

### CLI Config (`~/.config/apas/config.toml`)
```toml
[remote]
server = "wss://api.example.com"
token = "your-auth-token"

[local]
claude_path = "claude"  # Path to claude-code binary
```

## Security Considerations

1. **Authentication**: JWT tokens for all connections
2. **Authorization**: Users can only access their own sessions
3. **Token Storage**: CLI tokens stored securely using OS keychain when available
4. **WebSocket Security**: WSS (TLS) required in production
5. **Input Validation**: All inputs sanitized before processing

## Future Enhancements

- [ ] Multiple CLI clients per user (select which machine to use)
- [ ] Session sharing/collaboration
- [ ] Session history replay
- [ ] File browser integration
- [ ] Git integration (view diffs, commits)
- [ ] Notifications for long-running tasks
- [ ] React Native mobile app
