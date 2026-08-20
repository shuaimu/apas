## Why

Nothing runs managed team mode. Across every project this deployment knows there
are **zero** managed panes and no role-tagged panes at all — it is not merely
switched off by policy, it is unused. Meanwhile it is the largest feature in the
CLI by surface area: roughly 3,500 lines of dedicated modules, 184 references
inside the pane runtime, an entire MCP server whose every tool exists to serve
it, eight web panels, and a policy field carried at three levels.

That cost is paid by everything else. Team mode is why terminal panes are forced
unmanaged, why the `agent:*` launch profiles cannot be removed from the
catalogue, and why the pane runtime carries a second notion of who owns a pane.
Every change to panes, policy or providers has to be reasoned about twice.

## What Changes

- **The four managed roles go**: Manager, Tech Lead, Developer and Reviewer,
  their built-in prompts, and the promotion of a pane to managed.
- **The team artefacts stop being read or written**: `project_goal.md`,
  `team-todo.md`, `.apas-team.jsonl`. Existing files are left on disk untouched;
  APAS simply no longer watches, parses or publishes them.
- **The delegation MCP server goes.** Every one of its tools exists to serve the
  team protocol.
- **The team surfaces go from the web**: the Manager goal bar, the Team TODO
  panel, the delegation board, the scratchpad ticker, suggested workers, the team
  setup card, the team-mode switch and the Tech-Lead autonomy toggles.
- **Team authorization and the `team_available` policy stop deciding anything.**
  The field remains on the wire and in the database, ignored, so older web and
  mobile builds keep parsing what they are sent — the same treatment
  `cluster_role` already has.
- **BREAKING**: a project that still holds managed panes will find them ordinary
  panes. Their conversations remain; nothing dispatches them.

## Capabilities

### Modified Capabilities

- `project-policy-governance`: team availability stops being enforced, and the
  policy no longer governs a managed team.

## Impact

- `crates/client-cli/src/`: `role.rs`, `team_todo.rs`, `manager.rs`,
  `scratchpad.rs`, `suggested_workers.rs` and `mcp.rs` are removed outright
  (~3,600 lines); `mode/dual_pane.rs` loses its team runtime.
- `crates/shared/src/messages.rs`: the team message variants and role spec.
- `crates/server/src/routes/`: team routing in `ws_cli` and `ws_web`, and the
  team launch authorization.
- `packages/web/src/components/overview/`: eight panels and their tests.
- Untouched deliberately: the "Start bot" deadloop, which is not team mode and
  is used by 13 panes; existing agent panes, which remain runnable; and plan
  review, which is a per-pane feature with no team dependency.
