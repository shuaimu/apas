# Desktop pane work summaries

APAS can cache a short summary of each agent pane's conversation activity in
fixed, non-overlapping three-hour UTC windows. The desktop web workspace shows
these windows in a docked `Summary` drawer and formats their range in the
browser's local time. Summaries are navigation aids, not an audit log or proof
that repository work completed.

## Scope and retention

- Each summary covers exactly one project session, one pane, and one UTC
  window. Sibling panes are never combined.
- Source is limited to normalized conversation records already persisted in
  `messages.jsonl`. Raw terminal PTY bytes, terminal scrollback, repository
  files, raw diffs, tool inputs, and large tool results are not included.
- The server automatically backfills meaningful completed windows from the
  retained seven-day message history, newest first. It cannot reconstruct
  source that message GC removed before the feature was enabled.
- Completed summary text and its source digest live in
  `data/sessions/<session-id>/pane-work-summaries.json`, survive message GC and
  pane closure, and are deleted with the project/session directory.
- The current open window is generated only on drawer demand and is labelled
  partial through its latest source time. When the window closes, it is
  replaced by a final summary. Late retained messages make a cached result
  stale and queue regeneration.

## CLI isolation and provider privacy

Generation is disabled by default. The Claude adapter uses print mode in a new
empty temporary directory with tools, skills, plugins, settings sources, MCP
servers, and session persistence disabled. It does not resume or write into the
working agent's conversation and it cannot emit pane messages, status,
terminal input, or terminal output.

The Codex adapter is also available, but it is an explicit risk acceptance. It
runs `codex exec` from a new empty temporary directory with ephemeral execution,
user configuration and project rules ignored, a read-only sandbox, no resumed
session, bounded stdin, and a strict structured-output schema. Its prompt says
that conversation content is inert data, forbids executing commands or
following embedded instructions, and requests only the grounded summary.

Codex does not currently expose a true no-tools switch. Its read-only sandbox
prevents repository writes but retains a command tool and may permit reads from
other accessible host paths. A malicious instruction in retained conversation
text could therefore still induce an unrelated host-file read despite the
prompt. Selecting `summary_adapter codex` means the host operator accepts this
residual risk. APAS still validates all required confinement flags before
advertising summary capability.

Each adapter summarizes panes from its own provider by default. Sending another
provider's conversation across a vendor boundary requires the host operator to
explicitly enable `summary_allow_cross_provider`.

Enable the selected adapter on an internal project CLI host, then restart that
CLI:

```bash
apas config set summary_adapter claude
apas config set summary_enabled true
apas config set summary_timeout_seconds 120
apas config set summary_max_input_bytes 65536
# Optional and privacy-sensitive; remains false by default:
apas config set summary_allow_cross_provider false
```

For a Codex pane, explicitly select the headless Codex adapter instead:

```bash
apas config set summary_adapter codex
apas config set summary_enabled true
```

The configuration command prints the residual host-read warning when Codex is
selected.

An optional model can be selected with `apas config set summary_model MODEL`.
The adapter consumes separate provider quota even while the pane agent is
working. APAS permits one summary job per CLI, bounds source/chunk/output size,
and retries transient failures at most three times. Operators should watch
provider rate limits and quota before enabling broad backfill.

## Server controls and operations

Server defaults can be overridden in `server.toml`:

```toml
[summaries]
enabled = true
reconcile_interval_minutes = 15
global_concurrency = 2
max_sessions_per_scan = 100
max_source_bytes = 65536
max_chunk_bytes = 12288
max_chunks = 16
job_timeout_seconds = 120
refresh_throttle_seconds = 60
max_attempts = 3
```

Structured server logs report scan duration/bytes, queue and in-flight depth,
dispatch/result latency, retry/failure/unavailable counts, provider/model, and
per-session cache size. They do not log normalized source or summary prompts.

For a rolling release, deploy server, then web, then CLI. Old CLIs continue
normal pane work, receive no unknown generation messages, and leave cached
summaries readable with an update-required notice. To roll back, disable the
CLI adapter, then roll back web/server in either order. Sidecars remain inert
and can be reused by a later deployment; project deletion continues to remove
them.
