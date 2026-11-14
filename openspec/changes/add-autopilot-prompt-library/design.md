## Context

The start dialog's templates are built into the web (`lib/autopilotTemplates.ts`). `.apas` is owned by the CLI: every write takes the per-project lock in the shared config directory, re-reads the file and replaces only its own fields (`project_instances.rs`, `update_project_settings`); unknown fields are kept. Project-wide settings already travel web → server → every instance → `.apas`, and back as an echo (`UpdateProjectFlags`, `ProjectFlagsChanged`).

## Goals / Non-Goals

**Goals:** named prompts per project, one optional default, editable from the start dialog on desktop and phone, kept in `.apas`, the same for every viewer.

**Non-Goals:** sharing prompts between projects or accounts; versions or history of a prompt; storing them on the server; per-tab prompts; native mobile.

## Decisions

1. **Storage.** Two optional top-level `.apas` fields: `autopilot_prompts: [{name, prompt, updated_at}]` (RFC 3339) and `default_autopilot_prompt: <name> | absent`. Written only through the existing locked merge, so other machines' panes and unknown fields are kept. A malformed list is reported as unreadable and left as it is, never rewritten.
2. **Wire.** `WebToServer::UpdateAutopilotPrompts { session_id, request_id, change }` where `change` is `save {name, prompt, replace}` (replace must be true to overwrite an existing name), `delete {name}` or `set_default {name | null}`. The server checks runtime access (the same check a start uses), validates, and sends it to the project's instances. The primary instance's answer `AutopilotPromptsUpdated { request_id, outcome, reason? }` is relayed to the requesting connection only. Every instance reports its list with `CliToServer::ProjectAutopilotPrompts { session_id, prompts, default }` at registration, after each change and when its `.apas` list changes on disk (checked by the existing 5-second poller, sent only when its hash differs). The server caches the primary instance's report and sends `ServerToWeb::ProjectAutopilotPrompts` to viewers that negotiated the capability, on change and on attach.
3. **Several machines.** A change is sent to every instance that runs the project, like project flags. On a shared home they all write one `.apas` (the locked merge makes repeats harmless); separate checkouts each write their own. The list shown is the primary instance's. A machine that is not running the project gets the change only when someone saves again; this is stated in the runbook.
4. **Capabilities.** CLI `project_autopilot_prompts_v1` (echoed in `Registered`), viewer `project_autopilot_prompts_v1`. A primary instance without it: requests are refused with the older-machine text. Viewers without it get no new frame.
5. **Dialog.** The Template control gets two groups, "This project" (saved prompts, the default marked "(default)") and "Built in", then "My own text". Next to it: "Save to project…" (asks for a name, pre-filled with the selected saved prompt's name, says when it will replace one), and, when a saved prompt is selected, "Delete" (with a confirmation) and "Make default" / "Clear default". The dialog's starting text: the last ended autopilot's prompt, else the project default, else APAS worker. The existing rule that the control names an entry only while the field holds its exact text applies to saved prompts too. Pending requests show their outcome inline and are never resent.

## Risks / Trade-offs

- [`.apas` is usually git-ignored] → saved prompts stay on the machine(s) that hold that `.apas`; a fresh clone has none. Stated in the runbook.
- [Separate checkouts drift] → accepted; the primary's list is shown.
- [A 50 × 16 KB list in one frame] → about 800 KB at most, sent only on change and attach, never periodically.
