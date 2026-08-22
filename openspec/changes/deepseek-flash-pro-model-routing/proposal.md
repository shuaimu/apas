## Why

APAS currently treats Claude-over-DeepSeek as one hard-coded `deepseek-v4-pro` launch choice, so users cannot choose the substantially cheaper Flash model or use DeepSeek's recommended Pro-primary/Flash-subagent routing. DeepSeek now supports both V4 Pro and V4 Flash through the same Anthropic-compatible endpoint and API key, so APAS should expose them as models of one backend rather than duplicate credential profiles.

## What Changes

- Keep one machine-level Claude/DeepSeek backend configuration and API key.
- Add policy-controlled launch profiles for `deepseek-v4-pro` and `deepseek-v4-flash` under that backend.
- Add matching `terminal:claude:deepseek:*` launch profiles so new terminal panes can select DeepSeek as the Claude backend without reviving structured agent panes.
- Preserve Pro as the default primary model for existing and newly selected generic DeepSeek panes.
- Route Claude Code's small/Haiku and subagent work to Flash while the pane's selected primary model remains independently selectable.
- Present Pro and Flash as nested model choices under one Claude/DeepSeek backend in the new-tab menu, the mobile create-pane sheet, the retained structured-pane agent control, and the policy catalog.
- Restart the structured provider process with a fresh provider session when switching an existing pane between Flash and Pro, while retaining the visible APAS transcript.
- Preserve existing Pro panes and fail closed when either model-specific launch profile is disallowed.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-support`: Support DeepSeek V4 Pro and V4 Flash through one Claude runtime/backend configuration with Flash routing for small and subagent work.
- `pane-model-selection`: Permit the existing combined agent/backend control to select a DeepSeek model variant without restoring a generic standalone model selector.
- `project-policy-governance`: Govern DeepSeek Pro and Flash as independent launch capabilities while keeping them under one backend.

## Impact

- Shared launch-profile registry and policy validation/defaults.
- CLI DeepSeek environment construction, terminal pane spawn/restore model plumbing, pane switching, persistence, and compatibility tests.
- Web provider/model options, new-tab menu, mobile create-pane sheet, retained Overview controls, and policy editor labels.
- Production deployment and applicable project/default policy rows must explicitly permit Flash before it becomes selectable, including the new terminal profiles.
- No new credential, endpoint, external dependency, or wire field is required; existing provider/model fields carry the model choice.
