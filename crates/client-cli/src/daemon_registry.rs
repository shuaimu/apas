//! Cross-host daemon coordination.
//!
//! APAS runs **one daemon per host**. On a cluster whose `$HOME` is a shared
//! NFS mount (the zoo-00N setup this was written for), that intent was quietly
//! broken: `machine_id`, `daemon.pid` and `daemon.json` all lived under
//! `~/.config/apas/`, which every host sees, while every liveness check
//! resolved a pid through the *local* `/proc`. Shared state read with
//! host-local semantics produced three failures:
//!
//!   * all hosts registered under one `machine_id`, so the server's
//!     `machine_infos` map (keyed by that id) kept overwriting itself and the
//!     web Machines page showed one machine with a flickering hostname;
//!   * a daemon read a peer's pid and looked it up in its own `/proc`, so it
//!     either found an unrelated process and refused to start, or found
//!     nothing and took over state another host owned;
//!   * two hosts could each spawn a headless CLI for the *same* project,
//!     both writing the same `.apas` and worktrees over NFS.
//!
//! The split this module implements:
//!
//! **Host-local** ([`crate::config::Config::runtime_dir`]) — anything whose
//! meaning is bounded by one kernel: pid files, daemon liveness state. That
//! lives on tmpfs and is gone at reboot, which is correct for a pid.
//!
//! **NFS-shared** (here) — the things daemons genuinely need to tell each
//! other: who is alive, and who owns which project. NFS is used deliberately
//! as the coordination channel, not accidentally as pid storage.
//!
//! Liveness is by **heartbeat, not pid**. A pid is only interpretable on the
//! host that owns it, so a peer's record is considered live purely because it
//! was refreshed recently. Only a daemon's *own* record is ever cross-checked
//! against `/proc`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Namespace for deriving stable per-host machine ids. Arbitrary but fixed:
/// changing it renames every machine in the web UI.
const MACHINE_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

/// A daemon record older than this is treated as dead. Comfortably above the
/// heartbeat interval so a slow tick or a brief NFS stall doesn't evict a
/// healthy peer.
pub const STALE_AFTER_SECS: u64 = 90;

/// How often a running daemon should refresh its record and its claims.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 20;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// This host's name, normalized. Used both as the machine-id seed and as the
/// registry filename, so it must be stable.
pub fn hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|v| v.into_string().ok())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

/// Derive this host's machine id.
///
/// Deterministic from `user@hostname` rather than randomly generated and
/// persisted, which is what fixes the collision: the old code minted a random
/// UUID once and wrote it to the NFS-shared `config.toml`, so whichever host
/// ran first donated its identity to the whole cluster. Deriving it needs no
/// storage at all, is stable across reboots (unlike anything on tmpfs), and is
/// necessarily distinct per host.
///
/// Keyed on the user too, so two people sharing a cluster don't collide.
pub fn derive_machine_id() -> Uuid {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown-user".to_string());
    Uuid::new_v5(
        &MACHINE_ID_NAMESPACE,
        format!("{}@{}", user.trim(), hostname()).as_bytes(),
    )
}

/// One daemon's advertisement to its peers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRecord {
    pub hostname: String,
    pub machine_id: Uuid,
    /// Only meaningful on `hostname`; never resolve it locally for a peer.
    pub pid: u32,
    pub version: String,
    pub started_at: u64,
    /// Unix seconds; the only liveness signal that travels between hosts.
    pub heartbeat: u64,
}

impl DaemonRecord {
    pub fn is_stale(&self, now: u64) -> bool {
        now.saturating_sub(self.heartbeat) > STALE_AFTER_SECS
    }
}

/// Which host currently runs a given project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectClaim {
    pub project_id: String,
    pub project_path: String,
    pub hostname: String,
    pub machine_id: Uuid,
    pub pid: u32,
    pub claimed_at: u64,
    pub heartbeat: u64,
}

impl ProjectClaim {
    pub fn is_stale(&self, now: u64) -> bool {
        now.saturating_sub(self.heartbeat) > STALE_AFTER_SECS
    }
}

/// Outcome of trying to take a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// We own it — safe to spawn.
    Acquired,
    /// We already owned it; the claim was refreshed.
    AlreadyOurs,
    /// A live peer owns it. Do not spawn.
    HeldBy(DaemonRecordSummary),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonRecordSummary {
    pub hostname: String,
    pub age_secs: u64,
}

// ---------------------------------------------------------------------------
// Paths (all NFS-shared, under the existing config dir)
// ---------------------------------------------------------------------------

pub fn daemons_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("daemons")
}

pub fn claims_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("claims")
}

fn daemon_record_path(config_dir: &Path, host: &str) -> PathBuf {
    daemons_dir(config_dir).join(format!("{host}.json"))
}

fn claim_path(config_dir: &Path, project_id: &str) -> PathBuf {
    // Project ids are UUIDs from `.apas`, but sanitize anyway — this becomes a
    // filename and must not escape the directory.
    let safe: String = project_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    claims_dir(config_dir).join(format!("{safe}.json"))
}

/// Write JSON atomically: a peer reading mid-write over NFS must never see a
/// truncated record. Temp file in the same directory, then rename.
fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let body = serde_json::to_string_pretty(value)?;
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

// ---------------------------------------------------------------------------
// Peer registry
// ---------------------------------------------------------------------------

/// Publish (or refresh) this daemon's record. Called at startup and on every
/// heartbeat tick.
pub fn publish_self(config_dir: &Path, machine_id: Uuid, version: &str) -> Result<DaemonRecord> {
    let host = hostname();
    let path = daemon_record_path(config_dir, &host);
    let now = now_secs();
    // Preserve the original start time across heartbeats so uptime is real.
    let started_at = read_json::<DaemonRecord>(&path)
        .filter(|r| r.pid == std::process::id())
        .map(|r| r.started_at)
        .unwrap_or(now);

    let record = DaemonRecord {
        hostname: host,
        machine_id,
        pid: std::process::id(),
        version: version.to_string(),
        started_at,
        heartbeat: now,
    };
    write_atomic(&path, &record)?;
    Ok(record)
}

/// RAII cleanup for a daemon's shared-registry presence.
///
/// Withdrawal must not be tied to reaching the connected event loop. A daemon
/// that is still retrying its server connection when it gets SIGINT never gets
/// there, so an explicit call inside that loop leaves the record behind and
/// peers keep seeing a dead host until the staleness window expires. Dropping
/// covers every return path, matching how `DaemonStateGuard` already handles
/// the host-local pid file.
///
/// A SIGKILLed daemon still can't run this -- that case is exactly what the
/// heartbeat staleness check is for. The two layers are complementary: this
/// makes clean exits instant, staleness makes unclean ones eventually correct.
pub struct RegistrationGuard {
    config_dir: PathBuf,
}

impl RegistrationGuard {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }
}

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        release_all_own_claims(&self.config_dir);
        withdraw_self(&self.config_dir);
    }
}

/// Remove this host's record on clean shutdown, so peers see it leave
/// immediately instead of waiting out the staleness window.
pub fn withdraw_self(config_dir: &Path) {
    let path = daemon_record_path(config_dir, &hostname());
    if let Some(existing) = read_json::<DaemonRecord>(&path) {
        // Only withdraw our own entry — a restarted daemon on this host may
        // already have replaced it.
        if existing.pid != std::process::id() {
            return;
        }
    }
    let _ = std::fs::remove_file(path);
}

/// Every live daemon sharing this NFS config dir, excluding ourselves.
pub fn live_peers(config_dir: &Path) -> Vec<DaemonRecord> {
    let me = hostname();
    let now = now_secs();
    let Ok(entries) = std::fs::read_dir(daemons_dir(config_dir)) else {
        return Vec::new();
    };
    let mut peers: Vec<DaemonRecord> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| read_json::<DaemonRecord>(&e.path()))
        .filter(|r| r.hostname != me && !r.is_stale(now))
        .collect();
    peers.sort_by(|a, b| a.hostname.cmp(&b.hostname));
    peers
}

// ---------------------------------------------------------------------------
// Project claims
// ---------------------------------------------------------------------------

/// Try to take ownership of a project before spawning a headless CLI for it.
///
/// The pre-existing guard only checked the local `/proc` for a running
/// `apas --headless`, which cannot see a process on another host — so with a
/// shared NFS `projects.json`, two daemons would each conclude the project was
/// free and both spawn.
///
/// Ownership is decided by heartbeat: a claim from a peer that is still being
/// refreshed is respected; one that has gone quiet is taken over.
///
/// Two daemons racing on a never-before-claimed project can both win the
/// rename — NFS gives no cross-client atomicity here. That window is one
/// filesystem round-trip wide and self-heals on the next heartbeat, when the
/// loser sees a claim naming the other host and stands down. It is a large
/// improvement on "always both spawn", not a distributed lock.
pub fn claim_project(
    config_dir: &Path,
    project_id: &str,
    project_path: &str,
    machine_id: Uuid,
) -> Result<ClaimOutcome> {
    let path = claim_path(config_dir, project_id);
    let now = now_secs();
    let me = hostname();

    if let Some(existing) = read_json::<ProjectClaim>(&path) {
        if existing.hostname == me {
            let refreshed = ProjectClaim {
                heartbeat: now,
                ..existing
            };
            write_atomic(&path, &refreshed)?;
            return Ok(ClaimOutcome::AlreadyOurs);
        }
        if !existing.is_stale(now) {
            return Ok(ClaimOutcome::HeldBy(DaemonRecordSummary {
                hostname: existing.hostname,
                age_secs: now.saturating_sub(existing.heartbeat),
            }));
        }
        tracing::info!(
            project_id,
            previous_host = %existing.hostname,
            stale_for = now.saturating_sub(existing.heartbeat),
            "taking over a stale project claim"
        );
    }

    write_atomic(
        &path,
        &ProjectClaim {
            project_id: project_id.to_string(),
            project_path: project_path.to_string(),
            hostname: me,
            machine_id,
            pid: std::process::id(),
            claimed_at: now,
            heartbeat: now,
        },
    )?;
    Ok(ClaimOutcome::Acquired)
}

/// Refresh every claim this host owns. Call on the heartbeat tick, otherwise
/// our claims age out and a peer steals a project we are actively running.
pub fn refresh_own_claims(config_dir: &Path) {
    let me = hostname();
    let now = now_secs();
    let Ok(entries) = std::fs::read_dir(claims_dir(config_dir)) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        if let Some(claim) = read_json::<ProjectClaim>(&path) {
            if claim.hostname == me && claim.pid == std::process::id() {
                let _ = write_atomic(
                    &path,
                    &ProjectClaim {
                        heartbeat: now,
                        ..claim
                    },
                );
            }
        }
    }
}

/// Drop a claim we hold (project stopped, or daemon shutting down).
pub fn release_project(config_dir: &Path, project_id: &str) {
    let path = claim_path(config_dir, project_id);
    if let Some(claim) = read_json::<ProjectClaim>(&path) {
        if claim.hostname != hostname() {
            return; // never release someone else's claim
        }
    }
    let _ = std::fs::remove_file(path);
}

/// Release everything this host owns, on clean shutdown.
pub fn release_all_own_claims(config_dir: &Path) {
    let me = hostname();
    let Ok(entries) = std::fs::read_dir(claims_dir(config_dir)) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if let Some(claim) = read_json::<ProjectClaim>(&path) {
            if claim.hostname == me && claim.pid == std::process::id() {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn other_host_claim(id: &str, host: &str, heartbeat: u64) -> ProjectClaim {
        ProjectClaim {
            project_id: id.to_string(),
            project_path: "/proj".to_string(),
            hostname: host.to_string(),
            machine_id: Uuid::nil(),
            pid: 4242,
            claimed_at: heartbeat,
            heartbeat,
        }
    }

    #[test]
    fn machine_id_is_stable_and_host_specific() {
        // Stability matters because the server keys machines by this id: a
        // value that changed per run would create a new machine every boot.
        assert_eq!(derive_machine_id(), derive_machine_id());

        let a = Uuid::new_v5(&MACHINE_ID_NAMESPACE, b"shuai@zoo-002");
        let b = Uuid::new_v5(&MACHINE_ID_NAMESPACE, b"shuai@zoo-005");
        assert_ne!(a, b, "two hosts must not share a machine id");

        // Different users on one host are distinct too.
        let c = Uuid::new_v5(&MACHINE_ID_NAMESPACE, b"someone@zoo-002");
        assert_ne!(a, c);
    }

    #[test]
    fn claim_is_acquired_when_free_and_refreshed_when_ours() {
        let dir = TempDir::new().unwrap();
        let id = Uuid::new_v4();
        assert_eq!(
            claim_project(dir.path(), "p1", "/proj", id).unwrap(),
            ClaimOutcome::Acquired
        );
        assert_eq!(
            claim_project(dir.path(), "p1", "/proj", id).unwrap(),
            ClaimOutcome::AlreadyOurs
        );
    }

    #[test]
    fn live_peer_claim_blocks_a_second_host() {
        // The bug this prevents: shared projects.json + a /proc-only guard let
        // two hosts spawn the same project over NFS.
        let dir = TempDir::new().unwrap();
        let path = claims_dir(dir.path()).join("p1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(&path, &other_host_claim("p1", "zoo-002", now_secs())).unwrap();

        match claim_project(dir.path(), "p1", "/proj", Uuid::new_v4()).unwrap() {
            ClaimOutcome::HeldBy(peer) => assert_eq!(peer.hostname, "zoo-002"),
            other => panic!("expected the live peer to keep the project, got {other:?}"),
        }
    }

    #[test]
    fn stale_peer_claim_is_taken_over() {
        // A host that died must not strand its projects forever.
        let dir = TempDir::new().unwrap();
        let path = claims_dir(dir.path()).join("p1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let long_ago = now_secs().saturating_sub(STALE_AFTER_SECS + 30);
        write_atomic(&path, &other_host_claim("p1", "zoo-002", long_ago)).unwrap();

        assert_eq!(
            claim_project(dir.path(), "p1", "/proj", Uuid::new_v4()).unwrap(),
            ClaimOutcome::Acquired
        );
    }

    #[test]
    fn release_never_touches_another_hosts_claim() {
        let dir = TempDir::new().unwrap();
        let path = claims_dir(dir.path()).join("p1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(&path, &other_host_claim("p1", "zoo-002", now_secs())).unwrap();

        release_project(dir.path(), "p1");
        assert!(path.exists(), "a peer's claim must survive our release call");
    }

    #[test]
    fn peers_exclude_self_and_stale_records() {
        let dir = TempDir::new().unwrap();
        let now = now_secs();
        let mk = |host: &str, hb: u64| DaemonRecord {
            hostname: host.to_string(),
            machine_id: Uuid::nil(),
            pid: 1,
            version: "test".to_string(),
            started_at: hb,
            heartbeat: hb,
        };
        std::fs::create_dir_all(daemons_dir(dir.path())).unwrap();
        for (host, hb) in [
            ("zoo-002", now),
            ("zoo-004", now.saturating_sub(STALE_AFTER_SECS + 30)),
            (&hostname(), now),
        ] {
            write_atomic(&daemons_dir(dir.path()).join(format!("{host}.json")), &mk(host, hb))
                .unwrap();
        }

        let peers = live_peers(dir.path());
        let names: Vec<&str> = peers.iter().map(|p| p.hostname.as_str()).collect();
        assert_eq!(names, vec!["zoo-002"], "self and stale hosts must be excluded");
    }

    #[test]
    fn publish_then_withdraw_round_trips() {
        let dir = TempDir::new().unwrap();
        let id = derive_machine_id();
        let rec = publish_self(dir.path(), id, "1.2.3").unwrap();
        assert_eq!(rec.machine_id, id);
        assert_eq!(rec.hostname, hostname());

        // Our own record is excluded from peers by design.
        assert!(live_peers(dir.path()).is_empty());

        withdraw_self(dir.path());
        assert!(!daemons_dir(dir.path()).join(format!("{}.json", hostname())).exists());
    }

    #[test]
    fn guard_withdraws_on_drop_even_without_a_clean_loop_exit() {
        // Regression: withdrawal used to live inside the connected event loop,
        // so a daemon SIGINTed while still retrying its connection never
        // reached it and left its record behind for peers to see.
        let dir = TempDir::new().unwrap();
        let id = derive_machine_id();
        publish_self(dir.path(), id, "test").unwrap();
        claim_project(dir.path(), "p1", "/proj", id).unwrap();

        let record = daemons_dir(dir.path()).join(format!("{}.json", hostname()));
        assert!(record.exists());

        {
            let _guard = RegistrationGuard::new(dir.path().to_path_buf());
        } // dropped here, as it would be on any early return

        assert!(!record.exists(), "daemon record should be withdrawn on drop");
        assert!(
            !claims_dir(dir.path()).join("p1.json").exists(),
            "our claims should be released on drop"
        );
    }

    #[test]
    fn claim_filenames_cannot_escape_the_directory() {
        let dir = TempDir::new().unwrap();
        let p = claim_path(dir.path(), "../../etc/passwd");
        assert_eq!(p.parent().unwrap(), claims_dir(dir.path()));
        assert!(!p.to_string_lossy().contains(".."));
    }
}
