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
//! other: who is alive, and who runs which project. NFS is used deliberately
//! as the coordination channel, not accidentally as pid storage.
//!
//! Liveness is by **heartbeat, not pid**. A pid is only interpretable on the
//! host that owns it, so a peer's record is considered live purely because it
//! was refreshed recently. Only a daemon's *own* record is ever cross-checked
//! against `/proc`.
//!
//! **A project may run on several machines at once**, one instance per
//! machine, all attached to its one server session. Claims are therefore per
//! project *and machine* (`claims/<project>/<machine>.json`), and an instance
//! releases its claim when it stops. The project-wide `claims/<project>.json`
//! survives as a guard against daemons that predate instances: a capable
//! daemon running the project holds it marked `"instances": true`, which such
//! a daemon ignores, so it still sees a live peer and stays out. See
//! [`claim_project`].

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

/// Whether this host can run a project beside other machines' instances of
/// it, and so advertise `shared::PROJECT_INSTANCES_CAPABILITY`. Takes the raw
/// `daemon.machine_id` from `config.toml`; the daemon and every project
/// runtime on the host must decide this the same way, so both call this.
///
/// An instance is told apart by its machine id, and that setting lives in the
/// shared `config.toml`: every host on the same home would report the same
/// machine. So any non-blank value turns instances off, even one that fails to
/// parse and is ignored in favour of the derived id. Someone meant to override
/// this host's identity, and running each project alone, as every daemon did
/// before instances, is always safe.
pub fn project_instances_supported(configured_machine_id: Option<&str>) -> bool {
    configured_machine_id.is_none_or(|raw| raw.trim().is_empty())
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

/// Which host runs a project: either one machine's instance
/// (`claims/<project>/<machine>.json`) or the project-wide legacy guard
/// (`claims/<project>.json`). The guard keeps exactly the shape daemons
/// predating instances read and write, plus `instances`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectClaim {
    pub project_id: String,
    pub project_path: String,
    pub hostname: String,
    pub machine_id: Uuid,
    pub pid: u32,
    pub claimed_at: u64,
    pub heartbeat: u64,
    /// Written by daemons that run one instance per machine. On the guard it
    /// tells another such daemon that it may start its own instance beside
    /// the holder's. A daemon predating instances drops the field and sees an
    /// ordinary live peer, which is what keeps it out. A guard without it
    /// means its holder runs the project alone.
    #[serde(default, skip_serializing_if = "is_false")]
    pub instances: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ProjectClaim {
    pub fn is_stale(&self, now: u64) -> bool {
        now.saturating_sub(self.heartbeat) > STALE_AFTER_SECS
    }

    fn holder(&self, now: u64) -> InstanceHolder {
        InstanceHolder {
            machine_id: self.machine_id,
            hostname: self.hostname.clone(),
            claimed_at: self.claimed_at,
            age_secs: now.saturating_sub(self.heartbeat),
        }
    }
}

/// How a daemon claims the projects it runs. Chosen once per daemon, with
/// [`ClaimMode::for_configured_machine_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimMode {
    /// One instance per machine: a claim per (project, machine), plus the
    /// legacy guard marked `instances` while no other host holds it. Other
    /// capable daemons may then run their own instances, and older ones stay
    /// out. The daemon advertises `project_instances_v1`.
    Instances,
    /// The project-wide guard alone, unmarked, refusing while any other host
    /// runs the project: how every daemon behaved before instances. For a host
    /// whose `daemon.machine_id` is set, which shares its machine id with every
    /// other host on the same home and so cannot claim per machine.
    Exclusive,
}

impl ClaimMode {
    pub fn for_configured_machine_id(configured_machine_id: Option<&str>) -> Self {
        if project_instances_supported(configured_machine_id) {
            Self::Instances
        } else {
            Self::Exclusive
        }
    }
}

/// A machine running an instance of a project, as its claim says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceHolder {
    pub machine_id: Uuid,
    pub hostname: String,
    /// Unix seconds when the holder claimed the project: when its instance
    /// started, or, for a host seen only through the guard, when it took the
    /// guard. Orders starts for adopting untagged panes
    /// ([`LiveInstances::blocks_adoption`]).
    pub claimed_at: u64,
    /// Seconds since the holder last refreshed its claim.
    pub age_secs: u64,
}

/// Outcome of trying to take a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// We own it — safe to spawn.
    Acquired,
    /// We already owned it; the claim was refreshed.
    AlreadyOurs,
    /// A live claim rules out running it here. Do not spawn.
    HeldBy(ClaimRefusal),
}

/// Why a live claim keeps this daemon from starting a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRefusal {
    /// Another host holds the guard without the instances marker: a daemon
    /// predating instances, or one whose machine id is configured, runs the
    /// project and would not tolerate a second runtime beside it.
    ExclusiveHolder(InstanceHolder),
    /// This daemon runs projects alone ([`ClaimMode::Exclusive`]), and another
    /// host runs this one.
    RunningElsewhere(InstanceHolder),
}

impl ClaimRefusal {
    /// The refusal as the project's `last_error` and the server report it.
    pub fn reason(&self) -> String {
        match self {
            Self::ExclusiveHolder(holder) => format!(
                "{host} is running this project with a daemon that cannot share it with \
                 other machines; stop it there or reboot {host} to update it",
                host = holder.hostname
            ),
            Self::RunningElsewhere(holder) => format!(
                "{host} is running this project, and this host cannot run it beside another \
                 machine because config.toml sets daemon.machine_id",
                host = holder.hostname
            ),
        }
    }
}

/// Who else is running a project, as [`live_instance_machines`] reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveInstances {
    /// Other machines with a live instance, sorted by hostname. A capable host
    /// holding the guard counts even if its own instance claim is missing.
    pub machines: Vec<InstanceHolder>,
    /// Another host holds the guard without the instances marker, so it runs
    /// the project alone: a daemon predating instances, or one whose machine
    /// id is configured.
    pub exclusive_legacy: Option<InstanceHolder>,
    /// This machine's own live instance claim, which its daemon writes before
    /// it starts the instance. `None` for a run no daemon claimed, such as a
    /// standalone `apas --headless`.
    pub own: Option<InstanceHolder>,
}

impl LiveInstances {
    /// Whether any other machine runs the project.
    pub fn any(&self) -> bool {
        !self.machines.is_empty() || self.exclusive_legacy.is_some()
    }

    /// Whether this machine's instance, starting now, must leave the
    /// project's untagged panes unadopted.
    ///
    /// A host that runs the project alone always rules it out: it runs every
    /// untagged pane itself. Otherwise the earliest claimant adopts, so this
    /// machine is ruled out only by another machine whose live claim is older
    /// than its own, compared by `claimed_at` and then by machine id, which
    /// gives two machines claiming in the same second one answer between them.
    ///
    /// Order, not presence, decides because each daemon writes its claim
    /// before its instance reaches this check. Two machines starting the
    /// project together each saw the other's claim, and when presence alone
    /// ruled adoption out, neither adopted: the untagged tabs stayed on no
    /// machine, listed under a host that did not run them, until every
    /// instance stopped. Adopting beside a later claimant takes nothing it
    /// runs, since an instance with the capability never runs an untagged
    /// pane; and adoption happens under the project lock on a fresh read, so
    /// each pane is tagged by one of them only. A run with no claim of its own
    /// has no start to order, and any other live instance rules it out.
    pub fn blocks_adoption(&self) -> bool {
        if self.exclusive_legacy.is_some() {
            return true;
        }
        let Some(own) = &self.own else {
            return self.any();
        };
        self.machines
            .iter()
            .any(|other| (other.claimed_at, other.machine_id) < (own.claimed_at, own.machine_id))
    }
}

/// The host and process acting on the shared files: this one in production.
/// Tests play several hosts against one directory with it.
struct Actor {
    host: String,
    pid: u32,
}

impl Actor {
    fn current() -> Self {
        Self {
            host: hostname(),
            pid: std::process::id(),
        }
    }
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

/// A project id as a file name. Project ids are UUIDs from `.apas`, but
/// sanitize anyway — this becomes a filename and must not escape the
/// directory.
fn claim_file_stem(project_id: &str) -> String {
    project_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The project-wide legacy guard.
fn claim_path(config_dir: &Path, project_id: &str) -> PathBuf {
    claims_dir(config_dir).join(format!("{}.json", claim_file_stem(project_id)))
}

/// One file per machine running an instance. A directory, so daemons that
/// predate instances, which read only the `*.json` files directly under
/// `claims/`, never see them.
fn instance_claims_dir(config_dir: &Path, project_id: &str) -> PathBuf {
    claims_dir(config_dir).join(claim_file_stem(project_id))
}

fn instance_claim_path(config_dir: &Path, project_id: &str, machine_id: Uuid) -> PathBuf {
    instance_claims_dir(config_dir, project_id).join(format!("{machine_id}.json"))
}

/// Where [`write_atomic`] stages `path` for `actor`.
///
/// Every host writes into the same NFS directories and pids repeat across
/// hosts, so a pid alone let two hosts write one temp file and rename each
/// other's halves into place; naming the host makes the name unique. Hidden,
/// and never ending in `.json`, so no reader takes it for a record.
fn temp_path_for(path: &Path, actor: &Actor) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let host: String = actor
        .host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    path.with_file_name(format!(".{name}.{host}.{}.tmp", actor.pid))
}

/// Write JSON atomically: a peer reading mid-write over NFS must never see a
/// truncated record. Temp file in the same directory, then rename.
fn write_atomic<T: Serialize>(path: &Path, value: &T, actor: &Actor) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = temp_path_for(path, actor);
    let body = serde_json::to_string_pretty(value)?;
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err).with_context(|| format!("renaming into {}", path.display()));
    }
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
    let actor = Actor::current();
    let path = daemon_record_path(config_dir, &actor.host);
    let now = now_secs();
    // Preserve the original start time across heartbeats so uptime is real.
    let started_at = read_json::<DaemonRecord>(&path)
        .filter(|r| r.pid == actor.pid)
        .map(|r| r.started_at)
        .unwrap_or(now);

    let record = DaemonRecord {
        hostname: actor.host.clone(),
        machine_id,
        pid: actor.pid,
        version: version.to_string(),
        started_at,
        heartbeat: now,
    };
    write_atomic(&path, &record, &actor)?;
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

/// Claim a project for this machine before starting an instance of it.
///
/// The pre-existing guard only checked the local `/proc` for a running
/// `apas --headless`, which cannot see a process on another host — so with a
/// shared NFS `projects.json`, two daemons would each conclude the project was
/// free and both spawn.
///
/// Ownership is decided by heartbeat: a claim from a peer that is still being
/// refreshed is respected; one that has gone quiet is taken over. What a live
/// claim rules out depends on `mode`:
///
/// * **Instances.** Write this machine's own claim. Another machine's live
///   instance is no obstacle: this one becomes a further instance of the same
///   project. Only a live guard from another host *without* the instances
///   marker refuses it, because that holder runs the project alone. This
///   daemon also holds the guard, marked, unless another live host already
///   does, so a daemon predating instances keeps seeing a live peer.
/// * **Exclusive.** The behaviour before instances: hold the guard, unmarked,
///   and refuse while another host has any live claim — the guard, marked or
///   not, or an instance claim, which can be live while its host is between
///   releasing the guard and the next refresh retaking it.
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
    mode: ClaimMode,
) -> Result<ClaimOutcome> {
    claim_project_at(
        config_dir,
        project_id,
        project_path,
        machine_id,
        mode,
        &Actor::current(),
        now_secs(),
    )
}

fn claim_project_at(
    config_dir: &Path,
    project_id: &str,
    project_path: &str,
    machine_id: Uuid,
    mode: ClaimMode,
    actor: &Actor,
    now: u64,
) -> Result<ClaimOutcome> {
    let guard_path = claim_path(config_dir, project_id);
    let guard = read_json::<ProjectClaim>(&guard_path);
    let foreign_guard = guard
        .as_ref()
        .filter(|claim| claim.hostname != actor.host && !claim.is_stale(now));

    match mode {
        ClaimMode::Exclusive => {
            if let Some(holder) = foreign_guard {
                return Ok(ClaimOutcome::HeldBy(ClaimRefusal::RunningElsewhere(
                    holder.holder(now),
                )));
            }
            if guard
                .as_ref()
                .is_some_and(|claim| claim.hostname == actor.host)
            {
                // Take over the pid as well as the heartbeat: a guard written
                // by a *previous* daemon on this host still carries that
                // daemon's pid, and `release_all_own_claims` matches on pid.
                hold_guard(
                    &guard_path,
                    guard.as_ref(),
                    project_id,
                    project_path,
                    machine_id,
                    mode,
                    actor,
                    now,
                )?;
                return Ok(ClaimOutcome::AlreadyOurs);
            }
            if let Some(peer) = live_instance_claims(config_dir, project_id, now)
                .into_iter()
                .find(|claim| claim.hostname != actor.host)
            {
                return Ok(ClaimOutcome::HeldBy(ClaimRefusal::RunningElsewhere(
                    peer.holder(now),
                )));
            }
            if let Some(stale) = &guard {
                tracing::info!(
                    project_id,
                    previous_host = %stale.hostname,
                    stale_for = now.saturating_sub(stale.heartbeat),
                    "taking over a stale project claim"
                );
            }
            hold_guard(
                &guard_path,
                guard.as_ref(),
                project_id,
                project_path,
                machine_id,
                mode,
                actor,
                now,
            )?;
            Ok(ClaimOutcome::Acquired)
        }
        ClaimMode::Instances => {
            if let Some(holder) = foreign_guard.filter(|claim| !claim.instances) {
                return Ok(ClaimOutcome::HeldBy(ClaimRefusal::ExclusiveHolder(
                    holder.holder(now),
                )));
            }
            // A claim left here by a previous daemon on this host is ours, and
            // the write below takes over its pid: shutdown releases only the
            // claims naming the live daemon's pid.
            let own_path = instance_claim_path(config_dir, project_id, machine_id);
            let previous =
                read_json::<ProjectClaim>(&own_path).filter(|claim| claim.hostname == actor.host);
            write_instance_claim(
                &own_path,
                previous.as_ref(),
                project_id,
                project_path,
                machine_id,
                actor,
                now,
            )?;
            hold_guard(
                &guard_path,
                guard.as_ref(),
                project_id,
                project_path,
                machine_id,
                mode,
                actor,
                now,
            )?;
            Ok(if previous.is_some() {
                ClaimOutcome::AlreadyOurs
            } else {
                ClaimOutcome::Acquired
            })
        }
    }
}

fn write_instance_claim(
    path: &Path,
    previous: Option<&ProjectClaim>,
    project_id: &str,
    project_path: &str,
    machine_id: Uuid,
    actor: &Actor,
    now: u64,
) -> Result<()> {
    write_atomic(
        path,
        &ProjectClaim {
            project_id: project_id.to_string(),
            project_path: project_path.to_string(),
            hostname: actor.host.clone(),
            machine_id,
            pid: actor.pid,
            claimed_at: previous.map_or(now, |claim| claim.claimed_at),
            heartbeat: now,
            instances: true,
        },
        actor,
    )
}

/// Hold the guard for a project this host runs: refresh it when it is ours,
/// take it when it is absent or stale, and leave a live holder on another
/// host alone.
///
/// Only its holder refreshes it. When that holder's instance stops, it
/// releases the guard, and whichever running instance refreshes next takes it
/// over; until then a daemon predating instances could start the project, for
/// at most one refresh interval. The marker is rewritten from `mode` every
/// time rather than carried over, so an exclusive daemon's guard never keeps a
/// stale marker, and a capable one never loses it.
#[allow(clippy::too_many_arguments)]
fn hold_guard(
    path: &Path,
    current: Option<&ProjectClaim>,
    project_id: &str,
    project_path: &str,
    machine_id: Uuid,
    mode: ClaimMode,
    actor: &Actor,
    now: u64,
) -> Result<()> {
    let claimed_at = match current {
        Some(claim) if claim.hostname == actor.host => claim.claimed_at,
        Some(claim) if !claim.is_stale(now) => return Ok(()),
        _ => now,
    };
    write_atomic(
        path,
        &ProjectClaim {
            project_id: project_id.to_string(),
            project_path: project_path.to_string(),
            hostname: actor.host.clone(),
            machine_id,
            pid: actor.pid,
            claimed_at,
            heartbeat: now,
            instances: mode == ClaimMode::Instances,
        },
        actor,
    )
}

/// Live instance claims for a project, any host's.
fn live_instance_claims(config_dir: &Path, project_id: &str, now: u64) -> Vec<ProjectClaim> {
    let Ok(entries) = std::fs::read_dir(instance_claims_dir(config_dir, project_id)) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|x| x == "json"))
        .filter_map(|path| read_json::<ProjectClaim>(&path))
        .filter(|claim| !claim.is_stale(now))
        .collect()
}

/// Keep this host's claims alive for the projects it is running, and only
/// those. `running` lists each as (project id, project path). Call on the
/// heartbeat tick, otherwise the claims age out and peers stop seeing
/// instances this host is actively running.
///
/// A claim this host stops refreshing lapses within [`STALE_AFTER_SECS`],
/// even when a stop path failed to release it. The refresh this replaced kept
/// every claim the daemon had ever written alive for as long as it ran, so a
/// project stopped on one host could not start on another until that daemon
/// exited.
///
/// A running instance's own claim is rewritten even if something removed it,
/// and the guard is retaken when absent or stale: that the instance runs here
/// is exactly what the claims state.
pub fn refresh_own_claims(
    config_dir: &Path,
    machine_id: Uuid,
    mode: ClaimMode,
    running: &[(String, String)],
) {
    refresh_own_claims_at(
        config_dir,
        machine_id,
        mode,
        running,
        &Actor::current(),
        now_secs(),
    )
}

fn refresh_own_claims_at(
    config_dir: &Path,
    machine_id: Uuid,
    mode: ClaimMode,
    running: &[(String, String)],
    actor: &Actor,
    now: u64,
) {
    for (project_id, project_path) in running {
        if mode == ClaimMode::Instances {
            let path = instance_claim_path(config_dir, project_id, machine_id);
            let previous =
                read_json::<ProjectClaim>(&path).filter(|claim| claim.hostname == actor.host);
            if let Err(err) = write_instance_claim(
                &path,
                previous.as_ref(),
                project_id,
                project_path,
                machine_id,
                actor,
                now,
            ) {
                tracing::warn!(project_id, %err, "could not refresh the instance claim");
            }
        }
        let guard_path = claim_path(config_dir, project_id);
        let guard = read_json::<ProjectClaim>(&guard_path);
        if let Some(holder) = guard
            .as_ref()
            .filter(|claim| claim.hostname != actor.host && !claim.is_stale(now))
        {
            if mode == ClaimMode::Exclusive || !holder.instances {
                // One of the two runs the project alone, so two runtimes are
                // live where only one may be. Seizing the guard would hide
                // that, and the operator needs to see it.
                tracing::warn!(
                    project_id,
                    holder = %holder.hostname,
                    "project is running here and on another host, and one of them cannot share it"
                );
            }
        }
        if let Err(err) = hold_guard(
            &guard_path,
            guard.as_ref(),
            project_id,
            project_path,
            machine_id,
            mode,
            actor,
            now,
        ) {
            tracing::warn!(project_id, %err, "could not refresh the project claim");
        }
    }
}

/// Release this machine's claim on a project and, if this host holds it, the
/// guard. Called whenever an instance here stops; another host's running
/// instance retakes the guard on its next refresh.
///
/// Never removes another host's claim. The guard goes only when it positively
/// names this host: an unreadable one may be a peer's seen through a
/// transient NFS error, and it ages out on its own if nobody refreshes it.
pub fn release_project(config_dir: &Path, project_id: &str, machine_id: Uuid) {
    release_project_as(config_dir, project_id, machine_id, &Actor::current());
}

fn release_project_as(config_dir: &Path, project_id: &str, machine_id: Uuid, actor: &Actor) {
    let own = instance_claim_path(config_dir, project_id, machine_id);
    if read_json::<ProjectClaim>(&own).is_none_or(|claim| claim.hostname == actor.host) {
        let _ = std::fs::remove_file(own);
    }
    let guard = claim_path(config_dir, project_id);
    if read_json::<ProjectClaim>(&guard).is_some_and(|claim| claim.hostname == actor.host) {
        let _ = std::fs::remove_file(guard);
    }
}

/// Release everything this daemon holds, on clean shutdown: every guard and
/// every instance claim naming this host and process.
pub fn release_all_own_claims(config_dir: &Path) {
    release_all_claims_of(config_dir, &Actor::current());
}

fn release_all_claims_of(config_dir: &Path, actor: &Actor) {
    let Ok(entries) = std::fs::read_dir(claims_dir(config_dir)) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(instances) = std::fs::read_dir(&path) {
                for instance in instances.filter_map(|e| e.ok()) {
                    remove_claim_if_held_by(&instance.path(), actor);
                }
            }
        } else {
            remove_claim_if_held_by(&path, actor);
        }
    }
}

fn remove_claim_if_held_by(path: &Path, actor: &Actor) {
    if path.extension().is_none_or(|x| x != "json") {
        return;
    }
    if let Some(claim) = read_json::<ProjectClaim>(path) {
        if claim.hostname == actor.host && claim.pid == actor.pid {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Releases an instance's claims when the instance ends.
///
/// Owned by the instance's task, so every way that task can end releases
/// them: a stop, a failure, a panic unwinding it, or an abort. Replacing the
/// whole daemon is the one exception, and keeps them on purpose: the task
/// calls [`InstanceClaimGuard::keep`] before asking for the replacement, and
/// the `exec` that follows runs no destructors at all. The replacement has the
/// same host and pid and takes the claims straight back when it resumes the
/// project, so peers never see it unclaimed.
pub struct InstanceClaimGuard {
    config_dir: PathBuf,
    project_id: String,
    machine_id: Uuid,
    armed: bool,
}

impl InstanceClaimGuard {
    pub fn new(config_dir: PathBuf, project_id: String, machine_id: Uuid) -> Self {
        Self {
            config_dir,
            project_id,
            machine_id,
            armed: true,
        }
    }

    /// Leave the claims in place when this guard goes.
    pub fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for InstanceClaimGuard {
    fn drop(&mut self) {
        if self.armed {
            release_project(&self.config_dir, &self.project_id, self.machine_id);
        }
    }
}

/// The other machines running `project_id`, from their live claims, and
/// this machine's own live claim beside them.
///
/// For the decision to adopt untagged panes at instance start: adopt unless
/// [`LiveInstances::blocks_adoption`]. The daemon writes this machine's claim
/// before it starts the instance, so two instances starting together each
/// see the other's claim, and the earlier claimant adopts. Pass this
/// machine's id so its own claim is told apart from the others.
///
/// Liveness is the claim's heartbeat ([`STALE_AFTER_SECS`]), never a pid.
pub fn live_instance_machines(
    config_dir: &Path,
    project_id: &str,
    own_machine_id: Uuid,
) -> LiveInstances {
    live_instance_machines_at(
        config_dir,
        project_id,
        own_machine_id,
        &Actor::current(),
        now_secs(),
    )
}

fn live_instance_machines_at(
    config_dir: &Path,
    project_id: &str,
    own_machine_id: Uuid,
    actor: &Actor,
    now: u64,
) -> LiveInstances {
    let mut live = LiveInstances::default();
    for claim in live_instance_claims(config_dir, project_id, now) {
        if claim.machine_id == own_machine_id {
            live.own = Some(claim.holder(now));
        } else {
            live.machines.push(claim.holder(now));
        }
    }
    if let Some(guard) = read_json::<ProjectClaim>(&claim_path(config_dir, project_id))
        .filter(|claim| claim.hostname != actor.host && !claim.is_stale(now))
    {
        if !guard.instances {
            live.exclusive_legacy = Some(guard.holder(now));
        } else if guard.machine_id != own_machine_id
            && !live
                .machines
                .iter()
                .any(|machine| machine.hostname == guard.hostname)
        {
            live.machines.push(guard.holder(now));
        }
    }
    live.machines.sort_by(|a, b| {
        a.hostname
            .cmp(&b.hostname)
            .then(a.machine_id.cmp(&b.machine_id))
    });
    live
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn host(name: &str, pid: u32) -> Actor {
        Actor {
            host: name.to_string(),
            pid,
        }
    }

    fn read_claim(path: &Path) -> Option<ProjectClaim> {
        read_json::<ProjectClaim>(path)
    }

    fn refusing_host(refusal: &ClaimRefusal) -> &str {
        match refusal {
            ClaimRefusal::ExclusiveHolder(holder) | ClaimRefusal::RunningElsewhere(holder) => {
                &holder.hostname
            }
        }
    }

    fn other_host_claim(id: &str, host: &str, heartbeat: u64) -> ProjectClaim {
        ProjectClaim {
            project_id: id.to_string(),
            project_path: "/proj".to_string(),
            hostname: host.to_string(),
            machine_id: Uuid::nil(),
            pid: 4242,
            claimed_at: heartbeat,
            heartbeat,
            instances: false,
        }
    }

    /// The guard exactly as a daemon predating instances declared it: no
    /// `instances` field, unknown fields ignored, which is serde's default.
    #[derive(Debug, Deserialize)]
    struct PreInstancesClaim {
        #[allow(dead_code)]
        project_id: String,
        #[allow(dead_code)]
        project_path: String,
        hostname: String,
        #[allow(dead_code)]
        machine_id: Uuid,
        #[allow(dead_code)]
        pid: u32,
        #[allow(dead_code)]
        claimed_at: u64,
        heartbeat: u64,
    }

    /// What such a daemon concluded from that guard in `claim_project`: a
    /// claim from another host refreshed within the window is a live peer,
    /// and it does not spawn.
    fn pre_instances_daemon_sees_live_peer(path: &Path, its_host: &str, now: u64) -> bool {
        let body = std::fs::read_to_string(path).expect("guard present");
        let claim: PreInstancesClaim = serde_json::from_str(&body).expect("older daemon parses it");
        claim.hostname != its_host && now.saturating_sub(claim.heartbeat) <= STALE_AFTER_SECS
    }

    #[test]
    fn reclaiming_our_own_host_claim_adopts_the_current_pid() {
        // A claim written by a *previous* daemon on this host carries that
        // daemon's pid. Shutdown releases only claims naming the live pid, so
        // keeping the old one would leave them behind; and a guard naming a
        // dead pid looks like nobody's. Both modes adopt the live pid.
        for mode in [ClaimMode::Exclusive, ClaimMode::Instances] {
            let dir = TempDir::new().unwrap();
            let machine = Uuid::new_v4();
            let now = now_secs();
            let earlier = host("zoo-005", 999_999);
            claim_project_at(
                dir.path(),
                "proj-1",
                "/p",
                machine,
                mode,
                &earlier,
                now - 100,
            )
            .unwrap();

            let current = host("zoo-005", 4321);
            let outcome =
                claim_project_at(dir.path(), "proj-1", "/p", machine, mode, &current, now).unwrap();
            assert_eq!(outcome, ClaimOutcome::AlreadyOurs, "{mode:?}");

            let guard = read_claim(&claim_path(dir.path(), "proj-1")).expect("guard present");
            assert_eq!(guard.pid, 4321, "{mode:?}: the guard names the live daemon");
            assert_eq!(
                guard.claimed_at,
                now - 100,
                "{mode:?}: first claim time kept"
            );
            if mode == ClaimMode::Instances {
                let own = read_claim(&instance_claim_path(dir.path(), "proj-1", machine)).unwrap();
                assert_eq!(own.pid, 4321);
                assert_eq!(own.heartbeat, now);
            }

            // And the whole point: it is now released with this daemon.
            release_all_claims_of(dir.path(), &current);
            assert!(!claim_path(dir.path(), "proj-1").exists(), "{mode:?}");
            assert!(
                !instance_claim_path(dir.path(), "proj-1", machine).exists(),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_live_peer_claim_is_still_refused_after_the_pid_change() {
        // The pid adoption must apply only to our own hostname — never as a
        // way to take a claim a live peer holds.
        let dir = tempfile::tempdir().unwrap();
        let path = claim_path(dir.path(), "proj-2");
        write_atomic(
            &path,
            &ProjectClaim {
                project_id: "proj-2".to_string(),
                project_path: "/p".to_string(),
                hostname: "some-other-host".to_string(),
                machine_id: Uuid::new_v4(),
                pid: 4242,
                claimed_at: now_secs(),
                heartbeat: now_secs(),
                instances: false,
            },
            &host("some-other-host", 4242),
        )
        .unwrap();

        for mode in [ClaimMode::Exclusive, ClaimMode::Instances] {
            let outcome = claim_project(dir.path(), "proj-2", "/p", Uuid::new_v4(), mode).unwrap();
            assert!(matches!(outcome, ClaimOutcome::HeldBy(_)), "{mode:?}");
            let stored = read_claim(&path).unwrap();
            assert_eq!(stored.hostname, "some-other-host", "peer claim untouched");
            assert_eq!(stored.pid, 4242);
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

    /// A machine id configured in the shared `config.toml` is every host's
    /// machine id, so it cannot tell instances apart.
    #[test]
    fn a_configured_machine_id_turns_instances_off() {
        assert!(project_instances_supported(None));
        assert!(project_instances_supported(Some("")));
        assert!(project_instances_supported(Some("  ")));
        assert!(!project_instances_supported(Some(
            "6ba7b811-9dad-11d1-80b4-00c04fd430c8"
        )));
        // Ignored in favour of the derived id, but someone meant to override
        // this host's identity: decline rather than guess.
        assert!(!project_instances_supported(Some("not-a-uuid")));

        assert_eq!(
            ClaimMode::for_configured_machine_id(None),
            ClaimMode::Instances
        );
        assert_eq!(
            ClaimMode::for_configured_machine_id(Some("6ba7b811-9dad-11d1-80b4-00c04fd430c8")),
            ClaimMode::Exclusive
        );
    }

    #[test]
    fn claim_is_acquired_when_free_and_refreshed_when_ours() {
        for mode in [ClaimMode::Exclusive, ClaimMode::Instances] {
            let dir = TempDir::new().unwrap();
            let id = Uuid::new_v4();
            assert_eq!(
                claim_project(dir.path(), "p1", "/proj", id, mode).unwrap(),
                ClaimOutcome::Acquired,
                "{mode:?}"
            );
            assert_eq!(
                claim_project(dir.path(), "p1", "/proj", id, mode).unwrap(),
                ClaimOutcome::AlreadyOurs,
                "{mode:?}"
            );
        }
    }

    /// The layout and marker the rest of the cluster depends on: this
    /// machine's own claim under `claims/<project>/`, and the guard marked so
    /// other capable daemons may join while older ones stay out.
    #[test]
    fn a_capable_daemon_claims_its_machine_and_marks_the_guard() {
        let dir = TempDir::new().unwrap();
        let machine = Uuid::new_v4();
        let zoo5 = host("zoo-005", 11);
        let now = now_secs();
        assert_eq!(
            claim_project_at(
                dir.path(),
                "p1",
                "/proj",
                machine,
                ClaimMode::Instances,
                &zoo5,
                now
            )
            .unwrap(),
            ClaimOutcome::Acquired
        );

        let own_path = claims_dir(dir.path())
            .join("p1")
            .join(format!("{machine}.json"));
        let own = read_claim(&own_path).expect("instance claim at claims/<project>/<machine>.json");
        assert_eq!(own.hostname, "zoo-005");
        assert_eq!(own.machine_id, machine);
        assert_eq!(own.project_path, "/proj");

        let guard_path = claims_dir(dir.path()).join("p1.json");
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&guard_path).unwrap()).unwrap();
        assert_eq!(raw["instances"], serde_json::json!(true));
        assert_eq!(raw["hostname"], serde_json::json!("zoo-005"));

        // An exclusive daemon's guard is byte-for-byte the old shape.
        let exclusive_dir = TempDir::new().unwrap();
        claim_project_at(
            exclusive_dir.path(),
            "p1",
            "/proj",
            machine,
            ClaimMode::Exclusive,
            &zoo5,
            now,
        )
        .unwrap();
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(claims_dir(exclusive_dir.path()).join("p1.json")).unwrap(),
        )
        .unwrap();
        assert!(raw.get("instances").is_none(), "{raw}");
        assert!(
            !instance_claims_dir(exclusive_dir.path(), "p1").exists(),
            "a shared machine id has no claim of its own to write"
        );
    }

    /// zoo-006 starting a project zoo-005 already runs becomes a second
    /// instance; it does not take zoo-005's guard.
    #[test]
    fn a_second_capable_machine_joins_beside_a_running_instance() {
        let dir = TempDir::new().unwrap();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (zoo5, zoo6) = (host("zoo-005", 11), host("zoo-006", 12));
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            m5,
            ClaimMode::Instances,
            &zoo5,
            now,
        )
        .unwrap();

        assert_eq!(
            claim_project_at(
                dir.path(),
                "p1",
                "/proj",
                m6,
                ClaimMode::Instances,
                &zoo6,
                now
            )
            .unwrap(),
            ClaimOutcome::Acquired
        );
        assert!(instance_claim_path(dir.path(), "p1", m5).exists());
        assert!(instance_claim_path(dir.path(), "p1", m6).exists());
        let guard = read_claim(&claim_path(dir.path(), "p1")).unwrap();
        assert_eq!(guard.hostname, "zoo-005", "the guard stays with its holder");

        // Each sees the other as a live instance, and its own claim beside it.
        // Claimed in the same second, the two are ordered by machine id for
        // adopting untagged panes: exactly one of them may.
        let seen_by_6 = live_instance_machines_at(dir.path(), "p1", m6, &zoo6, now);
        assert_eq!(
            seen_by_6
                .machines
                .iter()
                .map(|m| m.hostname.as_str())
                .collect::<Vec<_>>(),
            vec!["zoo-005"]
        );
        assert!(seen_by_6.exclusive_legacy.is_none());
        let seen_by_5 = live_instance_machines_at(dir.path(), "p1", m5, &zoo5, now);
        assert_eq!(
            seen_by_5
                .machines
                .iter()
                .map(|m| m.hostname.as_str())
                .collect::<Vec<_>>(),
            vec!["zoo-006"]
        );
        assert!(seen_by_5.any());
        assert_eq!(seen_by_5.own.as_ref().map(|own| own.machine_id), Some(m5));
        assert_eq!(seen_by_6.own.as_ref().map(|own| own.machine_id), Some(m6));
        assert_ne!(
            seen_by_5.blocks_adoption(),
            seen_by_6.blocks_adoption(),
            "one answer between them"
        );
        assert_eq!(seen_by_5.blocks_adoption(), m6 < m5);
    }

    /// The bug this prevents: shared projects.json + a /proc-only guard let
    /// two hosts spawn the same project over NFS. An unmarked guard means its
    /// holder runs the project alone, so even a capable daemon stays out.
    #[test]
    fn an_unmarked_live_guard_keeps_every_daemon_out() {
        let dir = TempDir::new().unwrap();
        let path = claims_dir(dir.path()).join("p1.json");
        write_atomic(
            &path,
            &other_host_claim("p1", "older-peer.invalid", now_secs()),
            &host("older-peer.invalid", 1),
        )
        .unwrap();

        match claim_project(
            dir.path(),
            "p1",
            "/proj",
            Uuid::new_v4(),
            ClaimMode::Exclusive,
        )
        .unwrap()
        {
            ClaimOutcome::HeldBy(refusal) => {
                assert_eq!(refusing_host(&refusal), "older-peer.invalid")
            }
            other => panic!("expected the live peer to keep the project, got {other:?}"),
        }

        let machine = Uuid::new_v4();
        match claim_project(dir.path(), "p1", "/proj", machine, ClaimMode::Instances).unwrap() {
            ClaimOutcome::HeldBy(refusal) => {
                assert!(matches!(refusal, ClaimRefusal::ExclusiveHolder(_)));
                assert_eq!(refusing_host(&refusal), "older-peer.invalid");
                assert!(refusal
                    .reason()
                    .contains("reboot older-peer.invalid to update it"));
            }
            other => panic!("an exclusive holder must refuse a second instance, got {other:?}"),
        }
        assert!(
            !instance_claim_path(dir.path(), "p1", machine).exists(),
            "a refused start claims nothing"
        );
        assert_eq!(read_claim(&path).unwrap().hostname, "older-peer.invalid");
    }

    /// A daemon predating instances reads the marked guard as an ordinary
    /// live peer, because it ignores the marker. That is the whole exclusion.
    #[test]
    fn a_marked_guard_keeps_an_older_daemon_out() {
        let dir = TempDir::new().unwrap();
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            Uuid::new_v4(),
            ClaimMode::Instances,
            &host("zoo-005", 11),
            now,
        )
        .unwrap();
        let guard = claim_path(dir.path(), "p1");
        assert!(pre_instances_daemon_sees_live_peer(&guard, "zoo-006", now));

        // So does this build when it runs projects alone.
        match claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            Uuid::new_v4(),
            ClaimMode::Exclusive,
            &host("zoo-006", 12),
            now,
        )
        .unwrap()
        {
            ClaimOutcome::HeldBy(ClaimRefusal::RunningElsewhere(holder)) => {
                assert_eq!(holder.hostname, "zoo-005")
            }
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    /// Between a guard's holder stopping and another instance retaking it,
    /// only the instance claims say the project still runs. An exclusive
    /// daemon must read them too.
    #[test]
    fn an_exclusive_daemon_respects_a_live_instance_without_a_guard() {
        let dir = TempDir::new().unwrap();
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            Uuid::new_v4(),
            ClaimMode::Instances,
            &host("zoo-005", 11),
            now,
        )
        .unwrap();
        std::fs::remove_file(claim_path(dir.path(), "p1")).unwrap();

        match claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            Uuid::new_v4(),
            ClaimMode::Exclusive,
            &host("zoo-006", 12),
            now,
        )
        .unwrap()
        {
            ClaimOutcome::HeldBy(refusal) => assert_eq!(refusing_host(&refusal), "zoo-005"),
            other => panic!("expected refusal, got {other:?}"),
        }
        assert!(!claim_path(dir.path(), "p1").exists());
    }

    #[test]
    fn stale_peer_claim_is_taken_over() {
        // A host that died must not strand its projects forever.
        for mode in [ClaimMode::Exclusive, ClaimMode::Instances] {
            let dir = TempDir::new().unwrap();
            let path = claims_dir(dir.path()).join("p1.json");
            let long_ago = now_secs().saturating_sub(STALE_AFTER_SECS + 30);
            write_atomic(
                &path,
                &other_host_claim("p1", "dead-peer.invalid", long_ago),
                &host("dead-peer.invalid", 1),
            )
            .unwrap();

            assert_eq!(
                claim_project(dir.path(), "p1", "/proj", Uuid::new_v4(), mode).unwrap(),
                ClaimOutcome::Acquired,
                "{mode:?}"
            );
            let guard = read_claim(&path).unwrap();
            assert_eq!(guard.hostname, hostname(), "{mode:?}");
            assert_eq!(guard.instances, mode == ClaimMode::Instances, "{mode:?}");
        }
    }

    /// Stopping on one machine frees the project at once: the instance claim
    /// and the guard go, and another host starts it without waiting out the
    /// staleness window or the first daemon's exit.
    #[test]
    fn a_released_project_starts_elsewhere_immediately() {
        let dir = TempDir::new().unwrap();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (zoo5, zoo6) = (host("zoo-005", 11), host("zoo-006", 12));
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            m5,
            ClaimMode::Instances,
            &zoo5,
            now,
        )
        .unwrap();

        release_project_as(dir.path(), "p1", m5, &zoo5);
        assert!(!instance_claim_path(dir.path(), "p1", m5).exists());
        assert!(!claim_path(dir.path(), "p1").exists());
        assert!(!live_instance_machines_at(dir.path(), "p1", m6, &zoo6, now).any());

        // Even a daemon that runs projects alone may start it now.
        assert_eq!(
            claim_project_at(
                dir.path(),
                "p1",
                "/proj",
                m6,
                ClaimMode::Exclusive,
                &zoo6,
                now
            )
            .unwrap(),
            ClaimOutcome::Acquired
        );
    }

    #[test]
    fn release_never_touches_another_hosts_claim() {
        let dir = TempDir::new().unwrap();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (zoo5, zoo6) = (host("zoo-005", 11), host("zoo-006", 12));
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            m5,
            ClaimMode::Instances,
            &zoo5,
            now,
        )
        .unwrap();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            m6,
            ClaimMode::Instances,
            &zoo6,
            now,
        )
        .unwrap();

        // zoo-006 stops: its own claim goes, zoo-005's claim and guard stay.
        release_project_as(dir.path(), "p1", m6, &zoo6);
        assert!(!instance_claim_path(dir.path(), "p1", m6).exists());
        assert!(instance_claim_path(dir.path(), "p1", m5).exists());
        assert_eq!(
            read_claim(&claim_path(dir.path(), "p1")).unwrap().hostname,
            "zoo-005"
        );

        // A release naming zoo-005's machine from zoo-006 removes nothing.
        release_project_as(dir.path(), "p1", m5, &zoo6);
        assert!(instance_claim_path(dir.path(), "p1", m5).exists());

        // And the public path, from this host, leaves an old-style peer alone.
        let legacy = TempDir::new().unwrap();
        let path = claims_dir(legacy.path()).join("p1.json");
        write_atomic(
            &path,
            &other_host_claim("p1", "older-peer.invalid", now),
            &host("older-peer.invalid", 1),
        )
        .unwrap();
        release_project(legacy.path(), "p1", Uuid::new_v4());
        assert!(
            path.exists(),
            "a peer's claim must survive our release call"
        );
    }

    /// The guard is refreshed only by its holder. When the holder's instance
    /// stops, the next running instance to refresh takes it, so a daemon
    /// predating instances keeps seeing a live peer for as long as any
    /// capable instance runs.
    #[test]
    fn the_guard_passes_to_a_running_instance_when_its_holder_stops() {
        let dir = TempDir::new().unwrap();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (zoo5, zoo6) = (host("zoo-005", 11), host("zoo-006", 12));
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            m5,
            ClaimMode::Instances,
            &zoo5,
            now,
        )
        .unwrap();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            m6,
            ClaimMode::Instances,
            &zoo6,
            now,
        )
        .unwrap();
        let running = vec![("p1".to_string(), "/proj".to_string())];

        // While zoo-005 holds it, zoo-006's refresh leaves it alone.
        refresh_own_claims_at(
            dir.path(),
            m6,
            ClaimMode::Instances,
            &running,
            &zoo6,
            now + 10,
        );
        let guard = read_claim(&claim_path(dir.path(), "p1")).unwrap();
        assert_eq!(guard.hostname, "zoo-005");
        assert_eq!(guard.heartbeat, now, "only the holder refreshes it");

        release_project_as(dir.path(), "p1", m5, &zoo5);
        refresh_own_claims_at(
            dir.path(),
            m6,
            ClaimMode::Instances,
            &running,
            &zoo6,
            now + 20,
        );
        let guard = read_claim(&claim_path(dir.path(), "p1")).unwrap();
        assert_eq!(guard.hostname, "zoo-006");
        assert!(guard.instances);
        assert_eq!(guard.heartbeat, now + 20);
        assert!(pre_instances_daemon_sees_live_peer(
            &claim_path(dir.path(), "p1"),
            "zoo-002",
            now + 20
        ));
    }

    /// Only running instances are kept alive. The refresh this replaced
    /// renewed every claim the daemon had ever written, so a project stopped
    /// here could never start elsewhere while this daemon lived.
    #[test]
    fn refresh_keeps_only_running_instances_claimed() {
        let dir = TempDir::new().unwrap();
        let machine = Uuid::new_v4();
        let zoo5 = host("zoo-005", 11);
        let now = now_secs();
        for project in ["running", "stopped"] {
            let mode = ClaimMode::Instances;
            claim_project_at(dir.path(), project, "/proj", machine, mode, &zoo5, now).unwrap();
        }
        let running = vec![("running".to_string(), "/proj".to_string())];
        refresh_own_claims_at(
            dir.path(),
            machine,
            ClaimMode::Instances,
            &running,
            &zoo5,
            now + 60,
        );

        let refreshed = read_claim(&instance_claim_path(dir.path(), "running", machine)).unwrap();
        assert_eq!(refreshed.heartbeat, now + 60);
        assert_eq!(refreshed.claimed_at, now);
        assert_eq!(
            read_claim(&claim_path(dir.path(), "running"))
                .unwrap()
                .heartbeat,
            now + 60
        );
        assert_eq!(
            read_claim(&instance_claim_path(dir.path(), "stopped", machine))
                .unwrap()
                .heartbeat,
            now,
            "a stopped project's claim is left to lapse"
        );
        assert_eq!(
            read_claim(&claim_path(dir.path(), "stopped"))
                .unwrap()
                .heartbeat,
            now
        );

        // Once lapsed, peers no longer see the stopped one.
        let other = Uuid::new_v4();
        let zoo6 = host("zoo-006", 12);
        let later = now + 60 + STALE_AFTER_SECS;
        assert!(!live_instance_machines_at(dir.path(), "stopped", other, &zoo6, later).any());
        assert!(live_instance_machines_at(dir.path(), "running", other, &zoo6, later).any());
    }

    /// Running is what the claim states, so a refresh restores a running
    /// instance's claim even if something removed it.
    #[test]
    fn refresh_restores_a_running_instances_missing_claims() {
        let dir = TempDir::new().unwrap();
        let machine = Uuid::new_v4();
        let zoo5 = host("zoo-005", 11);
        let now = now_secs();
        let running = vec![("p1".to_string(), "/proj".to_string())];
        refresh_own_claims_at(
            dir.path(),
            machine,
            ClaimMode::Instances,
            &running,
            &zoo5,
            now,
        );
        assert_eq!(
            read_claim(&instance_claim_path(dir.path(), "p1", machine))
                .unwrap()
                .hostname,
            "zoo-005"
        );
        assert!(read_claim(&claim_path(dir.path(), "p1")).unwrap().instances);

        // An exclusive daemon writes only the unmarked guard.
        let exclusive = TempDir::new().unwrap();
        let mode = ClaimMode::Exclusive;
        refresh_own_claims_at(exclusive.path(), machine, mode, &running, &zoo5, now);
        assert!(
            !read_claim(&claim_path(exclusive.path(), "p1"))
                .unwrap()
                .instances
        );
        assert!(!instance_claims_dir(exclusive.path(), "p1").exists());
    }

    #[test]
    fn live_instances_exclude_this_machine_and_stale_claims() {
        let dir = TempDir::new().unwrap();
        let (mine, live, stale) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let now = now_secs();
        claim_project_at(
            dir.path(),
            "p1",
            "/p",
            mine,
            ClaimMode::Instances,
            &host("me", 1),
            now,
        )
        .unwrap();
        let zoo6 = host("zoo-006", 2);
        claim_project_at(
            dir.path(),
            "p1",
            "/p",
            live,
            ClaimMode::Instances,
            &zoo6,
            now,
        )
        .unwrap();
        claim_project_at(
            dir.path(),
            "p1",
            "/p",
            stale,
            ClaimMode::Instances,
            &host("zoo-004", 3),
            now - STALE_AFTER_SECS - 30,
        )
        .unwrap();

        let seen = live_instance_machines_at(dir.path(), "p1", mine, &host("me", 1), now);
        assert_eq!(
            seen.machines,
            vec![InstanceHolder {
                machine_id: live,
                hostname: "zoo-006".to_string(),
                claimed_at: now,
                age_secs: 0,
            }]
        );
        assert!(seen.exclusive_legacy.is_none());
        assert_eq!(
            seen.own,
            Some(InstanceHolder {
                machine_id: mine,
                hostname: "me".to_string(),
                claimed_at: now,
                age_secs: 0,
            }),
            "this machine's own claim is reported apart from the others"
        );

        // An older daemon's guard is reported as such.
        let older = TempDir::new().unwrap();
        write_atomic(
            &claim_path(older.path(), "p1"),
            &other_host_claim("p1", "zoo-002", now),
            &host("zoo-002", 1),
        )
        .unwrap();
        let seen = live_instance_machines_at(older.path(), "p1", mine, &host("me", 1), now);
        assert_eq!(
            seen.exclusive_legacy.map(|h| h.hostname),
            Some("zoo-002".to_string())
        );
        assert!(seen.machines.is_empty());
    }

    /// Instances starting together each see the other's claim, because both
    /// daemons claim before either instance runs. The earlier claimant adopts
    /// untagged panes; when presence alone decided, neither did.
    #[test]
    fn only_the_earliest_live_claimant_may_adopt() {
        let dir = TempDir::new().unwrap();
        let now = now_secs();
        let instances = ClaimMode::Instances;
        // The earlier claimant gets the larger id, so it is the claim times,
        // not the tie-break, that order these two.
        let mut ids = [Uuid::new_v4(), Uuid::new_v4()];
        ids.sort();
        let (later, earlier) = (ids[0], ids[1]);
        let (zoo5, zoo6) = (host("zoo-005", 11), host("zoo-006", 12));
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            earlier,
            instances,
            &zoo5,
            now - 30,
        )
        .unwrap();
        claim_project_at(dir.path(), "p1", "/proj", later, instances, &zoo6, now).unwrap();
        // A claim that went stale long ago counts for nothing, however early.
        let zoo4 = host("zoo-004", 13);
        let long_ago = now - STALE_AFTER_SECS - 300;
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            Uuid::nil(),
            instances,
            &zoo4,
            long_ago,
        )
        .unwrap();
        let seen = |machine: Uuid, actor: &Actor| {
            live_instance_machines_at(dir.path(), "p1", machine, actor, now)
        };

        assert!(!seen(earlier, &zoo5).blocks_adoption());
        assert!(seen(later, &zoo6).blocks_adoption());

        // A run no daemon claimed, such as a standalone `--headless`, has no
        // start to order: any other live instance rules it out, and nothing
        // does when nothing else runs.
        let unclaimed = seen(Uuid::new_v4(), &host("zoo-007", 14));
        assert!(unclaimed.own.is_none());
        assert!(unclaimed.blocks_adoption());
        let alone = TempDir::new().unwrap();
        assert!(
            !live_instance_machines_at(alone.path(), "p1", earlier, &zoo5, now).blocks_adoption()
        );

        // A capable host seen only through the guard is ordered by when it
        // took the guard.
        let guard_only = TempDir::new().unwrap();
        claim_project_at(
            guard_only.path(),
            "p1",
            "/proj",
            earlier,
            instances,
            &zoo5,
            now - 30,
        )
        .unwrap();
        let mut guard = other_host_claim("p1", "zoo-009", now);
        guard.instances = true;
        guard.machine_id = Uuid::new_v4();
        for (claimed_at, blocks) in [(now - 60, true), (now, false)] {
            guard.claimed_at = claimed_at;
            write_atomic(
                &claim_path(guard_only.path(), "p1"),
                &guard,
                &host("zoo-009", 1),
            )
            .unwrap();
            let seen = live_instance_machines_at(guard_only.path(), "p1", earlier, &zoo5, now);
            assert_eq!(
                seen.blocks_adoption(),
                blocks,
                "guard claimed at {claimed_at}"
            );
        }

        // A host that runs the project alone rules out even the earliest
        // claimant: it runs every untagged pane itself.
        write_atomic(
            &claim_path(dir.path(), "p1"),
            &other_host_claim("p1", "zoo-002", now),
            &host("zoo-002", 1),
        )
        .unwrap();
        assert!(seen(earlier, &zoo5).blocks_adoption());
    }

    /// A temp name carrying only the pid could be the same name on two hosts
    /// sharing the directory.
    #[test]
    fn temp_names_carry_host_and_pid_and_never_look_like_records() {
        let path = Path::new("/c/claims/p1.json");
        let a = temp_path_for(path, &host("zoo-005", 77));
        let b = temp_path_for(path, &host("zoo-006", 77));
        assert_ne!(a, b, "same pid on two hosts must not share a temp file");
        let name = a.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, ".p1.json.zoo-005.77.tmp");
        assert_eq!(
            a.parent(),
            path.parent(),
            "staged beside the record for the rename"
        );
        assert!(a.extension().is_some_and(|x| x != "json"));
        // A hostile hostname cannot escape the directory either.
        let odd = temp_path_for(path, &host("../evil/host", 1));
        assert_eq!(odd.parent(), path.parent());

        // Nothing is left behind after real writes.
        let dir = TempDir::new().unwrap();
        claim_project(
            dir.path(),
            "p1",
            "/proj",
            Uuid::new_v4(),
            ClaimMode::Instances,
        )
        .unwrap();
        let mut leftovers = Vec::new();
        for entry in walk(&claims_dir(dir.path())) {
            if entry.extension().is_none_or(|x| x != "json") {
                leftovers.push(entry);
            }
        }
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }

    #[test]
    fn an_instance_guard_releases_on_drop_unless_kept() {
        let dir = TempDir::new().unwrap();
        let machine = Uuid::new_v4();
        claim_project(dir.path(), "p1", "/proj", machine, ClaimMode::Instances).unwrap();
        drop(InstanceClaimGuard::new(
            dir.path().to_path_buf(),
            "p1".into(),
            machine,
        ));
        assert!(!instance_claim_path(dir.path(), "p1", machine).exists());
        assert!(!claim_path(dir.path(), "p1").exists());

        claim_project(dir.path(), "p2", "/proj", machine, ClaimMode::Instances).unwrap();
        InstanceClaimGuard::new(dir.path().to_path_buf(), "p2".into(), machine).keep();
        assert!(instance_claim_path(dir.path(), "p2", machine).exists());
        assert!(claim_path(dir.path(), "p2").exists());
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
        for (name, hb) in [
            ("zoo-002", now),
            ("zoo-004", now.saturating_sub(STALE_AFTER_SECS + 30)),
            (&hostname(), now),
        ] {
            write_atomic(
                &daemons_dir(dir.path()).join(format!("{name}.json")),
                &mk(name, hb),
                &host(name, 1),
            )
            .unwrap();
        }

        let peers = live_peers(dir.path());
        let names: Vec<&str> = peers.iter().map(|p| p.hostname.as_str()).collect();
        assert_eq!(
            names,
            vec!["zoo-002"],
            "self and stale hosts must be excluded"
        );
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
        assert!(!daemons_dir(dir.path())
            .join(format!("{}.json", hostname()))
            .exists());
    }

    #[test]
    fn guard_withdraws_on_drop_even_without_a_clean_loop_exit() {
        // Regression: withdrawal used to live inside the connected event loop,
        // so a daemon SIGINTed while still retrying its connection never
        // reached it and left its record behind for peers to see.
        let dir = TempDir::new().unwrap();
        let id = derive_machine_id();
        publish_self(dir.path(), id, "test").unwrap();
        claim_project(dir.path(), "p1", "/proj", id, ClaimMode::Instances).unwrap();
        claim_project(dir.path(), "p2", "/proj", id, ClaimMode::Exclusive).unwrap();
        // A peer's instance claim beside ours must survive our shutdown.
        let peer = Uuid::new_v4();
        claim_project_at(
            dir.path(),
            "p1",
            "/proj",
            peer,
            ClaimMode::Instances,
            &host("peer.invalid", 12),
            now_secs(),
        )
        .unwrap();

        let record = daemons_dir(dir.path()).join(format!("{}.json", hostname()));
        assert!(record.exists());

        {
            let _guard = RegistrationGuard::new(dir.path().to_path_buf());
        } // dropped here, as it would be on any early return

        assert!(
            !record.exists(),
            "daemon record should be withdrawn on drop"
        );
        assert!(
            !claims_dir(dir.path()).join("p1.json").exists(),
            "our claims should be released on drop"
        );
        assert!(!claims_dir(dir.path()).join("p2.json").exists());
        assert!(
            !instance_claim_path(dir.path(), "p1", id).exists(),
            "including the instance claims under claims/<project>/"
        );
        assert!(instance_claim_path(dir.path(), "p1", peer).exists());
    }

    #[test]
    fn claim_filenames_cannot_escape_the_directory() {
        let dir = TempDir::new().unwrap();
        let p = claim_path(dir.path(), "../../etc/passwd");
        assert_eq!(p.parent().unwrap(), claims_dir(dir.path()));
        assert!(!p.to_string_lossy().contains(".."));
        let instance = instance_claim_path(dir.path(), "../../etc/passwd", Uuid::nil());
        assert_eq!(
            instance.parent().unwrap().parent().unwrap(),
            claims_dir(dir.path())
        );
        assert!(!instance.to_string_lossy().contains(".."));
    }
}
