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
rsync -av --exclude 'node_modules' --exclude '.next' packages/web/ root@apas.mpaxos.com:/opt/apas/web/
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm install && NEXT_PUBLIC_WEB_UI_VERSION=${web_version} npm run build && systemctl restart apas-web"
```

## Web UI

- **URL:** http://apas.mpaxos.com

## Versioning Rule

Version is computed at build time using the format `YY.MM.N`.

- `YY.MM` is the current year/month.
- `N` is `git rev-list --count --since="<YYYY-MM-01 00:00:00>" HEAD`.
- Web version is resolved in `packages/web/next.config.ts`.
- CLI and server versions are resolved in Rust `build.rs` files.
- Do not manage `packages/web/.apas-version`; it is no longer used.

If building web in a directory without `.git` (for example `/opt/apas/web` on production), pass `NEXT_PUBLIC_WEB_UI_VERSION` explicitly using the deploy command above.
