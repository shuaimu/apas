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
