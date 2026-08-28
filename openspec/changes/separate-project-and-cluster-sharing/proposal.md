## Why

APAS currently lists project membership and cluster membership as separate concepts but requires both for every foreign runtime attachment. That makes an ordinary project share appear valid in the project list while failing with `Access denied`, and the rejected web selection can expose an unauthorized, policy-incorrect workspace shell.

## What Changes

- Define project sharing as a project-scoped grant: a project user can read and operate that project on runtime instances hosted by the project owner's own cluster without receiving machine discovery, project creation, or unrelated-project access.
- Keep cluster sharing as a separate machine-scoped grant: a cluster member can use only permitted machines and only with projects they own or belong to; cluster membership alone reveals no unrelated project content.
- Preserve the hosting cluster owner's trust boundary: a project share does not authorize runtime use on an instance hosted by a third-party cluster unless the user also has active permission for that host and machine.
- Make the project-access and cluster-access interfaces name their distinct effects and prerequisites.
- Make web attachment transactional: a rejected attachment does not become the selected workspace, render Overview, restore cached project content, or emit follow-on project messages.
- Fail closed while project policy is unknown so an Overview disabled by policy is never displayed during attachment.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `project-access-control`: Distinguish project-scoped runtime authorization from cluster-wide machine authorization, including the third-party-host boundary.
- `web-project-workspace`: Render a project workspace only after attachment succeeds and apply authoritative Overview policy without a fail-open loading state.

## Impact

- Server database authorization in `crates/server/src/db/mod.rs` and WebSocket attachment/control gates in `crates/server/src/routes/ws_web.rs`.
- Project sharing and cluster sharing copy in the desktop web interface, with corresponding component/store tests.
- Web session selection and attachment state in `packages/web/src/lib/store.ts` and workspace visibility in `TabbedView.tsx`.
- Authorization regression tests for owner-hosted shares, third-party-hosted projects, cluster-only members, revocation, and machine allowlists.
