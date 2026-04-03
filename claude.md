# APAS Project Notes

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
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm install && npm run build && systemctl restart apas-web"
```

## Web UI

- **URL:** http://apas.mpaxos.com

## Versioning Rule (Mandatory, No Exceptions)

For every git commit, recalculate `packages/web/.apas-version` immediately before committing.

- This applies to all commits (code, docs, refactor, hotfix, etc.).
- Do not reuse or carry forward the previous value.
- The commit is incomplete unless `packages/web/.apas-version` is included in that same commit.

- Format must be: `YY.MM.N`
- `YY.MM` = current year/month
- `N` = number of commits in the current month + 1 (the next commit index)

Use this command sequence:

```bash
month_start="$(date +%Y-%m-01) 00:00:00"
month_count="$(git rev-list --count --since="$month_start" --until="now" HEAD)"
next_index="$((month_count + 1))"
printf "%s.%s\n" "$(date +%y.%m)" "$next_index" > packages/web/.apas-version
```

Then stage and commit `packages/web/.apas-version` together with the rest of the changes.
