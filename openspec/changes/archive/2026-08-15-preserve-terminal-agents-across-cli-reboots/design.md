## Context

See `proposal.md` for motivation. Today `run_server_connection` already reconnects a lost WebSocket without disturbing panes, but the web exposes only `RebootCli`. That command sets the project-wide shutdown flag and calls `exec()` after optional self-update. Terminal PTY masters, output-reader threads, structured-agent pipes, and control state all live inside that replaceable process. On startup APAS can re-execute a provider with Claude `--resume`, Codex `resume`, or OpenCode `--continue`, but that creates a different process and interrupts any turn that was in flight.

The machine daemon already uses project-scoped tmux servers so headless project CLIs survive daemon replacement. Reusing that supervision boundary is attractive, but a plain tmux pane is not sufficient for APAS adoption: `capture-pane` returns a rendered screen rather than ordered raw PTY bytes, provides no APAS authentication or ownership protocol, and cannot reliably bridge input, resize, lifecycle, and sequence reconciliation.

## Goals / Non-Goals

**Goals:**

- Make transport recovery an explicit in-process operation with no pane lifecycle side effects.
- Preserve the exact live provider process for supported terminal panes while the project CLI binary is replaced.
- Reuse the existing server instance/sequence model so transport recovery and reboot cannot duplicate or reorder output.
- Make local adoption exclusive, authenticated, bounded, observable, and safely cleanable.
- Roll out across mixed server, web, daemon, CLI, and persistent-host versions without turning an unsupported request into a destructive operation.

**Non-Goals:**

- Preserving legacy structured agent processes across a full CLI replacement. Their JSON parsers, pending tool decisions, input queues, and status inference remain in-process.
- Preserving a terminal provider across host reboot, cross-host project movement, tmux-server failure, or kernel process loss.
- Durably storing raw terminal bytes on the server or host.
- Hot-upgrading an already running pane-host executable. Existing panes may remain on the compatible host version that created them until they close.
- Providing persistent terminal hosting on non-Unix platforms in the first release.

## Decisions

### 1. Leave transport recovery automatic; make reboot the only lifecycle control

Introduce a capability-gated lifecycle request with a client-generated request ID and operation enum carrying exactly one operation: `RebootCli`, for binary replacement. The server reauthorizes the project, records a bounded pending operation, and routes the exact request to the owning CLI.

Connection recovery needs no request at all. `run_server_connection` already re-dials a lost WebSocket from its outer loop with exponential backoff (1s, doubling, capped at 60s) without setting the project shutdown flag, rebuilding state, saving/restoring panes, or touching any input/PTY handle. That path is the whole mechanism.

This reverses an earlier decision in this change, which added a user-triggered `ReconnectTransport` operation alongside reboot. It was withdrawn deliberately, and the reasoning is worth keeping: a reconnect button asks the user to diagnose a transport state they cannot observe, and its presence next to `Reboot CLI` frames a dead connection as something to choose a remedy for. In practice a degraded transport is either already recovering on its own or the CLI is not reachable at all — and in the second case the request cannot be delivered, because it would travel over the very transport that is down. The control could therefore only ever help in the case that needed no help. Withdrawing it also removes the in-process `queue_transport_reconnect` / `immediate_reconnect` path whose sole purpose was to skip the automatic backoff for a requested reconnect.

Alternatives considered:

- **Keep using Reboot CLI for connection recovery:** rejected because it couples network health to process and agent lifecycle.
- **Have the server forcibly drop the socket without a protocol message:** rejected because it cannot distinguish an intentional recovery, correlate outcome, or provide authorization and UI feedback.

### 2. Run one persistent pane host per terminal pane under tmux supervision

Add a hidden `apas pane-host` mode. Each host owns exactly one PTY master, provider child, descendant process group, output sequence, volatile ring buffer, lifecycle, and resize state. The project CLI becomes a controller connected over a host-local Unix socket. New terminal input, resize, state, and output pass through this local protocol before following the existing CLI/server terminal protocol.

Each pane host runs in its own detached tmux session on the existing project-scoped tmux socket. Its session name includes a sanitized project ID, pane ID, and random runtime ID. The project CLI's own tmux session may be replaced or killed without killing these sibling sessions. Per-pane hosts isolate failures and allow newly created panes to use the newest installed host binary while older live panes continue using the compatible version that created them.

The host is an APAS protocol wrapper around the real PTY, not merely the provider directly in a tmux pane. It retains raw output chunks and their monotonically increasing sequence values, which pure tmux capture cannot provide.

Alternatives considered:

- **Plain tmux panes plus `capture-pane` and `send-keys`:** rejected because screen capture loses raw byte/sequence semantics, makes exact replay difficult, and lacks a secure adoption boundary.
- **One project-wide pane-host process:** rejected because one crash would terminate or strand every terminal and would pin all future panes to one old host version.
- **Pass PTY file descriptors across exec:** rejected because it does not preserve reader/control threads or work across a failed/replaced process without a separate broker; cross-process FD handoff is also substantially more platform-specific.
- **Run detached children without a supervisor:** rejected because discovery, liveness, cleanup, and daemon/project ownership would be less reliable than the existing tmux model.

### 3. Use a versioned, host-local adoption protocol with exclusive controller leases

Each runtime has `(project_id, pane_id, runtime_id, instance_id)` identity. The host creates a descriptor beneath a host-local APAS runtime directory, never the NFS-shared config directory or source repository. Directories are mode `0700`; the Unix socket and a random 256-bit bearer credential file are accessible only to the owning user. The credential is never placed in argv, environment sent to providers, logs, server messages, or `.apas`. Where available, the host also verifies peer UID from the Unix socket.

The length-delimited local protocol includes:

- `Create` with validated provider command, cwd/worktree, environment, initial instruction, and runtime identity;
- `Adopt` with protocol version, all identities, controller generation, credential, and desired replay position;
- `Adopted` with lifecycle, current sequence, oldest retained sequence, and truncation state;
- ordered `Output`, `Input`, `Resize`, `State`, `Ack`, `Detach`, and authenticated `Shutdown` messages.

Only one controller generation may hold the lease. A newer CLI can take over only through the expected reboot handoff token or after the old controller connection is demonstrably gone. Repeated adoption by the same generation is idempotent; a stale CLI cannot preempt a current controller.

The local descriptor contains no terminal content. `.apas` continues to persist the pane conversation ID and configuration, while the host-local registry maps the configured project/pane to an adoptable runtime. A project starting on a different machine therefore finds no runtime and follows fallback restoration.

Alternatives considered:

- **Rely only on filesystem permissions:** rejected because explicit project/pane/runtime binding and stale-controller rejection remain necessary even for processes under the same user.
- **Store the credential in `.apas`:** rejected because project directories and NFS configuration may be shared, backed up, or readable outside the host trust boundary.

### 4. Reuse stable instance IDs and server sequence deduplication for replay

The pane host, not the replaceable CLI, creates and owns the terminal `instance_id` and output sequence. Its volatile ring stores bounded raw chunks with their sequence values. On any CLI attachment it can replay the retained ring before live output. The server already rejects output at or below the current sequence for the same instance, so replaying retained chunks is safe after an uncertain disconnect and also repopulates state after a server restart.

The CLI reports the host's oldest/current sequence and truncation state during terminal reconciliation. If retained output rolled over, the server marks the presentation truncated and accepts the newest tail rather than pretending it has a complete stream. No raw output is written to the descriptor, `.apas`, SQLite, JSONL, or a spool file.

The host ring uses a configurable byte bound with a conservative default at least as large as the server terminal scrollback bound. Chunk boundaries and sequence values are retained; eviction removes whole oldest chunks.

Alternative considered:

- **Acknowledge and delete every chunk after WebSocket send:** rejected because a successful socket write does not prove application processing and would prevent reconstructing terminal state after server restart. Stable sequence deduplication is simpler and already implemented server-side.

### 5. Prepare updates before detach and complete reboot through a handoff marker

`RebootCli` first fetches/builds/validates/installs any update while the current project CLI and every pane remain attached. A preparation failure reports failure and leaves the process untouched. Immediately before replacement, the CLI:

1. persists the pane roster;
2. writes an atomic host-local handoff marker containing the request ID, project ID, controller generation, expected executable/version, terminal runtime IDs, and a short deadline (but no terminal credential or content);
3. asks each host to enter reboot-detached state under the handoff generation;
4. sends a `handoff` operation status and flushes the WebSocket;
5. executes the installed binary with the normal project arguments plus the handoff marker path.

The replacement reads and validates the marker before normal pane restoration. It adopts each matching runtime first, starts fallback restorations only for missing/incompatible runtimes, reconnects the server, reports the complete pane roster and terminal states, then emits success for the original request and deletes the marker. The server treats the transport gap as in-progress and applies a bounded timeout.

An `exec` failure leaves the old image alive but in a degraded handoff state; it clears host detach leases where possible, removes the marker, reports a local error, and exits with manual restart guidance. Provider processes remain protected by their host grace period.

Alternative considered:

- **Detach first, then build:** rejected because the common slow/failing phase would unnecessarily hide project control and consume the adoption grace period.

### 6. Distinguish intentional cleanup from unexpected controller loss

Normal CLI socket loss does not imply pane shutdown. The host starts a lease timer:

- an authenticated reboot detach receives a longer bounded handoff grace period;
- an unexpected disconnect receives a configurable adoption grace period;
- an authenticated pane close, project stop, or project deletion bypasses grace and shuts down immediately.

Shutdown terminates the provider's entire process group with graceful escalation, removes the tmux session, socket, credential, descriptor, and ring, and is idempotent. The daemon's project-stop path enumerates and kills the project CLI session plus every pane-host session/descriptor. This makes the existing project-deletion guarantee cover persistent runtimes even when no project CLI is responsive.

Startup reconciliation validates registry entries against live tmux sessions, host identity responses, and the current project roster. It removes stale descriptors but never adopts a runtime for a pane absent from `.apas`. An unadopted runtime self-terminates when its lease expires, providing defense if daemon cleanup is unavailable.

### 7. Capability detection is per operation and per live pane

Add protocol capabilities for lifecycle requests and persistent terminal hosting. The CLI advertises lifecycle-request support whenever the reboot path is supported. It advertises persistent hosting only after validating Unix sockets, secure host-local runtime storage, the pane-host subcommand, and tmux.

Preservation is also reported per terminal pane because a CLI upgraded in place may still own pre-feature PTYs that cannot be migrated live. Such panes remain usable but are marked `restart_required_on_cli_reboot`. Newly created/restored host-backed panes report their runtime ID and `live_adoptable`. After one fallback restart under the new architecture, future reboots can preserve them.

The server forwards lifecycle capabilities and statuses to currently authorized web clients. The web sends only `RebootCli`. Because an older web bundle may still send a retired lifecycle operation, the server ignores an operation it does not recognise rather than failing to decode the whole message and dropping the socket. Legacy reboot messages remain accepted during rollout and retain their current behavior and stronger warning.

### 8. Keep the toolbar concise and move lifecycle detail into one action menu

The existing top tab bar keeps its project-level lifecycle location, but replaces an ambiguous direct reboot confirmation with a compact action menu. `Reboot CLI` is the only entry, and shows the current preservation inventory (adoptable terminal count, non-adoptable terminal count, structured pane count) and explicit consequences. Progress is keyed by request ID and survives tab/project navigation in web state until success, failure, or timeout.

This does not return Bot, provider, or Role controls to terminal toolbars and does not add lifecycle controls to mobile in the first release.

## Risks / Trade-offs

- **[Old pane-host processes may run old code for a long time]** → Version the local protocol, keep a compatibility window, create new hosts with the latest binary, surface incompatible hosts as non-adoptable, and never kill a live provider merely to upgrade its wrapper.
- **[A detached autonomous agent can continue acting while no UI controls it]** → Use bounded leases, show detached status server-side, let the daemon/project-stop path kill hosts independently, and keep the default grace long enough for builds but not indefinite.
- **[Local credential or socket exposure could reveal terminal content/control]** → Use a host-local `0700` directory, `0600` credential/socket access, random credentials, peer-UID checks, exact identity binding, no secrets in argv/logs, and negative security tests.
- **[tmux is missing or fails]** → Probe before advertising support, keep the current CLI-owned PTY implementation as fallback, and warn accurately before reboot.
- **[Output ring truncation can begin mid-terminal state]** → Evict whole chunks, report truncation explicitly, preserve the newest bounded bytes, and retain the server's existing terminal-reset/truncation presentation.
- **[Two CLIs race to control one project after reboot]** → Combine existing daemon project claims with per-runtime controller generations and exclusive adoption; reject stale controllers without signaling the provider.
- **[Intentional stop races with reboot adoption]** → Make stop/delete create a host-local tombstone and revoke credentials before terminating sessions; adoption fails closed once cleanup begins.
- **[A host crash still interrupts the agent]** → Report a new instance and use provider-specific restart-and-resume; this design removes CLI replacement as a failure domain but cannot remove the pane host or machine failure domains.

## Migration Plan

1. Deploy server/shared protocol support that accepts legacy reboot messages, stores bounded lifecycle-operation state, and ignores unknown persistent-host metadata safely.
2. Deploy the web action menu capability-gated; old CLIs continue to show only the legacy reboot path with its destructive warning.
3. Ship the pane-host subcommand behind capability probing. Existing CLI-owned terminal panes are reported non-adoptable; no live PTY is migrated.
4. Create new terminal panes under persistent hosts. On the first CLI reboot after upgrade, old CLI-owned panes restart through existing continuation behavior and are recreated host-backed; later reboots adopt them live.
5. Extend daemon stop/reconciliation and project deletion checks before enabling the persistent-host capability by default.
6. Monitor operation latency, adoption success/fallback, detached-host count/age, output truncation, and cleanup failures without logging terminal content or credentials.

Rollback disables new host creation and hides the reboot enhancements while leaving already running hosts untouched until their panes close. If an older CLI cannot adopt them, an operator or newer daemon explicitly stops the project hosts, after which the older CLI restores provider sessions using existing behavior. Rollback documentation SHALL identify that this interrupts active terminal turns.
