# APAS Project Notes

## Deployment

- **Server access:** `ssh root@apas.mpaxos.com`
- **Server binary location:** `/opt/apas/apas-server`
- **Service management:** `systemctl restart apas-server`

### Deploy commands:
```bash
scp target/release/apas-server root@apas.mpaxos.com:/tmp/apas-server
ssh root@apas.mpaxos.com "mv /tmp/apas-server /opt/apas/apas-server && chmod +x /opt/apas/apas-server && systemctl restart apas-server"
```

## Web UI

- **URL:** http://apas.mpaxos.com
