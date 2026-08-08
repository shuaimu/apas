# MiniMax and GLM provider retirement

MiniMax and GLM are no longer supported APAS providers or Claude-compatible
backends. This is a breaking runtime change: upgraded components decode legacy
provider values and configuration messages for compatibility, but never expose
them in a supported catalog, build their environment, or launch a process.

## Upgrade behavior

- Historical panes remain in `.apas` and keep their labels, messages,
  worktrees, and other metadata. They are reported as unsupported and remain
  stopped; users can inspect history and delete eligible panes.
- An upgraded project host gracefully interrupts a running retired pane,
  removes its input path, and terminates its child without stopping supported
  sibling panes.
- MiniMax/GLM keys and URLs in an older `config.toml` are ignored. They are not
  read for runtime use, transmitted, logged, cached, or copied when the config
  is rewritten. Removing them from the file is optional hygiene.
- Server startup transactionally removes retired launch-profile keys from the
  cluster default and non-null project overrides. A retired-only override
  becomes explicit `[]`, which permits no launches until an administrator
  changes it. Only changed rows receive new, cluster-monotonic versions.
- Stale configuration mutations and launch requests receive an unsupported
  provider error (or are safely discarded for credential-bearing tombstone
  messages). No request falls back to Claude or another provider.

## Rollout

Use a coordinated server → project-host/daemon → web rollout:

1. Back up `/opt/apas/data/apas.db`, then deploy the server. Confirm migrations
   completed and that stale web launch/config requests are rejected rather
   than routed.
2. Deploy the static CLI to project hosts and allow daemons to upgrade. Confirm
   every historical retired pane is stopped and supported sibling panes remain
   healthy.
3. Deploy the web UI. Confirm add-pane, team, Machines, usage, and cluster
   policy surfaces contain only supported providers, while historical panes
   display `Unsupported provider` with read-only history.

Do not deploy the new web before server enforcement is live. The web is a
convenience boundary; the server and project host are the security/runtime
boundaries.

## Verification

Run these read-only checks on the server after startup:

```bash
sqlite3 /opt/apas/data/apas.db \
  "SELECT id, version, allowed_launch_profiles FROM cluster_settings;"

sqlite3 /opt/apas/data/apas.db \
  "SELECT project_id, version, allowed_launch_profiles FROM project_policy_overrides WHERE allowed_launch_profiles IS NOT NULL ORDER BY project_id;"

sqlite3 /opt/apas/data/apas.db \
  "SELECT 'cluster', id FROM cluster_settings WHERE lower(allowed_launch_profiles) GLOB '*minimax*' OR lower(allowed_launch_profiles) GLOB '*glm*' UNION ALL SELECT 'project', project_id FROM project_policy_overrides WHERE allowed_launch_profiles IS NOT NULL AND (lower(allowed_launch_profiles) GLOB '*minimax*' OR lower(allowed_launch_profiles) GLOB '*glm*');"
```

The last query must return no rows. Also check:

```bash
journalctl -u apas-server --since '30 minutes ago' | rg 'removed retired provider profiles|unsupported provider'
ssh <project-host> 'apas --version; pgrep -af "apas daemon"'
```

In the cluster-admin Machines view, compare every reported daemon version with
the release version. In the project list/sidebar, check connected CLI versions
and restart any long-running project still reporting an older build. Finally,
attempt one stale MiniMax or GLM add/resume request in staging: it must return
an explicit unsupported error, remain connected, and create no child process.

Compatibility expectations during the staged rollout:

- Old web/CLI → new server: legacy values decode, retired configuration data is
  not echoed, and launch requests are rejected before host routing.
- New CLI → old server: the host independently rejects retired launch, resume,
  reboot, switch, and team messages before spawning.

## Coordinated rollback

Rollback all three runtime layers together: web first, then project-host CLI
and daemons, then server. An older web with a new server remains safe but may
show obsolete controls; an older project host reintroduces the ability to
launch retired providers and therefore must not run behind a rolled-back
server unintentionally.

The policy cleanup is intentionally not reversed automatically. If the old
release must support these providers again, restore the database backup or
explicitly re-add the desired profile keys after every runtime component has
been rolled back. Restoring policy does not restore credentials removed from a
rewritten local config; recover those only from the operator's secret store.
Before ending rollback, issue a staging launch and verify the exact intended
provider starts—never rely on fallback behavior.
