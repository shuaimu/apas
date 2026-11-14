## Why

The owner's OMP tabs got no work summaries: the summarizer was Codex, and the rule against sending one provider's conversation to another refused every OMP tab, even one that runs on the same OpenAI/Codex login. The owner asked on 2026-11-14 to summarize with OMP itself, using whatever backend OMP is configured with, and accepted cross-provider summaries.

## What Changes

- A third summary adapter, `omp`, selected with `apas config set summary_adapter omp`. It runs OMP in print mode with OMP's own default model unless `summary_model` is set.
- Its isolation: no session written, no built-in tools, no extensions, skills, rules, context files, LSP or PTY, tool calls that need approval refused, a settings overlay that turns off MCP and custom-tool discovery, keywords, memory, the advisor and model fallback, escaped `@` and `^` mentions, its own process group killed on timeout and at the end. The CLI checks the flags at startup and leaves the summarizer off when OMP lacks one.
- The residual risks are printed when it is selected and documented: OMP plugin tools still load, a model served by the `opencode` or `cursor` provider is not used, and OMP's detached broker can outlive a job briefly.
- No change to the cross-provider rule; the owner enables it in their own configuration.

## Capabilities

### New Capabilities

### Modified Capabilities
- `pane-work-summaries`: OMP joins Claude and Codex as a summary adapter, with its own isolation and disclosed residual risks.

## Impact

- CLI only: `crates/client-cli/src/summary_runner.rs`, `config.rs`, `main.rs` and tests. A machine gains it at its next daemon reboot. No server, web or protocol change; the server stores the provider name as given.
