## Context

See `proposal.md` for motivation and the local-folder delta specification for the behavior contract.

All general creation entry points use `packages/web/src/components/CreateInstanceModal.tsx`. Its current `createProjectInstance` store action sends a clone request; the daemon's `DaemonState::create_instance` creates a destination, clones, changes branches, removes failed clones, and starts owner-created projects. None of those side effects is suitable for adopting an existing folder.

The sidebar and mobile project lists are session-based. Merely registering a directory does not create a session or a sidebar row. `/machines` already displays daemon project inventory and provides explicit Start controls. The directory registration flow must make that distinction visible rather than start saved agents as a workaround.

Shared-cluster provisioning deliberately confines guests to newly allocated clone destinations and preserves requester ownership through a separate provisioning flow. Machine visibility is not permission to import the owner's arbitrary existing files.

`project.rs::get_or_create_project` is not a safe wrapper for this operation: it deletes malformed `.apas` files to regenerate identity and suppresses registry failures. The lower-level registry writer is locked and fallible, but intentionally replaces conflicting ID/path associations. The registry can be shared across hosts through the user's configuration directory, so a local adoption must not silently retarget another registered project.

Daemon heartbeats update only in-memory machine inventory. The existing web Start gate additionally requires a durable hosting placement and canonical project policy. Register-only adoption therefore needs server finalization before it can truthfully offer a usable Start control.

## Goals / Non-Goals

**Goals:** reuse APAS project metadata, registry, machine inventory, and lifecycle controls; add one non-destructive daemon operation; preserve clone compatibility; provide complete local-registration feedback on desktop and mobile.

**Non-goals:** browser directory uploads, a remote filesystem browser, creating missing directories, changing Git configuration, starting projects automatically, granting guest filesystem access, changing canonical ownership, adding durable provisioning jobs, or repairing unrelated clone-flow defects.

## Decisions

### 1. Add a distinct local-registration protocol

Add `RegisterLocalProject` requests to the web/server and server/daemon protocols, with target machine, path, and a required request ID on the web request. Add `LocalProjectRegistered` results carrying that request ID and either the confirmed project ID/canonical path/name or an actionable error. Derive the responding machine from the authenticated daemon connection, not from a supplied actor field.

Advertise `local_project_registration_v1` from supporting daemons. Project a default-false `localProjectRegistrationAvailable` web capability from authenticated machine ownership, online state, and daemon support, following the existing machine-capability projection pattern. The server remains authoritative and rejects missing capability rather than dispatching an unknown command to an older daemon.

Update the generated web/mobile JSON schemas and `packages/protocol/src/generated.ts` through the existing `crates/shared/examples/export_mobile_schema.rs` pipeline. The schema validators consume these artifacts; changing Rust enums and handwritten web types alone is not a complete protocol change.

Bind in-flight local requests to their authenticated requesting user and target machine. All operation-specific failures, including pre-dispatch permission, path-shape, unsupported-version, and offline errors, return a correlated local result. A result from a different machine or for an unknown request cannot complete the operation. Connection loss clears or marks affected local operations unconfirmed; it must not leave an indefinite spinner or automatically replay writes. A manual retry is safe because the directory identity is reused.

**Alternative rejected:** overloading `clone_url` or `base_path` with a local directory. Those fields describe a clone source and destination parent and would retain destructive clone semantics or allow source confusion. Existing clone wire messages stay unchanged.

### 2. Authorize local adoption only for the machine owner

In `ws_web.rs`, derive machine ownership from the authenticated daemon registration, require an active requesting account with that owner ID, and verify any selected cluster context against it. Do not treat `sharedProvisioningAvailable`, cluster membership, or a project role as local-path authority. Recheck routing ownership before accepting the completion. Return no filesystem detail to rejected callers.

Local registration does not create a guest provisioning record or reserve a fresh identity over existing metadata. After a successful authenticated daemon result, acquire the existing project-operation guard and call `Database::authorize_project_registration(project_id, machine_owner)`. This rechecks account status and project lifecycle, creates a fresh canonical record only when absent, preserves existing ownership, verifies existing-project access, and records direct hosting placement. Resolve the effective project policy from the resulting placements without modifying overrides. Confirm the machine inventory contains the registration and only then acknowledge success to the web. Do not enqueue `StartProjectCli` or create a session.

If canonical finalization or policy resolution fails, return the correlated error and keep the directory and APAS identity intact for a safe retry. Local inventory may already contain that registration; it must not be mistaken for authorized completion. An unauthorized foreign project identity, suspended project, or deletion in progress remains denied. Reuse idempotent canonical registration rather than a clone transaction whose cancellation deletes a checkout.

**Alternative rejected:** allowing every shared-machine user to adopt arbitrary paths. Existing sharing has a managed clone root, not directory-level grants for pre-existing owner files.

### 3. Use a checked registration-only filesystem path

Add a focused daemon helper near the existing project-registration code rather than a branch inside `create_instance`. Run filesystem work off the async runtime. Its sequence is:

1. Accept an absolute path or expand a leading `~/` against the daemon user's home. Reject other relative forms; do not invoke a shell or expand arbitrary variables.
2. Canonicalize the existing directory and reject missing paths, regular files, or inaccessible required metadata. Resolve symlink aliases before comparing registrations.
3. Read the existing registry through its fallible, locked primitives. Check both canonical-path and project-ID associations; do not treat a registry read error as an empty registry. Conflict validation and mutation must share the existing registry lock. A repeat of the same identity/path is successful; a mismatch must not silently move another registration. Preserve the existing replacement policy for unrelated callers when extracting a shared primitive or adding an explicit conflict policy.
4. For existing `.apas`, reject symlinked or non-regular metadata, then strictly load the identity/name without rewriting, migrating, renaming, or resetting its content. Invalid metadata is an error. If metadata is absent, reuse a valid registered identity/name for that canonical path when present; otherwise use the zero-pane constructor. Serialize/recheck new metadata creation so concurrent requests do not overwrite a newly created identity or follow a dangling metadata symlink.
5. Persist the canonical registration through the existing checked registry writer. Git remote discovery is optional descriptive metadata, not a precondition; non-Git directories and repositories without an origin are valid. Propagate registry failures rather than acknowledging a best-effort write.
6. Refresh normal daemon inventory and return success only after registration is confirmed. Do not invoke `start_project`, recreate panes, or enter clone cleanup on any failure.

Reuse the strict parsing/registration approach already present in `restore_project_from_resume`, not its best-effort error handling or runtime restoration. Extract the necessary checked primitives in `project.rs` without adding a second registry format or compatibility aliases. Current native registration does not edit `.gitignore`; retain that behavior. If new APAS metadata is written before local persistence or server finalization fails, retain it for retry; never delete the user's directory as compensation.

**Alternative rejected:** shelling out to plain `apas` on an arbitrary path. It hides persistence errors and couples the request to CLI launch/bootstrap side effects instead of a correlated daemon operation.

### 4. Extend the existing form and operation feedback

Keep one creation modal. General New project opens a named, keyboard-accessible source-choice group using the existing pressed-button convention. Repository-group New instance remains clone-specific so a fixed repo identity cannot leak into an unrelated folder request. General button labels on Sidebar, SidebarRail, mobile home, and `/machines` become source-neutral.

Clone mode retains its existing derived name/branch, URL normalization, cluster filtering, and Create & start behavior. Local mode contains machine selection, a host-qualified path field, an in-place/registration-only explanation, and a Register folder action. It filters to owned machines with local support, retains explicit shared-cluster context rather than silently switching accounts, and explains why an unsupported target cannot register folders. Clone-only values and validation never enter a local request.

Use source-discriminated operation state with request ID, machine ID, display name/path, and pending/completed/failed state where needed for local completion. Generalize the current pending-instance consumers rather than render local paths as Git remotes or create a second unrelated feedback convention. Migrate all affected store and machines-page consumers; keep clone behavior unchanged. Scope results to the matching request and machine.

Keep the local form open through completion: retain entered values on send failure or registration error, prevent duplicate submission while pending, and show the confirmed canonical directory on success. Provide a View on Machines action that selects the owning cluster and identifies the target machine/project. Add narrowly scoped navigation context if needed so a remembered shared-cluster selection cannot hide the result. The project remains stopped until the user uses its existing Start control. Refresh machine inventory after success, but do not fabricate sessions, change the active workspace, or bypass authorized attachment.

**Alternative rejected:** only showing the existing three-second "created and starting" toast. It misstates the result and leaves a sessionless project undiscoverable from the creation entry point.

## Risks / Trade-offs

- **Existing project startup could restore agents** → registration-only completion with explicit Start controls; preserve any already-running runtime.
- **Alias paths, repeated clicks, or partial writes could split identity** → canonical-path and ID checks, serialized/rechecked metadata creation, and retry using retained APAS metadata.
- **A registration helper may silently mutate files or swallow errors** → inspect and reuse lower-level metadata/registry primitives; require durable confirmation before success.
- **Machine-local permissions vary** → let the daemon report concrete filesystem failures; do not use browser/server filesystem guesses or shell interpolation.
- **Registered folders lack sessions** → completion links to the owner machine inventory rather than broadening session-based sidebar models.
- **Shared users cannot import existing host folders** → intentionally retain the existing shared clone flow; directory-scoped guest grants are outside this change.
- **Rolling upgrades expose mixed capabilities** → default unsupported capability to false and enforce it server-side.

## Migration Plan

Implement and verify the protocol, daemon operation, server authorization, and web flow as one change. Update the canonical runbook after behavioral smoke proof. No database migration or provider restart is required for the feature itself.

When deployment is separately requested, roll out server first, web second, then gracefully update/reconnect daemons. Existing web clients continue cloning; new web clients offer local registration only for upgraded owner machines. Rollback hides/refuses local registration when capability projection disappears. Existing `.apas` files and normal registry entries remain usable by the CLI; do not remove user directories or their metadata during rollback.
