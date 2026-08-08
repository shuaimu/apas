## Mixed-Version Compatibility Verification

Performed locally on 2026-08-08 using pre-change commit `1fa65cf` as the old
release and the current worktree as the upgraded release. Both directions ran
against isolated localhost servers and temporary databases/project directories;
no production service was contacted or restarted.

### Old client against upgraded server

- Built the old CLI and server from `1fa65cf` in a detached temporary worktree.
- Connected the old CLI to the upgraded server and attached a legacy-protocol
  web client to its empty project session.
- Submitted a Claude-provider pane using model `MiniMax-M2.7`.
- Observed an explicit unsupported-provider error on the stale web connection.
- Requested the session list after the rejection to prove the WebSocket stayed
  usable, and confirmed the retired command was never routed to the old CLI.

Result: `STAGE1_PASS old-client/new-server explicit-rejection connection-alive`.

### Upgraded CLI against old server

- Connected the upgraded headless CLI to the old server from `1fa65cf` with an
  empty temporary project.
- Submitted a legacy Claude-provider pane using model `glm-5.1`; the old server
  accepted and routed the request.
- Observed the upgraded CLI return a legacy-compatible system output containing
  the explicit unsupported-provider rejection.
- Requested the connected CLI list after the rejection to prove the connection
  stayed usable.
- Confirmed the temporary `.apas` file still contained an empty pane list and
  that no retired-provider or supported-provider fallback process was spawned.

Result: `STAGE2_PASS new-cli/old-server explicit-rejection connection-alive no-fallback`.

All staged processes, databases, project directories, and the detached old-code
worktree were removed after verification.
