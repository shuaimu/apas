//! Match shared-daemon threads to the loopback MCP listener owned by their TUI.
//!
//! Codex 0.159+ can write every pane's rollout in one unrelated daemon. A
//! shared cwd or daemon PID is not ownership evidence. The thread's live
//! `codex_tui` MCP origin identifies its supplying TUI; never call that server's
//! tools, and refuse multiple user threads belonging to the same TUI.

use super::read_codex_rollout_meta;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::{client, Message, WebSocket};
use uuid::Uuid;

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

/// Batch only panes whose own process tree did not expose a rollout. One
/// daemon inventory serves all of them, instead of repeating it per pane.
pub(crate) fn find_rollouts(
    home: &Path,
    panes: &HashMap<i32, PathBuf>,
) -> Result<HashMap<i32, PathBuf>> {
    if panes.is_empty() {
        return Ok(HashMap::new());
    }
    let ports = listener_owners(Path::new("/proc"), panes)?;
    if ports.is_empty() {
        return Ok(HashMap::new());
    }
    let socket = home.join(".codex/app-server-control/app-server-control.sock");
    let metadata = match fs::metadata(&socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
        bail!("Codex control socket is not owned by this user");
    }
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(DISCOVERY_TIMEOUT))?;
    stream.set_write_timeout(Some(DISCOVERY_TIMEOUT))?;
    let (mut websocket, _) = client("ws://localhost/", stream)
        .map_err(|_| anyhow::anyhow!("Codex control socket handshake failed"))?;
    let result = query_rollouts(home, panes, &ports, &mut websocket, deadline);
    let _ = websocket.close(None);
    result
}

/// `/proc/net/tcp` names the socket inode; `/proc/<pid>/fd` proves which
/// requested provider process group owns it. A matching port alone is not proof.
fn listener_owners(proc_root: &Path, panes: &HashMap<i32, PathBuf>) -> Result<HashMap<u16, i32>> {
    let mut sockets = HashMap::new();
    for entry in fs::read_dir(proc_root)?.filter_map(|entry| entry.ok()) {
        if entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        let path = entry.path();
        let Ok(stat) = fs::read_to_string(path.join("stat")) else {
            continue;
        };
        let Some(fields) = stat.rsplit_once(')').map(|(_, fields)| fields) else {
            continue;
        };
        let Some(group) = fields
            .split_whitespace()
            .nth(2)
            .and_then(|value| value.parse::<i32>().ok())
        else {
            continue;
        };
        if !panes.contains_key(&group) {
            continue;
        }
        let Ok(fds) = fs::read_dir(path.join("fd")) else {
            continue;
        };
        for fd in fds.filter_map(|fd| fd.ok()) {
            let Ok(target) = fs::read_link(fd.path()) else {
                continue;
            };
            let Some(inode) = target
                .to_str()
                .and_then(|target| target.strip_prefix("socket:["))
                .and_then(|target| target.strip_suffix(']'))
                .and_then(|target| target.parse::<u64>().ok())
            else {
                continue;
            };
            sockets
                .entry(inode)
                .and_modify(|owner| {
                    if *owner != Some(group) {
                        *owner = None;
                    }
                })
                .or_insert(Some(group));
        }
    }
    let mut ports = HashMap::new();
    for line in fs::read_to_string(proc_root.join("net/tcp"))?
        .lines()
        .skip(1)
    {
        let mut fields = line.split_whitespace();
        let Some(address) = fields.nth(1) else {
            continue;
        };
        if fields.nth(1) != Some("0A") {
            continue;
        } // LISTEN
        let Some(inode) = fields.nth(5).and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let Some(Some(group)) = sockets.get(&inode) else {
            continue;
        };
        let Some(port) = address
            .strip_prefix("0100007F:")
            .and_then(|port| u16::from_str_radix(port, 16).ok())
        else {
            continue;
        };
        ports.insert(port, *group);
    }
    Ok(ports)
}

struct Rpc<'a> {
    socket: &'a mut WebSocket<UnixStream>,
    deadline: Instant,
    id: u64,
}

impl Rpc<'_> {
    fn remaining(&self) -> Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .context("Codex transcript discovery timed out")
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let remaining = self.remaining()?;
        self.socket.get_mut().set_write_timeout(Some(remaining))?;
        self.socket.send(Message::Text(
            json!({"id": self.id, "method": method, "params": params}).to_string(),
        ))?;
        loop {
            let remaining = self.remaining()?;
            self.socket.get_mut().set_read_timeout(Some(remaining))?;
            let message = self.socket.read()?;
            let Message::Text(text) = message else {
                continue;
            };
            let mut response: Value = serde_json::from_str(&text)?;
            if response.get("id").and_then(Value::as_u64) != Some(self.id) {
                continue;
            }
            if response.get("error").is_some() {
                bail!("Codex {method} metadata request failed");
            }
            return response
                .as_object_mut()
                .and_then(|response| response.remove("result"))
                .context("Codex metadata response has no result");
        }
    }
}

/// The daemon belongs to the person: their own Codex tabs run their turns in
/// it. `initialize` is not a private handshake there. Codex 0.160.0
/// (`codex-rs/app-server/src/request_processors/initialize_processor.rs`)
/// treats every client whose `clientInfo.name` is not in
/// `NON_ORIGINATING_CLIENT_NAMES` (`codex_app_server_daemon`, `codex-backend`)
/// as an originating client and changes three process-wide things for it:
///
/// - `USER_AGENT_SUFFIX` becomes `"<name>; <version>"`, so the User-Agent of
///   every later model request from the daemon, the person's own typed turns
///   included, names this client. Under the old name `apas_transcript` the
///   isolated harness saw a tab's next turn sent as `… (apas_transcript;
///   0.0.1)` instead of `… (codex-tui; 0.160.0)`, until another client
///   initialized.
/// - `set_default_originator(name)`, the `originator` header and User-Agent
///   prefix of those requests, taken by the first originating client: after a
///   daemon restart a locator poll can be the first client to arrive.
/// - gateway login: the first originating client decides whether automatic
///   login is allowed.
///
/// `codex_app_server_daemon` is the name Codex's own daemon tooling uses for
/// its read-only probe of this socket (`app-server-daemon/src/client.rs`,
/// `CLIENT_NAME`), and the one Codex keeps out of all three. Codex still
/// records each connection's `clientInfo` for its own analytics and tracing,
/// per connection, so the title and version say honestly that this is APAS.
/// The version never reaches a User-Agent under this name, and Codex parses
/// versions only from the server's own `userAgent`, not from a client's.
///
/// No `capabilities`, because the name alone is not enough: an
/// `explicitGatewayOauth` capability changes the daemon's login handling from
/// any client, whatever its name. Locating needs none of them:
/// `thread/loaded/list`, `thread/read` and `mcpServerStatus/list` are not
/// experimental (no `#[experimental]` in
/// `app-server-protocol/src/protocol/common.rs`, and all three are in the
/// non-experimental 0.166.0 schema), and the fields read here (`cwd`, `path`,
/// `name`, `runtimeStatus`, `httpOrigin`) are not experimental fields.
///
/// The allowlist is an internal Codex constant, not a documented contract.
/// Checked against the 0.160.0 source and a 0.166.0 daemon on 2026-11-14;
/// re-check both files whenever the Codex version changes.
fn initialize_params() -> Value {
    json!({
        "clientInfo": {
            "name": "codex_app_server_daemon",
            "title": "APAS transcript locator",
            "version": env!("APAS_VERSION")
        }
    })
}

fn query_rollouts(
    home: &Path,
    panes: &HashMap<i32, PathBuf>,
    ports: &HashMap<u16, i32>,
    socket: &mut WebSocket<UnixStream>,
    deadline: Instant,
) -> Result<HashMap<i32, PathBuf>> {
    let mut rpc = Rpc {
        socket,
        deadline,
        id: 0,
    };
    rpc.call("initialize", initialize_params())?;
    rpc.socket
        .send(Message::Text(json!({"method": "initialized"}).to_string()))?;
    let sessions_root = home.join(".codex/sessions");
    let mut found = HashMap::new();
    let mut ambiguous = HashSet::new();
    let mut cursor = Value::Null;
    loop {
        let page = rpc.call(
            "thread/loaded/list",
            json!({"limit": 100, "cursor": cursor}),
        )?;
        let ids = page
            .get("data")
            .and_then(Value::as_array)
            .context("Codex thread list has no data")?;
        for id in ids {
            let Some(id) = id.as_str().and_then(|id| Uuid::parse_str(id).ok()) else {
                continue;
            };
            let thread = rpc.call(
                "thread/read",
                json!({"threadId": id, "includeTurns": false}),
            )?;
            let thread = &thread["thread"];
            let Some(cwd) = thread.get("cwd").and_then(Value::as_str) else {
                continue;
            };
            if !panes.values().any(|expected| expected == Path::new(cwd)) {
                continue;
            }
            let Some(path) = thread
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
            else {
                continue;
            };
            if !path.starts_with(&sessions_root) {
                continue;
            }
            let Some(meta) = read_codex_rollout_meta(&path) else {
                continue;
            };
            if meta.session_id != Some(id) || meta.is_subagent() || meta.cwd != cwd {
                continue;
            }
            let status = rpc.call(
                "mcpServerStatus/list",
                json!({
                    "threadId": id, "serverName": "codex_tui", "detail": "toolsAndAuthOnly"
                }),
            )?;
            let Some(servers) = status.get("data").and_then(Value::as_array) else {
                continue;
            };
            for server in servers {
                if server.get("name").and_then(Value::as_str) != Some("codex_tui")
                    || server.get("runtimeStatus").and_then(Value::as_str) != Some("connected")
                {
                    continue;
                }
                let Some(port) = server
                    .get("httpOrigin")
                    .and_then(Value::as_str)
                    .and_then(|origin| origin.strip_prefix("http://127.0.0.1:"))
                    .and_then(|port| port.parse::<u16>().ok())
                else {
                    continue;
                };
                let Some(group) = ports.get(&port).filter(|group| {
                    panes
                        .get(group)
                        .is_some_and(|expected| expected == Path::new(cwd))
                }) else {
                    continue;
                };
                if found
                    .insert(*group, path.clone())
                    .is_some_and(|previous| previous != path)
                {
                    ambiguous.insert(*group);
                }
            }
        }
        let next = page.get("nextCursor").cloned().unwrap_or(Value::Null);
        if next.is_null() {
            break;
        }
        if next == cursor {
            bail!("Codex thread pagination did not advance");
        }
        cursor = next;
    }
    found.retain(|group, _| !ambiguous.contains(group));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(home: &Path, cwd: &str, port: u16, subagent: bool) -> Value {
        let id = Uuid::new_v4();
        let path = home.join(".codex/sessions").join(format!("{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            json!({
                "type": "session_meta",
                "payload": {"id": id, "cwd": cwd, "source": if subagent {
                    json!({"subagent": {"spawn": {}}})
                } else { json!("vscode") }}
            })
            .to_string(),
        )
        .unwrap();
        json!({
            "thread": {"id": id, "path": path, "cwd": cwd},
            "status": {"data": [{
                "name": "codex_tui", "runtimeStatus": "connected",
                "httpOrigin": format!("http://127.0.0.1:{port}")
            }]}
        })
    }

    fn discover(
        home: &Path,
        threads: Vec<Value>,
        panes: &HashMap<i32, PathBuf>,
        ports: &HashMap<u16, i32>,
    ) -> HashMap<i32, PathBuf> {
        discover_recording(home, threads, panes, ports).0
    }

    /// Also returns every frame the locator sent, in order.
    fn discover_recording(
        home: &Path,
        threads: Vec<Value>,
        panes: &HashMap<i32, PathBuf>,
        ports: &HashMap<u16, i32>,
    ) -> (HashMap<i32, PathBuf>, Vec<Value>) {
        let (server, client_stream) = UnixStream::pair().unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let mut sent = Vec::new();
            let mut socket = tokio_tungstenite::tungstenite::accept(server).unwrap();
            while let Ok(Message::Text(text)) = socket.read() {
                let request: Value = serde_json::from_str(&text).unwrap();
                sent.push(request.clone());
                let method = request["method"].as_str().unwrap();
                if method == "initialized" {
                    continue;
                }
                let result = match method {
                    "initialize" => json!({}),
                    "thread/loaded/list" => json!({
                        "data": threads.iter().map(|thread| &thread["thread"]["id"]).collect::<Vec<_>>(),
                        "nextCursor": null
                    }),
                    "thread/read" | "mcpServerStatus/list" => {
                        let thread = threads
                            .iter()
                            .find(|thread| thread["thread"]["id"] == request["params"]["threadId"])
                            .unwrap();
                        if method == "thread/read" {
                            json!({"thread": thread["thread"]})
                        } else {
                            thread["status"].clone()
                        }
                    }
                    _ => panic!("transcript discovery must not mutate provider state: {method}"),
                };
                socket
                    .send(Message::Text(
                        json!({
                            "id": request["id"], "result": result
                        })
                        .to_string(),
                    ))
                    .unwrap();
            }
            sent
        });
        let (mut socket, _) = client("ws://localhost/", client_stream).unwrap();
        let result = query_rollouts(
            home,
            panes,
            ports,
            &mut socket,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        socket.close(None).unwrap();
        drop(socket);
        (result, server.join().unwrap())
    }

    /// Any other name changes the person's own Codex daemon (see
    /// `initialize_params`), and so can a capability (`explicitGatewayOauth`
    /// is daemon-wide and sticky); locating needs none. Re-check Codex's
    /// `NON_ORIGINATING_CLIENT_NAMES` before changing this.
    #[test]
    fn initializes_as_codex_own_non_originating_daemon_client() {
        let home = tempfile::tempdir().unwrap();
        let owned = thread(home.path(), "/repo", 3000, false);
        let (found, sent) = discover_recording(
            home.path(),
            vec![owned],
            &HashMap::from([(100, PathBuf::from("/repo"))]),
            &HashMap::from([(3000, 100)]),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            sent[0],
            json!({
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "codex_app_server_daemon",
                        "title": "APAS transcript locator",
                        "version": env!("APAS_VERSION")
                    }
                }
            })
        );
        assert_eq!(sent[1], json!({"method": "initialized"}));
        let methods: Vec<&str> = sent[2..]
            .iter()
            .map(|frame| frame["method"].as_str().unwrap())
            .collect();
        assert_eq!(
            methods,
            ["thread/loaded/list", "thread/read", "mcpServerStatus/list"]
        );
    }

    /// Runs the real locator against an isolated Codex daemon, never the
    /// person's own: `APAS_CODEX_DAEMON_CHECK_HOME` is a private home whose
    /// `.codex/app-server-control/app-server-control.sock` must resolve to
    /// exactly `APAS_CODEX_DAEMON_CHECK_SOCKET`, checked before connecting.
    /// `APAS_CODEX_DAEMON_CHECK_PANE` is `<provider pgid>=<cwd>` and
    /// `APAS_CODEX_DAEMON_CHECK_THREAD` the conversation it must be bound to.
    /// Used by `~/.cache/apas-codex-check/bin/daemon-11-locator-identity.sh`.
    #[test]
    #[ignore = "needs an isolated Codex daemon; see the doc comment"]
    fn locates_a_tab_through_an_isolated_daemon() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        let home = PathBuf::from(var("APAS_CODEX_DAEMON_CHECK_HOME"));
        let expected_socket = PathBuf::from(var("APAS_CODEX_DAEMON_CHECK_SOCKET"));
        let real_home = std::env::var_os("HOME").map(PathBuf::from);
        assert!(
            real_home
                .is_none_or(|real| fs::canonicalize(&real).ok() != fs::canonicalize(&home).ok()),
            "refusing the person's own home"
        );
        let socket =
            fs::canonicalize(home.join(".codex/app-server-control/app-server-control.sock"))
                .expect("the private daemon's socket does not resolve");
        assert_eq!(
            socket, expected_socket,
            "refusing a socket other than the private one"
        );
        let (group, cwd) = var("APAS_CODEX_DAEMON_CHECK_PANE")
            .split_once('=')
            .map(|(group, cwd)| (group.parse::<i32>().unwrap(), PathBuf::from(cwd)))
            .unwrap();
        let found = find_rollouts(&home, &HashMap::from([(group, cwd)])).unwrap();
        println!("found: {found:?}");
        let path = found.get(&group).expect("the tab was not bound");
        let meta = read_codex_rollout_meta(path).unwrap();
        assert_eq!(
            meta.session_id.map(|id| id.to_string()),
            Some(var("APAS_CODEX_DAEMON_CHECK_THREAD"))
        );
    }

    #[test]
    fn shared_daemon_matches_owned_listener_not_cwd_or_subagents() {
        let home = tempfile::tempdir().unwrap();
        let owned = thread(home.path(), "/repo", 3000, false);
        let sibling = thread(home.path(), "/repo", 3001, false);
        let expected = HashMap::from([
            (
                100,
                PathBuf::from(owned["thread"]["path"].as_str().unwrap()),
            ),
            (
                200,
                PathBuf::from(sibling["thread"]["path"].as_str().unwrap()),
            ),
        ]);
        let mut wrong_id = thread(home.path(), "/repo", 3000, false);
        wrong_id["thread"]["id"] = json!(Uuid::new_v4());
        let found = discover(
            home.path(),
            vec![
                owned,
                sibling,
                thread(home.path(), "/repo", 3002, false),
                thread(home.path(), "/repo", 3000, true),
                thread(home.path(), "/other", 3000, false),
                wrong_id,
            ],
            &HashMap::from([(100, PathBuf::from("/repo")), (200, PathBuf::from("/repo"))]),
            &HashMap::from([(3000, 100), (3001, 200)]),
        );
        assert_eq!(found, expected);
    }

    #[test]
    fn shared_daemon_refuses_ambiguous_or_disconnected_threads() {
        let home = tempfile::tempdir().unwrap();
        let mut disconnected = thread(home.path(), "/repo", 3001, false);
        disconnected["status"]["data"][0]["runtimeStatus"] = json!("disconnected");
        let found = discover(
            home.path(),
            vec![
                thread(home.path(), "/repo", 3000, false),
                thread(home.path(), "/repo", 3000, false),
                disconnected,
            ],
            &HashMap::from([(100, PathBuf::from("/repo")), (200, PathBuf::from("/repo"))]),
            &HashMap::from([(3000, 100), (3001, 200)]),
        );
        assert!(found.is_empty());
    }

    #[test]
    fn only_owned_loopback_listeners_identify_a_tui() {
        let proc_root = tempfile::tempdir().unwrap();
        fs::create_dir_all(proc_root.path().join("net")).unwrap();
        let mut tcp = String::from("header\n");
        for (pid, group, inode, address, state) in [
            (100, 100, 10, "0100007F:0BB8", "0A"),
            (101, 100, 11, "0100007F:0BB9", "01"),
            (102, 100, 12, "00000000:0BBA", "0A"),
            (200, 200, 20, "0100007F:0BBB", "0A"),
        ] {
            let dir = proc_root.path().join(pid.to_string());
            fs::create_dir_all(dir.join("fd")).unwrap();
            fs::write(
                dir.join("stat"),
                format!("{pid} (codex ) tui) S 1 {group} 0"),
            )
            .unwrap();
            std::os::unix::fs::symlink(format!("socket:[{inode}]"), dir.join("fd/4")).unwrap();
            tcp.push_str(&format!(
                "0: {address} 00000000:0000 {state} 0:0 0:0 0 3000 0 {inode}\n"
            ));
        }
        fs::write(proc_root.path().join("net/tcp"), tcp).unwrap();
        assert_eq!(
            listener_owners(
                proc_root.path(),
                &HashMap::from([(100, PathBuf::from("/repo"))])
            )
            .unwrap(),
            HashMap::from([(3000, 100)])
        );
    }
}
