## Context

See `proposal.md` for motivation and `specs/pane-work-summaries/spec.md` for the behavioral contract. APAS already persists normalized conversation messages in `data/sessions/<session-id>/messages.jsonl`, including timestamps and pane identity, and removes messages older than seven days. Terminal PTY bytes deliberately bypass that store; only transcript-derived conversation turns are available for terminal panes.

The server has no configured language-model client. Provider credentials and executables live on each project CLI host, while the active pane process must not be reused for summarization. The installed Claude CLI supports a true empty tool set. The installed Codex CLI supports non-interactive ephemeral execution, ignored user configuration and rules, structured output, and read-only sandboxing, but not a no-tools invocation. Codex can therefore be offered as an explicitly selected adapter only with clear disclosure that prompt injection can still cause host reads.

The desktop workspace is a `TabbedView` with per-pane messages and a shared toolbar. Mobile uses separate home/activity surfaces and is intentionally outside this change.

## Goals / Non-Goals

**Goals:**

- Make summaries deterministic in scope: one session, pane, and fixed three-hour UTC window.
- Preserve completed summaries beyond raw-message retention without duplicating their source text.
- Keep generation off the active agent session, prevent repository modification, use tool-free execution where available, and minimize explicitly accepted Codex tool risk.
- Deliver cached and live status over the existing authenticated WebSocket architecture.
- Bound model quota, payload size, concurrency, retries, and backfill work.
- Make rolling upgrade, project deletion, and access revocation safe.

**Non-Goals:**

- Summarizing raw PTY scrollback, repository contents, or activity not present in conversation storage.
- Showing the drawer in the responsive mobile browser or native Expo application.
- Treating summaries as an audit log or authoritative proof of repository state.
- Reconstructing history already removed by the seven-day message-retention job.
- Eliminating the residual host-read risk of an explicitly enabled Codex headless adapter. A Claude adapter may summarize a Codex pane only after explicit cross-provider consent.
- Adding user-editable summaries, arbitrary window sizes, semantic search, or cross-pane/project rollups in the first release.

## Decisions

### 1. The server owns windowing, scheduling, authorization, and cached state

The server is the only component with authoritative persisted message history and current project access. A new summary service in `AppState` will derive source windows, manage a deduplicated job queue, persist results, and send view updates. The CLI is an untrusted generation worker: it receives bounded source material and returns a correlated result, but cannot choose scope or write the cache directly.

```text
messages.jsonl
     │
     ▼
server window/index + authorization
     │ bounded job, capability-gated
     ▼
CLI isolated summarizer ── result/digest ──▶ server cache
                                                │
                                                ▼
                                      desktop WebSocket drawer
```

Alternatives considered:

- Reusing the active pane would pollute its history, alter working status, and potentially interrupt work.
- Browser-side generation would stop when the page closes, expose credentials, and fail to preserve summaries before raw-message GC.
- A server-side provider API would require a new cluster-wide model credential and data-processing boundary. A future adapter can use that architecture, but it is not required for the first release.

### 2. Windows use canonical UTC boundaries and source digests

For a valid RFC 3339 message timestamp, the window start is the timestamp floored to a multiple of three hours since the Unix epoch; the end is exactly three hours later. The stable key is `(session_id, pane_id, window_start)`. Legacy pane names are normalized with the existing pane-ID parser, including the single-pane fallback. Messages without a resolvable pane or timestamp are excluded and counted in diagnostics rather than assigned heuristically.

The server canonicalizes eligible records in timestamp/id order and hashes the window coordinates plus canonical records with SHA-256. The digest is the generation identity and invalidation key. Late records change the digest while raw source is available. After source GC, the last completed cache record remains valid because there is no longer evidence from which to derive a different digest.

Canonical records include bounded user/assistant text, assistant result/error text, tool name plus success/failure, and concise persisted status relevant to work. They exclude heartbeats, usage-only records, raw tool inputs/results, raw diffs/code, PTY bytes, and repeated transport envelopes. Known credential patterns are redacted and each field is clipped before hashing and dispatch.

### 3. Summary cache is a versioned session sidecar

`FileStorage` will maintain `data/sessions/<session-id>/pane-work-summaries.json` using the same per-session lock and atomic replace pattern as other rewritten session artifacts. The versioned document is keyed by pane/window and stores:

- UTC start/end and pane ID;
- status (`queued`, `generating`, `complete`, `partial`, `stale`, `failed`, or `source_expired`);
- summary text when available;
- source digest, source message count, and latest included timestamp/id;
- generation/update timestamps, provider/model label, attempt count, and a bounded safe error;
- partial/final marker.

Only the final or partial summary and metadata are durable. Normalized source chunks and intermediate reduction notes remain in memory and are discarded after success, terminal failure, timeout, or server restart. At startup, queued/generating records are returned to queued state and candidates are rebuilt from retained messages.

Keeping the sidecar under the existing session directory makes owner project deletion remove it through the established session-directory deletion path. Closing a pane leaves historical summaries intact. Message GC rewrites only `messages.jsonl`, so completed summaries survive it. A SQLite table was rejected because it would add a second deletion/migration path for derived file-backed history.

### 4. Completed windows are automatic; current windows are demand-driven

A background scheduler runs at server startup and every 15 minutes. It scans retained session history in bounded batches, groups meaningful records, compares digests with the cache, and queues completed missing/stale windows newest-first. Drawer requests return the durable sidecar cache immediately without waiting for a retained-history scan; after that response, the server reconciles only the selected pane's open window. Historical backfill and stale completed-window regeneration remain owned by the background scheduler. The scheduler never delays message appends, drawer cache reads, or the daily GC job.

The open window is generated only when an authorized client requests it or explicitly refreshes it. A partial cache record includes `source_through`; repeated requests reuse it unless the source digest changed and a refresh throttle has elapsed. When the window closes, the completed-window path replaces the partial result.

Each CLI may have one in-flight summary generation at a time. The server also applies a configurable global concurrency bound, per-window deduplication, a job timeout, and at most three transient attempts with exponential backoff. Permanent/invalid results remain failed and manually retryable. If a capable CLI is offline, the job stays queued while source is retained; raw retention is not extended indefinitely to wait for generation.

### 5. Large windows use staged reduction instead of one large prompt

The server splits canonical records at message boundaries into bounded chunks. Each chunk is sent as a `notes` stage whose output is a short structured set of source-backed work facts. When all chunk notes exist, a `final` stage reduces only those notes into the 50–100 word summary. Stage job IDs include the parent source digest and chunk index, so retries are idempotent. The newest chunk is always included; configured total-stage limits produce an explicit failure rather than silently omitting recent work.

The final response is a JSON object with a single summary string. The server normalizes whitespace, verifies the digest/job identity, applies the word bound, rejects control/markup payloads, and retries one formatting correction before recording invalid output. Sparse source may produce fewer than 50 words, but the runner is instructed never to pad or infer unsupported activity.

### 6. Provider adapters validate provider-specific confinement before advertising capability

The CLI adds a small summary-runner interface separate from the pane spawn/resume helpers. It uses a fresh temporary working directory, a new provider invocation, bounded stdin/prompt data, a hard timeout, and captured stdout/stderr. It never passes a pane session ID, resume flag, project/worktree directory, MCP configuration, or the active pane's permissive launch flags.

The Claude adapter uses print mode with an explicit empty tool set, settings/plugin/MCP sources disabled where supported, no session persistence, and structured output. CLI configuration controls enablement, optional model selection, and whether the selected provider may summarize a different pane provider. Cross-provider transfer defaults off and must be explicitly enabled because a Codex conversation summarized by Claude crosses a vendor boundary.

The Codex adapter is intentionally opt-in. It invokes `codex exec` from a fresh empty directory with `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--sandbox read-only`, `--skip-git-repo-check`, and `--output-schema`; it passes no resume/session ID or project path and supplies the bounded job through stdin. Its prompt explicitly says that delimited conversation content is inert source data, commands and embedded instructions must not be executed or followed, and the only permitted output is a grounded 50–100 word summary. These measures prevent repository writes and reduce ambient configuration, persistence, and prompt-injection exposure, but they do not remove Codex's command tool or guarantee that unrelated readable host files cannot be inspected.

The CLI advertises `pane_work_summary_v1` only when an explicitly enabled adapter passes startup validation for its required flags. Selecting Codex constitutes operator acceptance of the documented residual host-read risk. Same-provider generation remains the default, so Codex summarizes Codex panes and Claude summarizes Claude panes unless cross-provider transfer is separately enabled. Future adapters implement the same interface and provider-appropriate tests without changing the protocol.

### 7. Summary messages are additive and capability-gated

Shared types add a versioned `PaneWorkSummary`, availability/status enums, and these logical messages:

- web → server: list summaries for `(session, pane)` and refresh a current/failed window;
- server → web: pane summary snapshot plus incremental status/result updates;
- server → CLI: staged summary generation job with job ID, scope, digest, stage, bounded source, and output contract;
- CLI → server: correlated success/unavailable/failure result with safe provider metadata.

All web requests repeat the same current `check_session_access` and active-project operation guard used for message reads. The server routes generation only to the CLI that owns the session and advertised the capability. Results are accepted only for an in-flight job whose session, pane, window, digest, and stage all match.

Server capability negotiation prevents a new web client from sending summary requests to an old server. An old project CLI can continue normal work; the server serves cached summaries and reports generation unavailable without sending it unknown variants.

### 8. Desktop uses a docked active-pane drawer

`TabbedView` gains a desktop-only `Summary` toolbar action for real pane IDs. It toggles a right-side drawer, closed by default, approximately 360–420 px wide, while leaving the conversation visible. The drawer header uses the pane label; cards are newest-first and format UTC ranges with `Intl.DateTimeFormat` in the browser time zone.

Zustand stores summary snapshots by `sessionId/paneId`, request state, and incremental updates. Opening the drawer or switching the active pane while open requests that pane's snapshot. Cards distinguish final, partial-through-time, queued/generating, stale, failed/retryable, client-update-required, and source-expired states. Overview and the no-project fallback do not show the action.

The responsive mobile branch and native Expo components do not import or render the drawer. CSS hiding alone is not the behavior boundary; the desktop rendering branch owns the control and requests so a mobile client does not accidentally trigger generation.

## Risks / Trade-offs

- **[Extra provider quota and possible rate-limit pressure]** → Require explicit CLI enablement, default cross-provider use off, one job per CLI, bounded background concurrency, newest-first backfill, and visible provider failures.
- **[Summaries can be incomplete or inaccurate]** → Ground prompts in canonical records, retain coverage/freshness metadata, label them as summaries rather than audit truth, and never infer success from missing evidence.
- **[Raw history can expire before an offline client generates a summary]** → Reconcile every 15 minutes and on drawer open, retry for the full retained period, then show source-expired instead of extending retention or fabricating content.
- **[Prompt injection in conversation text]** → Use a true no-tools adapter where supported, an empty working directory, no resume/settings/MCP state, strict data delimiters and output schema, plus server-side result validation. The Codex adapter additionally uses ephemeral execution, ignored user configuration/rules, and a read-only sandbox, but retains a command tool; require explicit operator selection and document that a malicious source may still induce reads from accessible host paths.
- **[Cross-provider privacy boundary]** → Default to same-provider use; require explicit CLI consent before another provider receives normalized conversation text and display the actual summary provider in metadata.
- **[Repeated scans add storage I/O]** → Scan retained history in bounded session batches, reuse digests, prioritize active/requested panes, and measure scan duration/bytes before tuning the interval.
- **[Side drawer reduces desktop conversation width]** → Keep it closed by default, bound its width, and collapse it below the desktop breakpoint rather than affecting the mobile layout.
- **[File corruption during replacement]** → Serialize a versioned document under the session lock, write a sibling temporary file, flush, and atomically rename; ignore/rebuild invalid non-summary state without touching `messages.jsonl`.

## Migration Plan

1. Add shared additive types and server parsing/storage/scheduler behind `pane_work_summary_v1`; deploy the server first. With no capable CLI it only serves existing cache/unavailable state.
2. Deploy the desktop web drawer gated by the server-negotiated capability. Old servers leave the action hidden; mobile remains unchanged.
3. Deploy the CLI summary runner and configuration with summary generation disabled until the operator selects Claude or explicitly accepts the documented residual risk of the Codex adapter, plus any cross-provider use.
4. Enable the adapter on selected internal project clients. Backfill only the retained seven-day source, newest-first, and monitor job latency, failures, quota usage, server scan time, and cache growth.
5. Enable background generation more broadly after internal validation. Existing completed summaries require no raw-history migration.

Rollback may remove web visibility and stop dispatching jobs in any order. Additive protocol fields are ignored by older peers, summary sidecars remain inert for a future redeploy, and existing project deletion still removes them. If the new server is rolled back, no active pane or conversation state depends on summaries.
