## Why

The web's new-project flow only offers GitHub cloning, so users cannot add an existing directory on an APAS machine without using the CLI. Existing work should be registerable from the web whether it is a GitHub checkout, another Git repository, or a directory without Git.

## What Changes

- Add **Existing folder** beside **Clone from GitHub** in the shared new-project flow, including its desktop and mobile entry points.
- Let the user select one of their own online APAS machines and enter an existing host-local directory path. This is not a browser file upload or a folder on the browser's computer unless that computer is the selected APAS machine.
- Register the directory in place through its daemon. Do not require a GitHub URL, clone files, initialize Git, change branches or remotes, or create default agent panes.
- Preserve an existing `.apas` identity and configuration; repeat requests for the same canonical directory reuse its registration instead of producing duplicates. Invalid metadata and conflicting registrations are errors, not reasons to overwrite existing data.
- Keep local-directory adoption restricted to the machine's owning account. Shared-cluster membership alone must not authorize importing arbitrary files from another account's machine; the existing shared GitHub-clone flow remains available under its current rules.
- Report path, permission, offline-machine, unsupported-daemon, and registration errors in the creation flow. Confirm both local registration and server project/hosting placement before success so the existing Start control is usable; starting remains a separate explicit operation.
- Include registered, never-started projects in desktop and mobile project lists as stopped entries. Keep session identity absent until a real session exists, and link these entries to their owner-scoped machine inventory for explicit Start.
- Keep GitHub cloning and its current ownership, policy, request-correlation, and lifecycle behavior unchanged.

## Capabilities

### New Capabilities

- `local-folder-project-creation`: Source selection, authorized registration of existing host-local directories, safe identity reuse, and observable completion/failure from the web.

### Modified Capabilities

None. Existing project-access, workspace, and lifecycle requirements remain in force; the new capability adds another registration source.

## Impact

- Web: shared project-creation form, source-neutral entry points, machine filtering, pending-operation state, operation feedback, and registered-project visibility in the desktop sidebar/rail and mobile All projects.
- Protocol/server: an explicit local-registration request/result, generated protocol artifacts, daemon capability advertisement, machine-owner authorization, authenticated result correlation, and canonical project/placement finalization without starting a runtime.
- CLI daemon: reuse project metadata and registry primitives behind a non-destructive existing-directory registration operation; report the project through the normal machine inventory.
- Verification: filesystem-preservation and identity-boundary regressions, authorization/protocol coverage, and browser smoke scenarios for all three directory types and the unchanged GitHub flow.
- Documentation: update the canonical runbook's web project-creation guidance during implementation. No provider integration, dependency upgrade, host configuration change, or deployment is part of this proposal.
