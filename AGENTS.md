# APAS Agent Instructions

This file is generated from `claude.md` and mirrors deployment-only operational notes.
For the canonical team-mode contributor/agent runbook, read `CLAUDE.md`.

# APAS Project Notes

Deployment-only operational notes. The canonical team-mode contributor/agent
runbook is `CLAUDE.md`; keep architecture, local development, and team-role
workflow guidance there. `AGENTS.md` is generated from this file for
Codex-style agents that need deployment commands.

## Deployment

- **Server access:** `ssh root@apas.mpaxos.com`
- **Server binary location:** `/opt/apas/apas-server`
- **Service management:** `systemctl restart apas-server`

### Deploy commands:
```bash
# Build all Rust crates
cargo build --release

# Run server
scp target/release/apas-server root@apas.mpaxos.com:/tmp/apas-server
ssh root@apas.mpaxos.com "mv /tmp/apas-server /opt/apas/apas-server && chmod +x /opt/apas/apas-server && systemctl restart apas-server"

# Run web
# Before release, verify `npm ci && npm run audit:npm` and, in a separate clean
# tree, `pnpm install --frozen-lockfile && pnpm run audit:pnpm`.
web_backup_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
ssh root@apas.mpaxos.com "install -d -m 700 /opt/apas/backups/web-${web_backup_stamp} && tar -C /opt/apas --exclude='web/node_modules' -czf /opt/apas/backups/web-${web_backup_stamp}/web.tgz web"
rsync -avn --delete --exclude 'node_modules' --exclude '.next' packages/web/ root@apas.mpaxos.com:/opt/apas/web/
rsync -av --delete --exclude 'node_modules' --exclude '.next' packages/web/ root@apas.mpaxos.com:/opt/apas/web/
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm ci && NEXT_PUBLIC_WEB_UI_VERSION=${web_version} npm run build && systemctl restart apas-web"
for path in / /login /machines /share /admin /health; do curl -fsSL "https://apas.mpaxos.com${path}" >/dev/null; done
ssh root@apas.mpaxos.com "systemctl is-active apas-web apas-server && journalctl -u apas-web --since '5 minutes ago' -p err --no-pager -q"
```

For rollback, move the failed `/opt/apas/web` aside, extract the selected
`/opt/apas/backups/web-<timestamp>/web.tgz` under `/opt/apas`, run `npm ci` and
the versioned build, restart `apas-web`, and repeat the checks. This temporarily
restores the vulnerable dependency graph and requires a corrected redeployment.

## Web UI

- **URL:** https://apas.mpaxos.com

## Versioning Rule

Version is computed at build time using the format `YY.MM.N`.

- `YY.MM` is the current year/month.
- `N` is `git rev-list --count --since="<YYYY-MM-01 00:00:00>" HEAD`.
- Web version is resolved in `packages/web/next.config.ts`.
- CLI and server versions are resolved in Rust `build.rs` files.
- Do not manage `packages/web/.apas-version`; it is no longer used.

If building web in a directory without `.git` (for example `/opt/apas/web` on production), pass `NEXT_PUBLIC_WEB_UI_VERSION` explicitly using the deploy command above.
