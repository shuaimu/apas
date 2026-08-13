## 1. Shared Catalog and Policy

- [x] 1.1 Add the OpenCode terminal tab type and `terminal:opencode:official:default` launch profile to the shared provider catalogs and web catalog mirror.
- [x] 1.2 Expose the allowed OpenCode terminal option in the desktop new-tab menu and mobile task-launch catalog while retaining effective-policy filtering.
- [x] 1.3 Regenerate protocol schemas/types and update catalog contract tests.

## 2. OpenCode Terminal Hosting

- [x] 2.1 Add OpenCode to the verified terminal-host allowlist and resolve launches through the configured `opencode_path`.
- [x] 2.2 Build provider-specific TUI arguments for automatic permission approval, `--prompt` on fresh launches, and `--continue` on restoration without replaying the initial instruction.
- [x] 2.3 Add PTY argument and host-boundary regression tests that preserve existing Claude/Codex behavior.

## 3. Conversation Recovery and Status

- [x] 3.1 Discover the newest OpenCode session whose recorded directory exactly matches the pane working directory and export it by native session ID.
- [x] 3.2 Parse completed user/assistant text while filtering synthetic, ignored, reasoning, tool, and attachment parts and preserving model/token metadata.
- [x] 3.3 Integrate OpenCode exports into the terminal transcript watcher and mark idle only on final non-tool-call completion.
- [x] 3.4 Add transcript fixtures covering directory scoping, partial assistant responses, internal parts, usage, and completion state.

## 4. Legacy Structured OpenCode Compatibility

- [x] 4.1 Update headless OpenCode arguments to use the supported JSON, automatic-permission, model, and continuation interfaces without passing the APAS UUID as an OpenCode session ID.
- [x] 4.2 Translate native OpenCode text, tool-use, final completion/usage, and error events into shared structured pane messages.
- [x] 4.3 Add regression tests proving tool-call steps do not clear working state and final steps preserve usage.

## 5. Mixed-Version Authorization

- [x] 5.1 Add and advertise a provider-specific OpenCode terminal CLI capability.
- [x] 5.2 Reject desktop OpenCode creation on an older connected project CLI with an actionable update-and-reconnect error.
- [x] 5.3 Reject mobile OpenCode task launch on an older connected project CLI without affecting compatible Claude/Codex launch.
- [x] 5.4 Add server authorization tests for capable and incapable project CLIs.

## 6. Documentation and Verification

- [x] 6.1 Document OpenCode installation/configuration expectations, session attribution ambiguity, policy opt-in behavior, and rolling deployment order.
- [x] 6.2 Verify formatting and generated contracts are stable.
- [x] 6.3 Run the complete shared, CLI, server, web, protocol, terminal-web, and native mobile test/typecheck suites.
- [x] 6.4 Build the production web bundle and validate the supported OpenCode CLI command shapes against a current official release.
