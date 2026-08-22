## 1. Canonical Models and Policy Registry

- [x] 1.1 Add shared canonical DeepSeek Pro, Flash, and default-model identities and register `agent:claude:deepseek:deepseek-v4-flash` beside the existing Pro launch profile.
- [x] 1.2 Make launch-profile derivation resolve the legacy/direct `Provider::Deepseek` default to Pro while rejecting unregistered DeepSeek-looking model identifiers rather than relying on upstream fallback.
- [x] 1.3 Add shared and server policy tests covering independent Pro/Flash authorization, fresh defaults containing both profiles, explicit persisted allowlists remaining narrow, and unknown-model rejection.

## 2. Claude Runtime Routing and Switching

- [x] 2.1 Refactor DeepSeek environment construction to pin the selected Pro or Flash primary model while mapping the documented small/Haiku and Claude Code subagent variables to Flash through the same endpoint and API key.
- [x] 2.2 Preserve Pro as the generic DeepSeek default and add CLI tests for the complete Pro-primary and Flash-primary environment matrices, missing credentials, and unsupported model identifiers.
- [x] 2.3 Verify and test that switching a structured pane between Pro and Flash takes the process-respawn path, persists the canonical model, allocates a fresh provider session, and does not claim provider-context continuity.

## 3. Unified Web Model Choices

- [x] 3.1 Extend the centralized web provider catalog with DeepSeek Pro and Flash variants under one Claude/DeepSeek backend, keeping the existing generic DeepSeek value compatible with Pro.
- [x] 3.2 Update the retained Overview combined agent/backend control and policy catalog to consume the shared variants, display the active variant, and filter each variant through its exact launch profile without reviving structured-pane creation.
- [x] 3.3 Add the confirmed Pro/Flash switch interaction without adding a generic pane-card or toolbar model selector, including the fresh-context warning and visible-transcript behavior.
- [x] 3.4 Add web tests for labels, reverse mapping, Pro compatibility default, Flash selection, independent policy filtering, switch confirmation, and emitted provider/model payloads.

## 4. Compatibility and Verification

- [x] 4.1 Run focused shared, CLI, server, and web tests for DeepSeek routing and policy enforcement, then resolve regressions without weakening fail-closed behavior.
- [x] 4.2 Run full Rust and web test suites, TypeScript checking, linting, and strict OpenSpec validation; record any unrelated pre-existing failures separately.
- [x] 4.3 With a configured key and an explicit small budget, run redacted non-persistent Claude bridge smoke tests proving both Flash and Pro model attribution and successful responses.

## 5. Deployment and Live Policy Enablement

- [x] 5.1 When deployment is explicitly authorized, perform the required npm and pnpm audit/install verification, build release server/web/CLI artifacts, and back up the production web, database, and installed CLI before replacement.
- [x] 5.2 Deploy server, web, and CLI together, then explicitly add Flash to the production deployment default and DeepSeek-eligible project overrides without widening terminal-only overrides.
- [x] 5.3 Reconnect clients and verify service health, error logs, policy visibility, one shared DeepSeek credential status, Pro/Flash selection, model attribution, and rollback readiness.

## 6. DeepSeek Terminal Pane Creation

- [x] 6.1 Register `terminal:claude:deepseek:deepseek-v4-pro` and `terminal:claude:deepseek:deepseek-v4-flash` launch profiles beside the structured-pane variants, with shared tests for independent derivation and authorization.
- [x] 6.2 Thread the pane model through terminal spawn and restore so a Claude terminal carrying a DeepSeek model launches with the DeepSeek environment overrides and fails closed on a missing API key.
- [x] 6.3 Offer DeepSeek Pro and Flash under the Claude terminal entry in the desktop new-tab menu and the mobile create-pane sheet, filtered through each variant's own launch profile.
- [x] 6.4 Update web, CLI, server, and mobile tests for the new profiles, creation paths, and policy filtering; run the full Rust and web suites plus typecheck and lint.
