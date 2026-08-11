## Why

Long-running agent panes accumulate more conversation history than a user can quickly review, while APAS retains raw messages for only seven days. Users need a compact, durable way to understand what one agent attempted, completed, validated, and left blocked without rereading the entire conversation.

## What Changes

- Divide each pane's persisted conversation activity into fixed three-hour UTC windows and produce a 50–100 word work summary for every window containing meaningful activity.
- Generate summaries outside the active agent conversation through an isolated CLI-side summarizer so summarization cannot interrupt work, modify the repository, or pollute pane history. Providers with a true tool-disable mode use it; Codex headless generation is an explicit opt-in that uses read-only confinement and documents its residual host-read risk.
- Cache completed-window summaries with the session so they remain available after raw conversation messages age out, while invalidating and regenerating a summary when its retained source window changes.
- Add authenticated summary request, delivery, generation-job, and capability-negotiation protocol messages across web, server, and CLI.
- Add a desktop-only `Summary` action and docked side drawer scoped to the active pane, including current-window freshness, generation, unavailable, and failure states.
- Keep mobile browser/native experiences unchanged in the initial release, and do not persist or summarize raw terminal scrollback.
- Apply existing project-access authorization and project-deletion guarantees to summaries, and bound model input, generation concurrency, retry behavior, and output length.

## Capabilities

### New Capabilities

- `pane-work-summaries`: Defines per-pane three-hour work-summary generation, retention, authorization, protocol behavior, and the desktop summary drawer.

### Modified Capabilities

None.

## Impact

- Shared protocol types gain version-tolerant summary records, desktop requests/updates, and CLI generation jobs/results.
- The server gains windowing, source normalization/digesting, durable summary storage under each session, background finalization/backfill, authorization, and retry scheduling.
- The CLI gains a capability-gated, single-concurrency summarizer runner with no pane resume, bounded input, structured output validation, and provider-specific confinement. Claude uses an empty tool set. Codex uses an explicitly selected ephemeral `codex exec` invocation in a fresh directory with user configuration and rules ignored, a read-only sandbox, and a strict data-only prompt; operators are warned that this reduces but does not eliminate prompt-injection-driven host reads.
- The desktop Next.js pane layout and Zustand store gain summary state, loading/error handling, and a right-side drawer; mobile routes and components remain unchanged.
- Summary generation consumes additional provider quota and introduces rolling-deployment behavior when a project CLI does not yet advertise the summary capability.
