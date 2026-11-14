//! Autopilot's private files between the project CLI and the OMP extension.
//!
//! The loop runs inside the tab's OMP process, in `omp_activity.mjs`. The CLI
//! never drives it directly: it leaves requests in the pane's runtime
//! directory (`…/apas/panes/<project>/<pane>/`), reads what the extension
//! reports there, and writes a heartbeat that says a controller is present.
//! The directory and every file are private to the OS account (0700/0600),
//! as for the question bridge. The design record is
//! `openspec/changes/archive/2026-11-13-add-omp-tab-autopilot/design.md`,
//! decisions 5 to 8.
//!
//! These files are not an authority boundary. The checks show which launch a
//! file belongs to, not who wrote it.
//!
//! # Files
//!
//! Written by the CLI, read by the extension. They carry the launch id of
//! `omp-launch.json` and nothing about the OMP process, so the CLI can write
//! them from the launch record alone:
//!
//! - [`CONTROLLER_FILE`]: [`ControllerHeartbeat`], every three seconds, from
//!   a thread that never sends on the project's outbound queue.
//! - [`REQUEST_FILE`]: [`StartRequest`], a start or a resume. Never left
//!   pending: the extension takes it by renaming it to
//!   `omp-autopilot-request.taken-<pid>` before applying it, and refuses one
//!   issued more than [`REQUEST_MAX_AGE_MS`] ago or in the future. When the
//!   CLI's wait ends it removes the file; a removal that succeeds means the
//!   tab never took the request.
//! - [`STOP_FILE`]: [`StopRequest`], a pause, stop or stop now. It stays in
//!   place until the extension has applied it. Only the CLI writes, replaces
//!   or removes it, always under [`LOCK_FILE`]. The extension never deletes
//!   it: it records the request id in the acknowledgement file, and the CLI
//!   removes an acknowledged request the next time it holds the lock.
//!
//! Written by the extension (temp file and rename), read by the CLI with the
//! same trust checks as `omp-activity.json` (private file, launch id, pid,
//! process start):
//!
//! - [`ACK_FILE`]: [`AckFile`], the results of the last [`ACK_KEEP`] requests.
//! - [`STATE_FILE`]: [`StateReport`], rewritten on every change and at least
//!   every 30 seconds. The CLI treats one it has not seen refreshed for
//!   [`STATE_STALE_AFTER`] as not reported.
//!
//! `omp-launch.json` gains `"features": ["autopilot_v1"]` when the provider
//! is spawned by a CLI that has autopilot. A launch record without it means
//! the agent needs a reboot before it can run autopilot.
//!
//! Every file is at most [`MAX_BYTES`].
//!
//! # Journal
//!
//! The extension journals the autopilot in the tab's OMP session as custom
//! entries of type [`JOURNAL_ENTRY_TYPE`] whose data is a [`JournalEntry`].
//! The transcript reader uses them to tell an iteration from a person's
//! message; see [`JournalEntry`].

#![allow(dead_code)]

use crate::omp_activity::{
    pane_runtime_dir, private_owner, process_start, read_launch, read_private_json_bounded,
    write_private_atomic, Launch,
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use shared::{AutopilotCommand, AutopilotReceiptOutcome, AutopilotSupport};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const CONTROLLER_FILE: &str = "apas-controller.json";
pub const REQUEST_FILE: &str = "omp-autopilot-request.json";
pub const STOP_FILE: &str = "omp-autopilot-stop.json";
pub const ACK_FILE: &str = "omp-autopilot-ack.json";
pub const STATE_FILE: &str = "omp-autopilot.json";
pub const LOCK_FILE: &str = "omp-autopilot.lock";
/// Feature marker in `omp-launch.json`.
pub const LAUNCH_FEATURE: &str = "autopilot_v1";
/// `customType` of the journal entries in the OMP session.
pub const JOURNAL_ENTRY_TYPE: &str = "apas-autopilot";

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_BYTES: u64 = 64 * 1024;
/// How many request results the extension keeps in [`ACK_FILE`].
pub const ACK_KEEP: usize = 16;

/// The heartbeat is written this often.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
/// The extension treats the heartbeat as fresh this long after it last saw
/// the writer id or the counter change.
pub const HEARTBEAT_FRESH_SECS: u64 = 60;
/// The CLI reports "connected" only while a frame arrived from the server
/// within this long, the bound the server uses on its side.
pub const SERVER_SILENCE_LIMIT: Duration = Duration::from_secs(90);
/// The extension holds once the server has been unreachable this long.
pub const SERVER_UNREACHABLE_HOLD_SECS: u64 = 600;

/// The CLI waits at most this long for its lock.
pub const LOCK_WAIT: Duration = Duration::from_secs(1);
/// The CLI waits this long for the extension's acknowledgement.
pub const ACK_WAIT: Duration = Duration::from_secs(2);
/// After the extension has taken a start or resume, the CLI waits this much
/// longer for its result.
pub const TAKEN_WAIT: Duration = Duration::from_secs(1);
/// The extension refuses a start or resume issued longer ago than this.
pub const REQUEST_MAX_AGE_MS: u64 = 2_000;
/// A state report not seen refreshed for this long is not reported.
pub const STATE_STALE_AFTER: Duration = Duration::from_secs(90);

/// `apas-controller.json`: proof that a controller is present, and what it
/// knows about the server.
///
/// The extension notes on its own monotonic clock when it last saw `writer`
/// or `counter` change, and compares for difference, never for order: a
/// replaced CLI process writes a new id and starts its counter again. No
/// wall-clock time is compared.
///
/// The extension holds while the heartbeat is not fresh, while
/// `!connected && unreachable_secs > 600`, or while
/// `connected && !server_controls_autopilot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerHeartbeat {
    pub v: u32,
    pub launch_id: Uuid,
    /// Chosen at random when the CLI process starts.
    pub writer: Uuid,
    /// Changes on every write.
    pub counter: u64,
    /// A registration completed and a frame arrived from the server within
    /// [`SERVER_SILENCE_LIMIT`].
    pub connected: bool,
    /// Seconds since the last frame arrived from the server, on the CLI's
    /// monotonic clock; 0 while connected.
    pub unreachable_secs: u64,
    /// The last registration reply echoed `omp_tab_autopilot_v1`.
    pub server_controls_autopilot: bool,
}

/// `omp-autopilot-request.json`: a start or a resume. Strict: the extension
/// rejects unknown fields and a format version it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub v: u32,
    pub launch_id: Uuid,
    pub request_id: String,
    /// Host wall clock in milliseconds, stamped after the CLI took its lock.
    pub issued_at_ms: u64,
    /// `{"kind":"start","spec":{…}}` or `{"kind":"resume","autopilot_id":…}`,
    /// exactly as [`shared::AutopilotCommand`] serializes them.
    pub command: shared::AutopilotCommand,
}

/// What a [`StopRequest`] asks for, weakest first. A later request replaces a
/// pending one only when it is equal or stronger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopAction {
    Pause,
    Stop,
    StopNow,
}

/// `omp-autopilot-stop.json`: a pause, stop or stop now. The extension
/// applies one that passes the trust checks whatever its format version and
/// unknown fields, because a stop must not be refused for a newer writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRequest {
    pub v: u32,
    pub launch_id: Uuid,
    pub request_id: String,
    pub issued_at_ms: u64,
    pub action: StopAction,
    /// A hint only: the request applies to whatever autopilot is current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot_id: Option<Uuid>,
}

/// One request's result in [`AckFile`]. It means only that the extension
/// took the request; the state report says what the tab is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    pub request_id: String,
    /// "accepted" or "refused".
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub at_ms: u64,
}

/// `omp-autopilot-ack.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckFile {
    pub v: u32,
    pub launch_id: Uuid,
    pub pid: i32,
    pub process_start: String,
    #[serde(default)]
    pub acks: Vec<Ack>,
}

/// `omp-autopilot.json`: what the tab reports. Unknown fields are accepted,
/// because a tab's extension can be newer than the CLI reading it. The one
/// strict part is the spec inside `autopilot` and `last_ended`: a CLI that
/// cannot read a newer spec shows the tab as not reported, and Stop still
/// works, instead of showing limits it may have misread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateReport {
    pub v: u32,
    /// Changes on every rewrite, so the CLI can tell a refresh from a stale
    /// file without comparing clocks.
    pub counter: u64,
    pub launch_id: Uuid,
    pub pid: i32,
    pub process_start: String,
    #[serde(default)]
    pub session_id: String,
    /// "supported", or "unsupported_omp" when the tab's OMP is below the
    /// minimum version or lacks a call autopilot uses.
    pub support: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot: Option<shared::AutopilotView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ended: Option<shared::AutopilotEnded>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omp_modes: Option<shared::OmpModesView>,
}

/// Values of [`JournalEntry::kind`].
pub mod journal_kind {
    pub const START: &str = "start";
    pub const ITERATION: &str = "iteration";
    pub const NOT_SUBMITTED: &str = "not_submitted";
    pub const PAUSE: &str = "pause";
    pub const RESUME: &str = "resume";
    pub const END: &str = "end";
}

/// The data of one `apas-autopilot` custom entry in the OMP session.
///
/// The extension appends one at start, at each submission (`iteration`,
/// written just before the prompt is handed to OMP), at each submission OMP
/// did not accept (`not_submitted`, which cancels the `iteration` entry it
/// follows), and at every pause, resume and end. Only the start entry holds
/// the prompt.
///
/// The transcript reader treats a user message as an iteration only when its
/// attribution is `agent` and it pairs with the nearest earlier unpaired
/// `iteration` entry of the same autopilot on the active branch. Text is not
/// compared. Entries whose `session_id` is not the session being read are
/// ignored: a fork copies them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub v: u32,
    pub autopilot_id: Uuid,
    pub session_id: String,
    pub kind: String,
    /// RFC 3339.
    #[serde(default)]
    pub at: String,
    /// `start` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<shared::AutopilotSpec>,
    /// `start` only: RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// `start` only, when the autopilot has a time limit: RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// `iteration` and `not_submitted`: the iteration's number, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u64>,
    /// `pause`, `resume` and `end`: iterations submitted so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u64>,
    /// `pause` and `end`: a reason code from `shared::autopilot_condition`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Refusals the spec words exactly.
pub const NOT_TAKEN_REASON: &str = "The tab did not take the request in time. Nothing was started.";
pub const AGENT_NOT_RUNNING_REASON: &str =
    "This tab's agent is not running. Reboot it first; its autopilot comes back paused.";
pub const NEEDS_AGENT_REBOOT_REASON: &str =
    "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress.";
pub const UNAVAILABLE_ON_PLATFORM_REASON: &str =
    "Autopilot is not available for tabs on this machine.";

/// How one request was answered, as `CliToServer::PaneAutopilotReceipt`
/// carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub outcome: AutopilotReceiptOutcome,
    pub reason: Option<String>,
}

impl Receipt {
    pub fn refused(reason: impl Into<String>) -> Self {
        Self {
            outcome: AutopilotReceiptOutcome::Refused,
            reason: Some(reason.into()),
        }
    }

    fn uncertain() -> Self {
        Self {
            outcome: AutopilotReceiptOutcome::Uncertain,
            reason: None,
        }
    }

    /// The extension's own answer. One this build cannot read is not taken
    /// for either outcome.
    fn of(ack: Ack) -> Self {
        match ack.outcome.as_str() {
            "accepted" => Self {
                outcome: AutopilotReceiptOutcome::Accepted,
                reason: None,
            },
            "refused" => Self::refused(
                ack.reason
                    .unwrap_or_else(|| "The tab refused the request.".into()),
            ),
            _ => Self::uncertain(),
        }
    }
}

/// Why a request is refused before anything is written for its tab, if it
/// is. `agent_running` is false once the tab's agent process has exited:
/// nothing there could apply even a stop, and its autopilot comes back
/// paused when the agent is launched again.
pub fn refusal_before_exchange(
    command: &AutopilotCommand,
    omp_terminal_tab: bool,
    agent_running: bool,
) -> Option<String> {
    if let Err(reason) = command.validate() {
        return Some(reason);
    }
    if !omp_terminal_tab {
        return Some("This tab is not an OMP terminal tab of this project.".into());
    }
    if !cfg!(target_os = "linux") {
        return Some(UNAVAILABLE_ON_PLATFORM_REASON.into());
    }
    if !agent_running {
        return Some(AGENT_NOT_RUNNING_REASON.into());
    }
    None
}

struct Waits {
    lock: Duration,
    ack: Duration,
    taken: Duration,
}

/// Hand one request to the tab's extension and wait for its answer. It
/// blocks for up to [`LOCK_WAIT`] + [`ACK_WAIT`] + [`TAKEN_WAIT`], which
/// stays inside the server's five seconds, so call it off the async path.
pub fn exchange(
    project_id: Uuid,
    pane_id: u32,
    process_group_id: Option<i32>,
    request_id: &str,
    command: &AutopilotCommand,
) -> Receipt {
    let Some(runtime) = pane_runtime_dir(project_id, pane_id) else {
        return Receipt::refused(NEEDS_AGENT_REBOOT_REASON);
    };
    let waits = Waits {
        lock: LOCK_WAIT,
        ack: ACK_WAIT,
        taken: TAKEN_WAIT,
    };
    exchange_at(&runtime, process_group_id, request_id, command, &waits)
}

fn exchange_at(
    runtime: &Path,
    pid: Option<i32>,
    request_id: &str,
    command: &AutopilotCommand,
    waits: &Waits,
) -> Receipt {
    if request_id.is_empty() || request_id.len() > shared::AUTOPILOT_REQUEST_ID_MAX_BYTES {
        return Receipt::refused("The request has no usable id.");
    }
    // A record without the marker belongs to an extension that reads none of
    // these files, so nothing is written for it, a lock file included.
    if autopilot_launch(runtime).is_none() {
        return Receipt::refused(NEEDS_AGENT_REBOOT_REASON);
    }
    let (action, autopilot_id) = match command {
        AutopilotCommand::Start { .. } | AutopilotCommand::Resume { .. } => {
            return start_exchange(runtime, pid, request_id, command, waits);
        }
        AutopilotCommand::Pause { autopilot_id } => (StopAction::Pause, *autopilot_id),
        AutopilotCommand::Stop { autopilot_id, now } if *now => {
            (StopAction::StopNow, *autopilot_id)
        }
        AutopilotCommand::Stop { autopilot_id, .. } => (StopAction::Stop, *autopilot_id),
    };
    stop_exchange(runtime, pid, request_id, action, autopilot_id, waits)
}

fn has_autopilot(launch: &Launch) -> bool {
    launch.features.iter().any(|name| name == LAUNCH_FEATURE)
}

/// The launch a request is written for. The launch record is all a request
/// is built from, so a stop still works when the tab's state is unreadable
/// or stale.
///
/// The exchanges read it again with their lock held. The provider can be
/// launched again while a request waits for that lock, and a request built
/// from the record read before would take the new launch's pending stop for
/// one left behind and remove it.
fn autopilot_launch(runtime: &Path) -> Option<Launch> {
    read_launch(runtime).filter(has_autopilot)
}

/// A pause, stop or stop now. Written unless a stronger one is already
/// pending, then left in place whatever happens: it stays in force until the
/// tab applies it, and is never sent twice. An equal one replaces the pending
/// request, so the tab orders it by when it was last asked for.
fn stop_exchange(
    runtime: &Path,
    pid: Option<i32>,
    request_id: &str,
    action: StopAction,
    autopilot_id: Option<Uuid>,
    waits: &Waits,
) -> Receipt {
    let (launch_id, awaited) = {
        let Ok(_lock) = lock_exchange(runtime, waits.lock) else {
            return Receipt::refused("APAS could not reach this tab's request files. Try again.");
        };
        let Some(launch) = autopilot_launch(runtime) else {
            return Receipt::refused(NEEDS_AGENT_REBOOT_REASON);
        };
        let acks = read_acks(runtime, launch.launch_id, pid);
        if let Some(ack) = acks.iter().find(|ack| ack.request_id == request_id) {
            // A repeated copy of a request the tab already answered.
            return Receipt::of(ack.clone());
        }
        let awaited = match pending_stop(runtime, launch.launch_id, &acks) {
            // A weaker request never displaces a pending one, and a repeated
            // copy of the pending request is not written again, which would
            // move its place among the tab's requests. Either is answered
            // with the pending request's result.
            Some(pending) if pending.action > action || pending.request_id == request_id => {
                pending.request_id
            }
            _ => {
                let request = StopRequest {
                    v: FORMAT_VERSION,
                    launch_id: launch.launch_id,
                    request_id: request_id.to_string(),
                    // Stamped with the lock held, so the tab orders requests
                    // as they were written.
                    issued_at_ms: now_ms(),
                    action,
                    autopilot_id,
                };
                if write_request(&runtime.join(STOP_FILE), &request).is_err() {
                    return Receipt::refused("APAS could not write the request for this tab.");
                }
                request.request_id
            }
        };
        (launch.launch_id, awaited)
    };
    let Some(ack) = wait_for_ack(runtime, launch_id, pid, &awaited, waits.ack) else {
        return Receipt::uncertain();
    };
    if let Ok(_lock) = lock_exchange(runtime, Duration::ZERO) {
        pending_stop(runtime, launch_id, &read_acks(runtime, launch_id, pid));
    }
    Receipt::of(ack)
}

/// A start or a resume. Never left pending: the tab takes it by renaming the
/// file while this wait lasts, or the file is removed and nothing starts.
fn start_exchange(
    runtime: &Path,
    pid: Option<i32>,
    request_id: &str,
    command: &AutopilotCommand,
    waits: &Waits,
) -> Receipt {
    let path = runtime.join(REQUEST_FILE);
    let lock_asked = Instant::now();
    let (launch_id, lock_left) = {
        let Ok(_lock) = lock_exchange(runtime, waits.lock) else {
            return Receipt::refused("APAS could not reach this tab's request files. Try again.");
        };
        // The whole exchange waits for its lock at most once over: what the
        // first wait used is not there for taking the request back.
        let lock_left = waits.lock.saturating_sub(lock_asked.elapsed());
        let Some(launch) = autopilot_launch(runtime) else {
            return Receipt::refused(NEEDS_AGENT_REBOOT_REASON);
        };
        let acks = read_acks(runtime, launch.launch_id, pid);
        if let Some(ack) = acks.iter().find(|ack| ack.request_id == request_id) {
            return Receipt::of(ack.clone());
        }
        pending_stop(runtime, launch.launch_id, &acks);
        // Replacing a start or resume that is still inside its wait would
        // leave its sender unable to tell whether the tab took it. Its sender
        // takes it back when that wait ends, after waiting for this lock at
        // most; one that is older was left behind by a CLI that is gone, and
        // is replaced.
        let in_flight =
            read_private_json_bounded::<StartRequest>(&path, MAX_BYTES).is_some_and(|other| {
                other.launch_id == launch.launch_id
                    && now_ms().abs_diff(other.issued_at_ms)
                        <= (waits.ack + waits.lock).as_millis() as u64
            });
        if in_flight {
            return Receipt::refused(
                "Another start or resume for this tab is still being answered. Try again in a moment.",
            );
        }
        let request = StartRequest {
            v: FORMAT_VERSION,
            launch_id: launch.launch_id,
            request_id: request_id.to_string(),
            issued_at_ms: now_ms(),
            command: command.clone(),
        };
        if write_request(&path, &request).is_err() {
            return Receipt::refused("APAS could not write the request for this tab.");
        }
        (launch.launch_id, lock_left)
    };
    if let Some(ack) = wait_for_ack(runtime, launch_id, pid, request_id, waits.ack) {
        // The tab renames the file away before it answers. Should one ever
        // answer without taking it, the request is still not left behind.
        withdraw(runtime, request_id, Duration::ZERO);
        return Receipt::of(ack);
    }
    if withdraw(runtime, request_id, lock_left) {
        return Receipt::refused(NOT_TAKEN_REASON);
    }
    // The file is gone, so the tab has taken the request. Only when no
    // result follows may a start have been applied without its sender
    // knowing, and the tab's reported state then shows it.
    wait_for_ack(runtime, launch_id, pid, request_id, waits.taken)
        .map(Receipt::of)
        .unwrap_or_else(Receipt::uncertain)
}

/// Take a start or resume back when its wait ends. True means the file still
/// held this request and is removed now, so the tab never took it.
///
/// The lock keeps another start from being written between reading the
/// request and removing it, which would remove that one instead. It is only
/// ever held for a few file operations. Should it be held for longer than
/// `lock_wait` all the same, the request is taken back without it rather than
/// left pending.
fn withdraw(runtime: &Path, request_id: &str, lock_wait: Duration) -> bool {
    let _lock = lock_exchange(runtime, lock_wait).ok();
    let path = runtime.join(REQUEST_FILE);
    read_private_json_bounded::<StartRequest>(&path, MAX_BYTES)
        .is_some_and(|request| request.request_id == request_id)
        && fs::remove_file(&path).is_ok()
}

fn write_request<T: Serialize>(path: &Path, request: &T) -> Result<()> {
    let body = serde_json::to_vec(request)?;
    if body.len() as u64 > MAX_BYTES {
        bail!("autopilot request exceeds the private file size limit");
    }
    write_private_atomic(path, &body)
}

/// The pause or stop that still waits for the tab, if any. Call it with the
/// lock held: a request the tab has acknowledged is no longer pending and is
/// removed here, as is one left from another launch.
fn pending_stop(runtime: &Path, launch_id: Uuid, acks: &[Ack]) -> Option<StopRequest> {
    let path = runtime.join(STOP_FILE);
    let request: StopRequest = read_private_json_bounded(&path, MAX_BYTES)?;
    if request.launch_id == launch_id
        && !acks.iter().any(|ack| ack.request_id == request.request_id)
    {
        return Some(request);
    }
    let _ = fs::remove_file(&path);
    None
}

/// The extension's request results, when the file belongs to this launch's
/// live process. Without the provider's pid nothing can be trusted.
fn read_acks(runtime: &Path, launch_id: Uuid, pid: Option<i32>) -> Vec<Ack> {
    let Some(pid) = pid.filter(|pid| *pid > 0) else {
        return Vec::new();
    };
    read_private_json_bounded::<AckFile>(&runtime.join(ACK_FILE), MAX_BYTES)
        .filter(|file| {
            file.launch_id == launch_id
                && file.pid == pid
                && process_start(pid).as_deref() == Some(file.process_start.as_str())
        })
        .map(|file| file.acks)
        .unwrap_or_default()
}

fn wait_for_ack(
    runtime: &Path,
    launch_id: Uuid,
    pid: Option<i32>,
    request_id: &str,
    wait: Duration,
) -> Option<Ack> {
    let deadline = Instant::now() + wait;
    loop {
        let ack = read_acks(runtime, launch_id, pid)
            .into_iter()
            .find(|ack| ack.request_id == request_id);
        let left = deadline.saturating_duration_since(Instant::now());
        if ack.is_some() || left.is_zero() {
            return ack;
        }
        std::thread::sleep(Duration::from_millis(10).min(left));
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

struct ExchangeLock(fs::File);

impl Drop for ExchangeLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // A provider spawn on another thread can briefly inherit this
            // descriptor, so closing ours alone may not release the flock.
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

/// The lock covers reading and writing the request files, never the wait for
/// an answer: a stop must not be refused because another request for the tab
/// is in flight.
fn lock_exchange(runtime: &Path, wait: Duration) -> Result<ExchangeLock> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options.open(runtime.join(LOCK_FILE))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || !private_owner(&metadata) {
            bail!("insecure autopilot lock");
        }
        let deadline = Instant::now() + wait;
        // The inode stays in place, as the question bridge's lock does.
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!("the autopilot lock is held");
            }
            std::thread::sleep(Duration::from_millis(10).min(left));
        }
        Ok(ExchangeLock(file))
    }
    #[cfg(not(unix))]
    {
        let _ = (options, runtime, wait);
        bail!("autopilot requires a Unix runtime");
    }
}

/// The tab's state report, when it belongs to this launch's live process.
fn read_state(runtime: &Path, launch_id: Uuid, pid: Option<i32>) -> Option<StateReport> {
    let pid = pid.filter(|pid| *pid > 0)?;
    let report: StateReport = read_private_json_bounded(&runtime.join(STATE_FILE), MAX_BYTES)?;
    (report.launch_id == launch_id
        && report.pid == pid
        && process_start(pid).as_deref() == Some(report.process_start.as_str()))
    .then_some(report)
}

/// What one `CliToServer::PaneAutopilotState` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFrame {
    pub support: AutopilotSupport,
    pub state: Option<shared::PaneAutopilotState>,
    pub reported_at: Option<String>,
}

/// What the project runtime knows of a tab's agent process when it polls the
/// tab's autopilot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentProcess {
    /// This CLI holds no runtime for the tab: its agent has not been
    /// launched yet, as in the moments after a tab is created or a project
    /// is started, or could not be.
    NotLaunched,
    /// Launched. It is running as far as this CLI knows, or this CLI asked
    /// it to end: a close, an agent reboot or a project stop.
    Launched,
    /// It ended, and this CLI did not ask for that.
    ExitedByItself,
}

/// One OMP terminal tab's autopilot as the project runtime relays it: what
/// the tab last reported, when that was last seen refreshed, and what was
/// last sent.
#[derive(Default)]
pub struct PaneAutopilotRelay {
    launch_id: Option<Uuid>,
    counter: Option<u64>,
    /// When the report's counter was last seen to change: on the monotonic
    /// clock for the staleness rule, and as the RFC 3339 time that is sent as
    /// `reported_at`. No clock of the tab is compared.
    refreshed: Option<(Instant, String)>,
    /// The autopilot, on or paused, of the state last relayed for this
    /// launch. Every change is relayed, so this is the last one read.
    relayed: Option<shared::AutopilotView>,
    /// The pause this CLI reports on the tab's behalf after its agent process
    /// exited by itself. Kept in memory only.
    exited: Option<shared::PaneAutopilotState>,
    /// What was last sent, and under which registration.
    sent: Option<(u64, AutopilotSupport, Option<shared::PaneAutopilotState>)>,
}

impl PaneAutopilotRelay {
    /// Read the tab's files in its runtime directory and return the frame to
    /// send, if what there is to say changed or the CLI has registered again
    /// since it was last said. `pid` is the provider's process group, when
    /// the CLI knows it; without it no report can be trusted.
    pub fn poll(
        &mut self,
        runtime: Option<&Path>,
        pid: Option<i32>,
        agent: AgentProcess,
        connection_generation: u64,
    ) -> Option<StateFrame> {
        let launch = runtime.and_then(read_launch);
        let report = runtime
            .zip(launch.as_ref())
            .and_then(|(runtime, launch)| read_state(runtime, launch.launch_id, pid));
        let wall = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.observe(
            launch.as_ref(),
            report,
            agent,
            connection_generation,
            Instant::now(),
            &wall,
        )
    }

    fn observe(
        &mut self,
        launch: Option<&Launch>,
        report: Option<StateReport>,
        agent: AgentProcess,
        connection_generation: u64,
        now: Instant,
        wall: &str,
    ) -> Option<StateFrame> {
        let launch_id = launch.map(|launch| launch.launch_id);
        if self.launch_id != launch_id {
            // The agent was launched again. Nothing of the old launch
            // carries over, its pause after an exit included.
            *self = Self {
                launch_id,
                sent: self.sent.take(),
                ..Self::default()
            };
        }
        if let Some(report) = &report {
            if self.counter != Some(report.counter) {
                self.counter = Some(report.counter);
                self.refreshed = Some((now, wall.to_string()));
            }
        }
        // A report that cannot be read now is not reported at once. One that
        // still reads but has stopped changing is not reported once it has
        // not been seen refreshed for the stale bound.
        let fresh = report.filter(|_| {
            self.refreshed
                .as_ref()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) < STATE_STALE_AFTER)
        });
        if agent != AgentProcess::ExitedByItself {
            self.exited = None;
        } else if self.exited.is_none() {
            // Built from what was last relayed: the tab's files fail the
            // trust checks once its process is gone.
            self.exited = self
                .relayed
                .take()
                .map(|autopilot| shared::PaneAutopilotState {
                    autopilot: Some(shared::AutopilotView {
                        next_iteration_at: None,
                        condition: shared::AutopilotCondition {
                            kind: shared::autopilot_condition::PAUSED.into(),
                            detail: Some(shared::autopilot_condition::PAUSE_AGENT_EXITED.into()),
                            since: Some(wall.to_string()),
                        },
                        ..autopilot
                    }),
                    last_ended: None,
                    omp_modes: None,
                    reported_by_cli: true,
                });
        }

        let (support, state) = if !cfg!(target_os = "linux") {
            (AutopilotSupport::UnavailableOnPlatform, None)
        } else if let Some(exited) = &self.exited {
            // In place of "not reported", and exempt from the stale bound.
            (AutopilotSupport::Supported, Some(exited.clone()))
        } else if !launch.is_some_and(has_autopilot) {
            if agent == AgentProcess::NotLaunched {
                // Nothing was launched for the tab yet, so the record, if
                // there is one, is a previous agent's. A tab is created
                // before its agent is launched, and it must not ask for a
                // reboot in between.
                (AutopilotSupport::NotReported, None)
            } else {
                (AutopilotSupport::NeedsAgentReboot, None)
            }
        } else {
            match fresh {
                Some(report) if report.support == "supported" => {
                    self.relayed = report.autopilot.clone();
                    (AutopilotSupport::Supported, Some(state_of(report)))
                }
                Some(report) if report.support == "unsupported_omp" => {
                    (AutopilotSupport::UnsupportedOmp, Some(state_of(report)))
                }
                // No fresh report, or one whose support value this build
                // does not know.
                _ => (AutopilotSupport::NotReported, None),
            }
        };

        let unchanged = self
            .sent
            .as_ref()
            .is_some_and(|(generation, sent_support, sent)| {
                *generation == connection_generation && *sent_support == support && *sent == state
            });
        if unchanged {
            return None;
        }
        self.sent = Some((connection_generation, support, state.clone()));
        Some(StateFrame {
            support,
            state,
            reported_at: self.refreshed.as_ref().map(|(_, wall)| wall.clone()),
        })
    }
}

fn state_of(report: StateReport) -> shared::PaneAutopilotState {
    shared::PaneAutopilotState {
        autopilot: report.autopilot,
        last_ended: report.last_ended,
        omp_modes: report.omp_modes,
        reported_by_cli: false,
    }
}

/// What the project runtime knows about its connection to the server, for
/// the heartbeat. The connection task records it; nothing here is inferred
/// from the socket staying open, because a silent network drop reads as
/// connected until the kernel gives up on it.
pub struct ServerLink {
    connected: AtomicBool,
    controls_autopilot: AtomicBool,
    /// When the last frame arrived from the server, once one has.
    last_frame: Mutex<Option<Instant>>,
}

/// `unreachable_secs` of a runtime that has not heard from the server yet.
/// It cannot say for how long the server has been out of reach: the outage
/// may be older than this process, as after a daemon restart in the middle
/// of one. Counting from its own start would end a tab's hold, and let it
/// submit for ten more minutes with nothing able to see or stop it. So such
/// a runtime says "longer than any hold threshold" until its first
/// registration, which on a reachable server is a moment after it starts.
const NEVER_HEARD_SECS: u64 = u32::MAX as u64;

impl ServerLink {
    pub fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            controls_autopilot: AtomicBool::new(false),
            last_frame: Mutex::new(None),
        }
    }

    /// The top of each reconnect attempt.
    pub fn reconnecting(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }

    /// A registration completed. `negotiated` is what the server's reply
    /// echoed; a server that does not echo the capability can relay neither
    /// autopilot state nor controls.
    pub fn registered(&self, negotiated: &[String]) {
        self.controls_autopilot.store(
            negotiated
                .iter()
                .any(|capability| capability == shared::OMP_TAB_AUTOPILOT_CAPABILITY),
            Ordering::SeqCst,
        );
        self.frame_arrived();
        self.connected.store(true, Ordering::SeqCst);
    }

    /// Whether the server of the last registration relays autopilot. One that
    /// does not cannot parse a state frame, and logs each one it is sent.
    pub fn controls_autopilot(&self) -> bool {
        self.controls_autopilot.load(Ordering::SeqCst)
    }

    /// Any frame arrived from the server.
    pub fn frame_arrived(&self) {
        *self
            .last_frame
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
    }

    /// `connected`, `unreachable_secs` and `server_controls_autopilot` of a
    /// heartbeat written at `now`.
    fn status(&self, now: Instant) -> (bool, u64, bool) {
        let silence = self
            .last_frame
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .map(|at| now.saturating_duration_since(at));
        let connected = self.connected.load(Ordering::SeqCst)
            && silence.is_some_and(|silence| silence <= SERVER_SILENCE_LIMIT);
        let unreachable_secs = match silence {
            _ if connected => 0,
            Some(silence) => silence.as_secs(),
            None => NEVER_HEARD_SECS,
        };
        (
            connected,
            unreachable_secs,
            self.controls_autopilot.load(Ordering::SeqCst),
        )
    }
}

/// Chosen at random when this process first writes a heartbeat. A replaced
/// CLI process writes another, which the extension sees as a change at once.
static WRITER: OnceLock<Uuid> = OnceLock::new();
/// One counter for every heartbeat this process writes, so it never repeats
/// when a project is stopped and started inside the same process.
static HEARTBEATS: AtomicU64 = AtomicU64::new(0);

fn write_heartbeat(runtime: &Path, link: &ServerLink, now: Instant) -> Option<()> {
    let launch = autopilot_launch(runtime)?;
    let (connected, unreachable_secs, server_controls_autopilot) = link.status(now);
    let heartbeat = ControllerHeartbeat {
        v: FORMAT_VERSION,
        launch_id: launch.launch_id,
        writer: *WRITER.get_or_init(Uuid::new_v4),
        counter: HEARTBEATS.fetch_add(1, Ordering::Relaxed) + 1,
        connected,
        unreachable_secs,
        server_controls_autopilot,
    };
    let body = serde_json::to_vec(&heartbeat).ok()?;
    write_private_atomic(&runtime.join(CONTROLLER_FILE), &body).ok()
}

/// Write the heartbeat of every runtime directory `runtimes` names, every
/// `interval`, until `shutdown`. It runs on a thread of its own and has no
/// way to send to the server: the project's outbound queue fills during an
/// outage, and a writer waiting on it would make every tab look abandoned.
pub fn run_controller_heartbeat(
    runtimes: impl Fn() -> Vec<PathBuf>,
    link: &ServerLink,
    shutdown: &AtomicBool,
    interval: Duration,
) {
    // The first one waits an interval. A runtime that has just started has
    // not registered yet, and would tell a tab it took over from the CLI
    // before it that the server is out of reach, a moment before it is not.
    // The heartbeat that CLI left behind stays fresh far longer than this.
    loop {
        std::thread::sleep(interval);
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        for runtime in runtimes() {
            // A tab without a launch record, or whose extension predates
            // autopilot, reads no heartbeat and gets none.
            let _ = write_heartbeat(&runtime, link, Instant::now());
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::AgentProcess::{ExitedByItself, Launched, NotLaunched};
    use super::*;
    use crate::omp_activity::install;
    use shared::{autopilot_condition, AutopilotLimit, AutopilotRepeat, AutopilotSpec};
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::Arc;
    use std::thread::JoinHandle;

    /// Short waits, for the paths in which the wait itself runs out.
    const WAITS: Waits = Waits {
        lock: Duration::from_millis(200),
        ack: Duration::from_millis(400),
        taken: Duration::from_millis(300),
    };
    /// Long ones, for the paths in which the tab answers: a wait ends as soon
    /// as the answer is there, and a loaded machine must not turn an answer
    /// into a timeout.
    const PATIENT: Waits = Waits {
        lock: Duration::from_secs(5),
        ack: Duration::from_secs(20),
        taken: Duration::from_secs(20),
    };

    /// A pane's runtime directory after a launch, with a live process
    /// standing in for the tab's OMP.
    struct Tab {
        root: tempfile::TempDir,
        runtime: PathBuf,
        launch_id: Uuid,
        provider: Child,
        process_start: String,
    }

    impl Tab {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let runtime = root.path().join("runtime");
            crate::pane_host::ensure_private_dir(&runtime).unwrap();
            fs::create_dir(root.path().join("sessions")).unwrap();
            let (_, launch) = install(&runtime, &root.path().join("sessions")).unwrap();
            let provider = Command::new("sleep")
                .arg("60")
                .process_group(0)
                .spawn()
                .unwrap();
            let process_start = process_start(provider.id() as i32).unwrap();
            Self {
                root,
                runtime,
                launch_id: launch.launch_id,
                provider,
                process_start,
            }
        }

        fn pid(&self) -> i32 {
            self.provider.id() as i32
        }

        fn path(&self, name: &str) -> PathBuf {
            self.runtime.join(name)
        }

        fn exchange(&self, request_id: &str, command: &AutopilotCommand) -> Receipt {
            exchange_at(&self.runtime, Some(self.pid()), request_id, command, &WAITS)
        }

        fn answered(&self, request_id: &str, command: &AutopilotCommand) -> Receipt {
            exchange_at(
                &self.runtime,
                Some(self.pid()),
                request_id,
                command,
                &PATIENT,
            )
        }

        /// The extension's results so far, as it writes them.
        fn acks(&self, acks: Vec<Ack>) -> AckFile {
            AckFile {
                v: FORMAT_VERSION,
                launch_id: self.launch_id,
                pid: self.pid(),
                process_start: self.process_start.clone(),
                acks,
            }
        }

        fn write_acks(&self, file: &AckFile) {
            write_private_atomic(&self.path(ACK_FILE), &serde_json::to_vec(file).unwrap()).unwrap();
        }

        /// The extension's side of one exchange, on a thread of its own: wait
        /// for `file`, take it by renaming when `take`, wait `delay`, then
        /// answer with `outcome` if there is one. Returns the request read.
        fn respond(
            &self,
            file: &'static str,
            take: bool,
            delay: Duration,
            outcome: Option<(&'static str, Option<&'static str>)>,
        ) -> JoinHandle<serde_json::Value> {
            let runtime = self.runtime.clone();
            let pid = self.pid();
            let mut acks = self.acks(Vec::new());
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                let request = loop {
                    if let Some(request) = read_private_json_bounded::<serde_json::Value>(
                        &runtime.join(file),
                        MAX_BYTES,
                    ) {
                        break request;
                    }
                    assert!(Instant::now() < deadline, "no request reached the tab");
                    std::thread::sleep(Duration::from_millis(1));
                };
                if take {
                    fs::rename(
                        runtime.join(file),
                        runtime.join(format!("omp-autopilot-request.taken-{pid}")),
                    )
                    .unwrap();
                }
                std::thread::sleep(delay);
                if let Some((outcome, reason)) = outcome {
                    acks.acks.push(ack(
                        request["request_id"].as_str().unwrap(),
                        outcome,
                        reason,
                    ));
                    write_private_atomic(
                        &runtime.join(ACK_FILE),
                        &serde_json::to_vec(&acks).unwrap(),
                    )
                    .unwrap();
                }
                request
            })
        }

        fn report(&self, counter: u64, autopilot: Option<shared::AutopilotView>) -> StateReport {
            StateReport {
                v: FORMAT_VERSION,
                counter,
                launch_id: self.launch_id,
                pid: self.pid(),
                process_start: self.process_start.clone(),
                session_id: "01a1d594-4d99-729d-a0a1-0d838bc273a9".into(),
                support: "supported".into(),
                unsupported_reason: None,
                autopilot,
                last_ended: None,
                omp_modes: None,
            }
        }

        fn write_state(&self, report: &StateReport) {
            write_private_atomic(&self.path(STATE_FILE), &serde_json::to_vec(report).unwrap())
                .unwrap();
        }

        fn launch(&self) -> Launch {
            read_launch(&self.runtime).unwrap()
        }

        /// The agent is launched again in this runtime directory. The same
        /// process goes on standing in for it.
        fn relaunch(&mut self) -> Uuid {
            let (_, launch) = install(&self.runtime, &self.root.path().join("sessions")).unwrap();
            self.launch_id = launch.launch_id;
            launch.launch_id
        }

        /// The record an APAS from before autopilot wrote for this launch.
        fn drop_the_launch_marker(&self) {
            let mut record: serde_json::Value =
                serde_json::from_slice(&fs::read(self.path("omp-launch.json")).unwrap()).unwrap();
            record.as_object_mut().unwrap().remove("features");
            write_private_atomic(
                &self.path("omp-launch.json"),
                &serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for Tab {
        fn drop(&mut self) {
            let _ = self.provider.kill();
            let _ = self.provider.wait();
        }
    }

    fn ack(request_id: &str, outcome: &str, reason: Option<&str>) -> Ack {
        Ack {
            request_id: request_id.into(),
            outcome: outcome.into(),
            reason: reason.map(str::to_string),
            at_ms: now_ms(),
        }
    }

    fn spec() -> AutopilotSpec {
        AutopilotSpec {
            prompt: "Check the build and fix what fails.".into(),
            repeat: AutopilotRepeat::AfterEachTurn,
            iteration_limit: AutopilotLimit::AtMost { value: 20 },
            time_limit: AutopilotLimit::None,
            end_without_tool_call: true,
        }
    }

    fn start() -> AutopilotCommand {
        AutopilotCommand::Start { spec: spec() }
    }

    fn stop(now: bool) -> AutopilotCommand {
        AutopilotCommand::Stop {
            autopilot_id: None,
            now,
        }
    }

    fn pause() -> AutopilotCommand {
        AutopilotCommand::Pause { autopilot_id: None }
    }

    fn view(kind: &str, detail: Option<&str>) -> shared::AutopilotView {
        shared::AutopilotView {
            autopilot_id: Uuid::from_u128(7),
            spec: spec(),
            started_at: "2026-11-13T14:00:00Z".into(),
            iterations: 7,
            deadline: None,
            next_iteration_at: Some("2026-11-13T14:30:00Z".into()),
            condition: shared::AutopilotCondition {
                kind: kind.into(),
                detail: detail.map(str::to_string),
                since: Some("2026-11-13T14:15:00Z".into()),
            },
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_stop_is_built_from_the_launch_record_and_answered_by_the_tab() {
        let tab = Tab::new();
        // No state report exists: the launch record is all a stop needs.
        assert!(!tab.path(STATE_FILE).exists());
        let before = now_ms();
        let responder = tab.respond(STOP_FILE, false, Duration::ZERO, Some(("accepted", None)));
        let receipt = tab.answered("stop-1", &stop(true));
        let request = responder.join().unwrap();
        assert_eq!(
            receipt,
            Receipt {
                outcome: AutopilotReceiptOutcome::Accepted,
                reason: None
            }
        );
        assert_eq!(request["v"], FORMAT_VERSION);
        assert_eq!(request["launch_id"], serde_json::json!(tab.launch_id));
        assert_eq!(request["request_id"], "stop-1");
        assert_eq!(request["action"], "stop_now");
        assert!(request.get("autopilot_id").is_none());
        let issued = request["issued_at_ms"].as_u64().unwrap();
        assert!((before..=now_ms()).contains(&issued));
        // Acknowledged, so it is no longer pending and was removed with the
        // lock held. The lock's inode stays.
        assert!(!tab.path(STOP_FILE).exists());
        assert_eq!(mode(&tab.path(LOCK_FILE)), 0o600);

        let refused = tab.respond(
            STOP_FILE,
            false,
            Duration::ZERO,
            Some(("refused", Some("This autopilot is no longer current."))),
        );
        let receipt = tab.answered("pause-1", &pause());
        assert_eq!(refused.join().unwrap()["action"], "pause");
        assert_eq!(
            receipt,
            Receipt::refused("This autopilot is no longer current.")
        );
    }

    #[test]
    fn an_unanswered_stop_is_uncertain_stays_in_force_and_is_never_resent() {
        let tab = Tab::new();
        let receipt = tab.exchange("stop-1", &stop(false));
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Uncertain);
        let written = fs::read(tab.path(STOP_FILE)).unwrap();
        assert!(written.len() as u64 <= MAX_BYTES);
        assert_eq!(mode(&tab.path(STOP_FILE)), 0o600);
        // Nothing rewrites it once the wait has ended.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(fs::read(tab.path(STOP_FILE)).unwrap(), written);
        // A repeated copy of the request, while it still waits for the tab,
        // waits with it. Written again it would carry a later issue time,
        // and the tab would order it after a start sent in between.
        assert_eq!(
            tab.exchange("stop-1", &stop(false)).outcome,
            AutopilotReceiptOutcome::Uncertain
        );
        assert_eq!(fs::read(tab.path(STOP_FILE)).unwrap(), written);

        // The tab applies it later. The next time the lock is held, the
        // acknowledged request is removed; here by an unrelated start.
        tab.write_acks(&tab.acks(vec![ack("stop-1", "accepted", None)]));
        assert_eq!(
            tab.exchange("start-1", &start()),
            Receipt::refused(NOT_TAKEN_REASON)
        );
        assert!(!tab.path(STOP_FILE).exists());
        // A repeated copy of the answered request is answered from the
        // tab's result and writes nothing.
        assert_eq!(
            tab.exchange("stop-1", &stop(false)).outcome,
            AutopilotReceiptOutcome::Accepted
        );
        assert!(!tab.path(STOP_FILE).exists());
    }

    #[test]
    fn a_weaker_request_never_displaces_a_pending_stop() {
        let tab = Tab::new();
        assert_eq!(
            tab.exchange("stop-1", &stop(false)).outcome,
            AutopilotReceiptOutcome::Uncertain
        );
        let pending = |tab: &Tab| -> StopRequest {
            read_private_json_bounded(&tab.path(STOP_FILE), MAX_BYTES).unwrap()
        };
        // A pause is weaker: it is not written, and is answered with the
        // pending stop's result, which has not arrived.
        assert_eq!(
            tab.exchange("pause-1", &pause()).outcome,
            AutopilotReceiptOutcome::Uncertain
        );
        assert_eq!(pending(&tab).request_id, "stop-1");
        assert_eq!(pending(&tab).action, StopAction::Stop);

        // An equal request replaces it, and so does a stronger one.
        assert_eq!(
            tab.exchange("stop-2", &stop(false)).outcome,
            AutopilotReceiptOutcome::Uncertain
        );
        assert_eq!(pending(&tab).request_id, "stop-2");
        assert_eq!(
            tab.exchange("stop-now-1", &stop(true)).outcome,
            AutopilotReceiptOutcome::Uncertain
        );
        assert_eq!(pending(&tab).request_id, "stop-now-1");
        assert_eq!(pending(&tab).action, StopAction::StopNow);

        // A stop arrives while the stop now is still pending. It is weaker,
        // and gets the stop now's result as its own once the tab applies
        // that. The tab answers late enough for the stop to be waiting.
        let responder = tab.respond(
            STOP_FILE,
            false,
            Duration::from_secs(1),
            Some(("accepted", None)),
        );
        let receipt = tab.answered("stop-3", &stop(false));
        assert_eq!(responder.join().unwrap()["request_id"], "stop-now-1");
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Accepted);
        assert!(StopAction::Pause < StopAction::Stop && StopAction::Stop < StopAction::StopNow);
    }

    #[test]
    fn a_start_the_tab_did_not_take_is_removed_and_refused() {
        let tab = Tab::new();
        let started = Instant::now();
        let receipt = tab.exchange("start-1", &start());
        assert!(started.elapsed() >= WAITS.ack);
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Refused);
        assert_eq!(
            receipt.reason.as_deref(),
            Some("The tab did not take the request in time. Nothing was started.")
        );
        // Never left pending.
        assert!(!tab.path(REQUEST_FILE).exists());

        let resume = AutopilotCommand::Resume {
            autopilot_id: Uuid::from_u128(7),
        };
        assert_eq!(
            tab.exchange("resume-1", &resume),
            Receipt::refused(NOT_TAKEN_REASON)
        );
        assert!(!tab.path(REQUEST_FILE).exists());
    }

    #[test]
    fn a_start_the_tab_took_is_answered_with_its_result() {
        let tab = Tab::new();
        let before = now_ms();
        let responder = tab.respond(REQUEST_FILE, true, Duration::ZERO, Some(("accepted", None)));
        let receipt = tab.answered("start-1", &start());
        let request = responder.join().unwrap();
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Accepted);
        assert!(!tab.path(REQUEST_FILE).exists());
        // The file is the strict form the extension parses.
        let request: StartRequest = serde_json::from_value(request).unwrap();
        assert_eq!(request.v, FORMAT_VERSION);
        assert_eq!(request.launch_id, tab.launch_id);
        assert_eq!(request.request_id, "start-1");
        assert_eq!(request.command, start());
        assert!((before..=now_ms()).contains(&request.issued_at_ms));

        let refused = tab.respond(
            REQUEST_FILE,
            true,
            Duration::ZERO,
            Some((
                "refused",
                Some("This tab is already on autopilot. Stop it first."),
            )),
        );
        let receipt = tab.answered("start-2", &start());
        refused.join().unwrap();
        assert_eq!(
            receipt,
            Receipt::refused("This tab is already on autopilot. Stop it first.")
        );
    }

    #[test]
    fn a_start_taken_at_the_end_of_the_wait_is_answered_or_left_uncertain() {
        // The file is gone when the wait ends, so the tab has it. Its result
        // arrives in the extra second.
        let tab = Tab::new();
        // The tab takes the request at once, well inside the first wait, and
        // answers after that wait has ended.
        let waits = Waits {
            ack: Duration::from_millis(1500),
            ..PATIENT
        };
        let responder = tab.respond(
            REQUEST_FILE,
            true,
            Duration::from_secs(3),
            Some(("accepted", None)),
        );
        let started = Instant::now();
        let receipt = exchange_at(&tab.runtime, Some(tab.pid()), "start-1", &start(), &waits);
        responder.join().unwrap();
        assert!(started.elapsed() > waits.ack);
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Accepted);

        // No result follows: only then may a start have been applied without
        // its sender knowing.
        let waits = Waits {
            taken: WAITS.taken,
            ..waits
        };
        let silent = tab.respond(REQUEST_FILE, true, Duration::ZERO, None);
        let receipt = exchange_at(&tab.runtime, Some(tab.pid()), "start-2", &start(), &waits);
        silent.join().unwrap();
        assert_eq!(
            receipt,
            Receipt {
                outcome: AutopilotReceiptOutcome::Uncertain,
                reason: None
            }
        );
        assert!(!tab.path(REQUEST_FILE).exists());
    }

    #[test]
    fn a_stop_is_taken_while_another_request_for_the_tab_is_in_flight() {
        let tab = Arc::new(Tab::new());
        // The start waits long for a tab that never takes it. Were the lock
        // held for that wait, the stop could not be written.
        const WAITS: Waits = Waits {
            lock: Duration::from_millis(100),
            ack: Duration::from_secs(4),
            taken: Duration::ZERO,
        };
        let starting = {
            let tab = tab.clone();
            std::thread::spawn(move || {
                exchange_at(&tab.runtime, Some(tab.pid()), "start-1", &start(), &WAITS)
            })
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !tab.path(REQUEST_FILE).exists() {
            assert!(Instant::now() < deadline, "the start was never written");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(mode(&tab.path(REQUEST_FILE)), 0o600);
        let responder = tab.respond(STOP_FILE, false, Duration::ZERO, Some(("accepted", None)));
        let receipt = exchange_at(
            &tab.runtime,
            Some(tab.pid()),
            "stop-1",
            &stop(false),
            &WAITS,
        );
        responder.join().unwrap();
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Accepted);

        // A second start is not written over the one still being answered.
        let second = exchange_at(&tab.runtime, Some(tab.pid()), "start-2", &start(), &WAITS);
        assert_eq!(second.outcome, AutopilotReceiptOutcome::Refused);
        assert_ne!(second.reason.as_deref(), Some(NOT_TAKEN_REASON));
        let waiting: StartRequest =
            read_private_json_bounded(&tab.path(REQUEST_FILE), MAX_BYTES).unwrap();
        assert_eq!(waiting.request_id, "start-1");

        assert_eq!(starting.join().unwrap(), Receipt::refused(NOT_TAKEN_REASON));
    }

    #[test]
    fn a_start_is_taken_back_under_the_lock_and_never_takes_another_one_with_it() {
        let tab = Tab::new();
        let waiting = |request_id: &str, issued_ago: Duration| {
            let request = StartRequest {
                v: FORMAT_VERSION,
                launch_id: tab.launch_id,
                request_id: request_id.into(),
                issued_at_ms: now_ms() - issued_ago.as_millis() as u64,
                command: start(),
            };
            write_request(&tab.path(REQUEST_FILE), &request).unwrap();
        };
        let held = || -> Option<StartRequest> {
            read_private_json_bounded(&tab.path(REQUEST_FILE), MAX_BYTES)
        };

        // Another start is in the file by now: it is not this one's to remove.
        waiting("start-2", Duration::ZERO);
        assert!(!withdraw(&tab.runtime, "start-1", Duration::ZERO));
        assert_eq!(held().unwrap().request_id, "start-2");
        assert!(withdraw(&tab.runtime, "start-2", Duration::ZERO));
        assert_eq!(held(), None);
        // Nothing to take back: the tab has it.
        assert!(!withdraw(&tab.runtime, "start-2", Duration::ZERO));

        // The lock is waited for, and a request is never left pending for
        // want of it.
        waiting("start-3", Duration::ZERO);
        let lock = lock_exchange(&tab.runtime, Duration::ZERO).unwrap();
        let started = Instant::now();
        assert!(withdraw(&tab.runtime, "start-3", WAITS.lock));
        assert!(started.elapsed() >= WAITS.lock);
        drop(lock);

        // A start whose sender may still be about to take it back is not
        // replaced: that sender waits for the lock after its own wait.
        let waits = Waits {
            lock: Duration::from_secs(2),
            ..WAITS
        };
        let another = || exchange_at(&tab.runtime, Some(tab.pid()), "start-5", &start(), &waits);
        waiting("start-4", waits.ack + Duration::from_millis(100));
        let receipt = another();
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Refused);
        assert_ne!(receipt.reason.as_deref(), Some(NOT_TAKEN_REASON));
        assert_eq!(held().unwrap().request_id, "start-4");
        // One left behind by a CLI that is gone is replaced.
        waiting("start-4", waits.ack + waits.lock + Duration::from_secs(1));
        assert_eq!(another(), Receipt::refused(NOT_TAKEN_REASON));
        assert_eq!(held(), None);
    }

    #[test]
    fn a_request_is_written_for_the_launch_that_is_current_when_the_lock_is_held() {
        let mut tab = Tab::new();
        let (runtime, pid) = (tab.runtime.clone(), tab.pid());
        let waiting = move |request_id: &'static str, command: AutopilotCommand| {
            let runtime = runtime.clone();
            std::thread::spawn(move || {
                exchange_at(&runtime, Some(pid), request_id, &command, &PATIENT)
            })
        };

        // A stop has read the launch record and waits for the lock.
        let lock = lock_exchange(&tab.runtime, Duration::ZERO).unwrap();
        let stopping = waiting("stop-old", stop(false));
        std::thread::sleep(Duration::from_millis(300));
        // Meanwhile the agent is launched again, and a stop now is left for
        // the new launch.
        let old_launch = tab.launch_id;
        assert_ne!(tab.relaunch(), old_launch);
        let newer = StopRequest {
            v: FORMAT_VERSION,
            launch_id: tab.launch_id,
            request_id: "stop-now-new".into(),
            issued_at_ms: now_ms(),
            action: StopAction::StopNow,
            autopilot_id: None,
        };
        write_request(&tab.path(STOP_FILE), &newer).unwrap();
        // The new agent applies it a moment after the lock is free.
        let responder = tab.respond(
            STOP_FILE,
            false,
            Duration::from_secs(1),
            Some(("accepted", None)),
        );
        drop(lock);
        // The waiting stop is weaker than the one pending for the launch it
        // now finds. It neither removes that one as left behind nor writes
        // itself for a launch that is gone: it gets that one's result.
        assert_eq!(
            stopping.join().unwrap().outcome,
            AutopilotReceiptOutcome::Accepted
        );
        assert_eq!(
            serde_json::from_value::<StopRequest>(responder.join().unwrap()).unwrap(),
            newer
        );

        // A start that waited the same way is written for the new launch,
        // whose agent takes it and whose answer is trusted.
        let lock = lock_exchange(&tab.runtime, Duration::ZERO).unwrap();
        let starting = waiting("start-old", start());
        std::thread::sleep(Duration::from_millis(300));
        let relaunched = tab.relaunch();
        let responder = tab.respond(REQUEST_FILE, true, Duration::ZERO, Some(("accepted", None)));
        drop(lock);
        assert_eq!(
            starting.join().unwrap().outcome,
            AutopilotReceiptOutcome::Accepted
        );
        assert_eq!(
            responder.join().unwrap()["launch_id"],
            serde_json::json!(relaunched)
        );
    }

    #[test]
    fn the_lock_is_waited_for_briefly_and_never_followed_through_a_symlink() {
        let tab = Tab::new();
        let held = lock_exchange(&tab.runtime, Duration::ZERO).unwrap();
        let started = Instant::now();
        let receipt = tab.exchange("stop-1", &stop(false));
        assert!(started.elapsed() >= WAITS.lock);
        // Refused, not uncertain: nothing was written, so nothing is in force.
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Refused);
        assert!(!tab.path(STOP_FILE).exists());
        drop(held);
        assert!(lock_exchange(&tab.runtime, Duration::ZERO).is_ok());

        fs::remove_file(tab.path(LOCK_FILE)).unwrap();
        let outside = tab.root.path().join("outside.lock");
        fs::write(&outside, b"").unwrap();
        symlink(&outside, tab.path(LOCK_FILE)).unwrap();
        assert!(lock_exchange(&tab.runtime, Duration::ZERO).is_err());
    }

    #[test]
    fn an_acknowledgement_is_trusted_only_from_this_launchs_live_process() {
        let tab = Tab::new();
        let trusted = tab.acks(vec![ack("stop-1", "accepted", None)]);
        let read = |tab: &Tab| read_acks(&tab.runtime, tab.launch_id, Some(tab.pid()));
        tab.write_acks(&trusted);
        assert_eq!(read(&tab).len(), 1);

        // Another launch, another process, a reused pid.
        for forged in [
            AckFile {
                launch_id: Uuid::new_v4(),
                ..trusted.clone()
            },
            AckFile {
                pid: tab.pid() + 1,
                ..trusted.clone()
            },
            AckFile {
                process_start: format!("{}0", tab.process_start),
                ..trusted.clone()
            },
        ] {
            tab.write_acks(&forged);
            assert!(read(&tab).is_empty());
            // An untrusted result answers nothing: the stop stays uncertain.
            assert_eq!(
                tab.exchange("stop-1", &stop(false)).outcome,
                AutopilotReceiptOutcome::Uncertain
            );
        }

        // Without the provider's pid nothing can be checked.
        tab.write_acks(&trusted);
        assert!(read_acks(&tab.runtime, tab.launch_id, None).is_empty());

        // Not private, not a regular file, too large.
        fs::set_permissions(tab.path(ACK_FILE), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read(&tab).is_empty());
        fs::remove_file(tab.path(ACK_FILE)).unwrap();
        let outside = tab.root.path().join("ack.json");
        write_private_atomic(&outside, &serde_json::to_vec(&trusted).unwrap()).unwrap();
        symlink(&outside, tab.path(ACK_FILE)).unwrap();
        assert!(read(&tab).is_empty());
        fs::remove_file(tab.path(ACK_FILE)).unwrap();
        let mut padded = serde_json::to_vec(&trusted).unwrap();
        padded.resize(MAX_BYTES as usize + 1, b' ');
        write_private_atomic(&tab.path(ACK_FILE), &padded).unwrap();
        assert!(read(&tab).is_empty());

        // A result this build cannot read is neither outcome.
        assert_eq!(
            Receipt::of(ack("stop-1", "deferred", None)).outcome,
            AutopilotReceiptOutcome::Uncertain
        );
    }

    #[test]
    fn a_request_is_refused_before_anything_is_written_when_the_tab_cannot_take_it() {
        let tab = Tab::new();
        // A tab launched by an APAS from before autopilot reads none of
        // these files.
        tab.drop_the_launch_marker();
        for command in [start(), stop(true)] {
            assert_eq!(
                tab.exchange("request-1", &command),
                Receipt::refused(
                    "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress."
                )
            );
        }
        assert!(!tab.path(REQUEST_FILE).exists() && !tab.path(STOP_FILE).exists());

        let empty = AutopilotCommand::Start {
            spec: AutopilotSpec {
                prompt: " ".into(),
                ..spec()
            },
        };
        assert_eq!(
            refusal_before_exchange(&empty, true, true).as_deref(),
            Some("Autopilot needs a prompt.")
        );
        assert!(refusal_before_exchange(&start(), false, true).is_some());
        // The tab's agent process has exited: nothing could apply even a stop.
        for command in [start(), pause(), stop(false), stop(true)] {
            assert_eq!(
                refusal_before_exchange(&command, true, false).as_deref(),
                Some("This tab's agent is not running. Reboot it first; its autopilot comes back paused.")
            );
        }
        assert_eq!(refusal_before_exchange(&start(), true, true), None);
        assert_eq!(refusal_before_exchange(&stop(true), true, true), None);

        let tab = Tab::new();
        assert_eq!(
            tab.exchange("", &stop(false)).outcome,
            AutopilotReceiptOutcome::Refused
        );
        let long = "r".repeat(shared::AUTOPILOT_REQUEST_ID_MAX_BYTES + 1);
        assert_eq!(
            tab.exchange(&long, &stop(false)).outcome,
            AutopilotReceiptOutcome::Refused
        );
        assert!(!tab.path(STOP_FILE).exists());
    }

    #[test]
    fn the_longest_prompt_fits_the_request_file() {
        let tab = Tab::new();
        // Newlines are the prompt's worst case: each one doubles when encoded.
        let command = AutopilotCommand::Start {
            spec: AutopilotSpec {
                prompt: format!("x{}", "\n".repeat(shared::AUTOPILOT_PROMPT_MAX_BYTES - 1)),
                ..spec()
            },
        };
        assert_eq!(command.validate(), Ok(()));
        let responder = tab.respond(REQUEST_FILE, true, Duration::ZERO, Some(("accepted", None)));
        let receipt = tab.answered(
            &"r".repeat(shared::AUTOPILOT_REQUEST_ID_MAX_BYTES),
            &command,
        );
        let request = responder.join().unwrap();
        assert_eq!(receipt.outcome, AutopilotReceiptOutcome::Accepted);
        assert!(serde_json::to_vec(&request).unwrap().len() as u64 <= MAX_BYTES);
        assert!(write_request(&tab.path(REQUEST_FILE), &"x".repeat(MAX_BYTES as usize)).is_err());
        assert!(!tab.path(REQUEST_FILE).exists());
    }

    #[test]
    fn support_is_derived_from_the_launch_marker_and_the_report() {
        let tab = Tab::new();
        let mut relay = PaneAutopilotRelay::default();
        let mut poll = |tab: &Tab, generation| {
            relay.poll(Some(&tab.runtime), Some(tab.pid()), Launched, generation)
        };

        // Just launched with the marker: not reported, never a reboot.
        let frame = poll(&tab, 1).unwrap();
        assert_eq!(frame.support, AutopilotSupport::NotReported);
        assert_eq!((frame.state, frame.reported_at), (None, None));
        assert_eq!(poll(&tab, 1), None);

        // The report may be newer than this build: unknown fields are read
        // past, here in the report and in its condition.
        let mut report =
            serde_json::to_value(tab.report(1, Some(view("waiting", Some("settle"))))).unwrap();
        report["future_field"] = serde_json::json!({ "a": 1 });
        report["autopilot"]["condition"]["extra"] = serde_json::json!(true);
        write_private_atomic(&tab.path(STATE_FILE), &serde_json::to_vec(&report).unwrap()).unwrap();
        let frame = poll(&tab, 1).unwrap();
        assert_eq!(frame.support, AutopilotSupport::Supported);
        let state = frame.state.unwrap();
        assert_eq!(state.autopilot, Some(view("waiting", Some("settle"))));
        assert!(!state.reported_by_cli);
        assert!(frame.reported_at.is_some());

        // Its spec is the strict part: one this build cannot read is shown
        // as not reported, where Stop still works, not with limits it may
        // have misread.
        report["autopilot"]["spec"]["token_limit"] = serde_json::json!(1000);
        report["counter"] = serde_json::json!(2);
        write_private_atomic(&tab.path(STATE_FILE), &serde_json::to_vec(&report).unwrap()).unwrap();
        let frame = poll(&tab, 1).unwrap();
        assert_eq!(
            (frame.support, frame.state),
            (AutopilotSupport::NotReported, None)
        );

        let mut unsupported = tab.report(3, None);
        unsupported.support = "unsupported_omp".into();
        unsupported.unsupported_reason = Some("OMP 18.17.0 is below 18.18.2".into());
        tab.write_state(&unsupported);
        assert_eq!(
            poll(&tab, 1).unwrap().support,
            AutopilotSupport::UnsupportedOmp
        );

        let mut unknown = tab.report(4, None);
        unknown.support = "supported_with_tokens".into();
        tab.write_state(&unknown);
        assert_eq!(
            poll(&tab, 1).unwrap().support,
            AutopilotSupport::NotReported
        );

        // The agent was started by an APAS that had no autopilot. Its report
        // file, were there one, changes nothing.
        tab.write_state(&tab.report(5, None));
        assert_eq!(poll(&tab, 1).unwrap().support, AutopilotSupport::Supported);
        tab.drop_the_launch_marker();
        let frame = poll(&tab, 1).unwrap();
        assert_eq!(
            (frame.support, frame.state),
            (AutopilotSupport::NeedsAgentReboot, None)
        );

        // No launch record at all: no extension was installed for this agent.
        let empty = tempfile::tempdir().unwrap();
        let mut relay = PaneAutopilotRelay::default();
        assert_eq!(
            relay
                .poll(Some(empty.path()), Some(tab.pid()), Launched, 1)
                .unwrap()
                .support,
            AutopilotSupport::NeedsAgentReboot
        );
    }

    #[test]
    fn a_tab_whose_agent_is_not_launched_yet_never_asks_for_a_reboot() {
        // A tab exists before its agent is launched: its runtime directory
        // is empty, or still holds the record of an agent that ran there
        // before, which an APAS without autopilot may have written.
        let tab = Tab::new();
        let empty = tempfile::tempdir().unwrap();
        let mut relay = PaneAutopilotRelay::default();
        let frame = relay
            .poll(Some(empty.path()), None, NotLaunched, 1)
            .unwrap();
        assert_eq!(
            (frame.support, frame.state, frame.reported_at),
            (AutopilotSupport::NotReported, None, None)
        );
        let mut relay = PaneAutopilotRelay::default();
        assert_eq!(
            relay.poll(None, None, NotLaunched, 1).unwrap().support,
            AutopilotSupport::NotReported
        );

        tab.drop_the_launch_marker();
        let mut relay = PaneAutopilotRelay::default();
        assert_eq!(
            relay
                .poll(Some(&tab.runtime), None, NotLaunched, 1)
                .unwrap()
                .support,
            AutopilotSupport::NotReported
        );
        // Once an agent runs there, a record without the marker is its own.
        assert_eq!(
            relay
                .poll(Some(&tab.runtime), Some(tab.pid()), Launched, 1)
                .unwrap()
                .support,
            AutopilotSupport::NeedsAgentReboot
        );
        // The agent is launched by an APAS with autopilot: not reported
        // until its extension reports, then whatever that says.
        let (_, launch) = install(&tab.runtime, &tab.root.path().join("sessions")).unwrap();
        assert_eq!(
            relay
                .poll(Some(&tab.runtime), Some(tab.pid()), Launched, 1)
                .unwrap()
                .support,
            AutopilotSupport::NotReported
        );
        let mut report = tab.report(1, None);
        report.launch_id = launch.launch_id;
        tab.write_state(&report);
        assert_eq!(
            relay
                .poll(Some(&tab.runtime), Some(tab.pid()), Launched, 1)
                .unwrap()
                .support,
            AutopilotSupport::Supported
        );
    }

    #[test]
    fn a_state_report_is_trusted_only_from_this_launchs_live_process() {
        let mut tab = Tab::new();
        let read = |tab: &Tab| read_state(&tab.runtime, tab.launch_id, Some(tab.pid()));
        let trusted = tab.report(1, None);
        tab.write_state(&trusted);
        assert_eq!(read(&tab), Some(trusted.clone()));
        assert_eq!(mode(&tab.path(STATE_FILE)), 0o600);

        for forged in [
            StateReport {
                launch_id: Uuid::new_v4(),
                ..trusted.clone()
            },
            StateReport {
                pid: tab.pid() + 1,
                ..trusted.clone()
            },
            StateReport {
                process_start: format!("{}0", tab.process_start),
                ..trusted.clone()
            },
        ] {
            tab.write_state(&forged);
            assert_eq!(read(&tab), None);
        }
        tab.write_state(&trusted);
        assert_eq!(read_state(&tab.runtime, tab.launch_id, None), None);

        fs::set_permissions(tab.path(STATE_FILE), fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(read(&tab), None);
        fs::remove_file(tab.path(STATE_FILE)).unwrap();
        let outside = tab.root.path().join("state.json");
        write_private_atomic(&outside, &serde_json::to_vec(&trusted).unwrap()).unwrap();
        symlink(&outside, tab.path(STATE_FILE)).unwrap();
        assert_eq!(read(&tab), None);
        fs::remove_file(tab.path(STATE_FILE)).unwrap();

        // The bound is 64 KiB, not the activity report's 8192 bytes.
        let mut padded = serde_json::to_vec(&trusted).unwrap();
        padded.resize(MAX_BYTES as usize, b' ');
        write_private_atomic(&tab.path(STATE_FILE), &padded).unwrap();
        assert_eq!(read(&tab), Some(trusted.clone()));
        padded.push(b' ');
        write_private_atomic(&tab.path(STATE_FILE), &padded).unwrap();
        assert_eq!(read(&tab), None);

        // A runtime directory others can enter authenticates nothing.
        tab.write_state(&trusted);
        fs::set_permissions(&tab.runtime, fs::Permissions::from_mode(0o755)).unwrap();
        let mut relay = PaneAutopilotRelay::default();
        assert_eq!(
            relay
                .poll(Some(&tab.runtime), Some(tab.pid()), Launched, 1)
                .unwrap()
                .support,
            AutopilotSupport::NeedsAgentReboot
        );
        fs::set_permissions(&tab.runtime, fs::Permissions::from_mode(0o700)).unwrap();

        // The process is gone: its last report says nothing about now.
        tab.provider.kill().unwrap();
        tab.provider.wait().unwrap();
        assert_eq!(read(&tab), None);
    }

    #[test]
    fn a_report_not_seen_refreshed_for_ninety_seconds_is_not_reported() {
        let tab = Tab::new();
        let launch = tab.launch();
        let mut relay = PaneAutopilotRelay::default();
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let on = Some(view("waiting", Some("next_iteration")));
        let mut observe = |counter, seconds: u64, wall: &str| {
            let report = Some(tab.report(counter, on.clone()));
            relay.observe(Some(&launch), report, Launched, 1, at(seconds), wall)
        };

        let frame = observe(1, 0, "2026-11-13T14:00:00Z").unwrap();
        assert_eq!(frame.support, AutopilotSupport::Supported);
        assert_eq!(frame.reported_at.as_deref(), Some("2026-11-13T14:00:00Z"));
        // A refresh alone is not a change worth a frame.
        assert_eq!(observe(2, 30, "2026-11-13T14:00:30Z"), None);
        // The file still reads, but its counter has stopped changing.
        assert_eq!(observe(2, 119, "2026-11-13T14:01:59Z"), None);
        let stale = observe(2, 120, "2026-11-13T14:02:00Z").unwrap();
        assert_eq!(
            (stale.support, stale.state),
            (AutopilotSupport::NotReported, None)
        );
        // Sent with when it was last seen refreshed.
        assert_eq!(stale.reported_at.as_deref(), Some("2026-11-13T14:00:30Z"));
        assert_eq!(observe(2, 500, "2026-11-13T14:08:20Z"), None);

        // The tab refreshes again.
        let back = observe(3, 501, "2026-11-13T14:08:21Z").unwrap();
        assert_eq!(back.support, AutopilotSupport::Supported);
        assert_eq!(back.state.unwrap().autopilot, on);
        assert_eq!(back.reported_at.as_deref(), Some("2026-11-13T14:08:21Z"));
    }

    #[test]
    fn every_change_is_sent_and_everything_is_sent_again_after_a_registration() {
        let tab = Tab::new();
        let launch = tab.launch();
        let mut relay = PaneAutopilotRelay::default();
        let now = Instant::now();
        let mut observe = |counter, autopilot, generation| {
            let report = Some(tab.report(counter, autopilot));
            relay.observe(
                Some(&launch),
                report,
                Launched,
                generation,
                now,
                "2026-11-13T14:00:00Z",
            )
        };

        assert!(observe(1, None, 1).is_some());
        assert_eq!(observe(2, None, 1), None);
        let started = observe(3, Some(view("starting", None)), 1).unwrap();
        assert_eq!(
            started.state.unwrap().autopilot,
            Some(view("starting", None))
        );
        assert_eq!(observe(4, Some(view("starting", None)), 1), None);
        assert!(observe(5, Some(view("running", None)), 1).is_some());
        // Reconnect, or a server restart: nobody has to ask.
        let again = observe(6, Some(view("running", None)), 2).unwrap();
        assert_eq!(again.support, AutopilotSupport::Supported);
        assert_eq!(again.state.unwrap().autopilot, Some(view("running", None)));
        assert_eq!(observe(7, Some(view("running", None)), 2), None);

        // A tab with nothing to report says so again as well.
        let mut idle = PaneAutopilotRelay::default();
        let mut silent =
            |generation| idle.observe(Some(&launch), None, Launched, generation, now, "wall");
        assert_eq!(silent(1).unwrap().support, AutopilotSupport::NotReported);
        assert_eq!(silent(1), None);
        assert_eq!(silent(2).unwrap().support, AutopilotSupport::NotReported);
    }

    #[test]
    fn an_agent_that_exits_by_itself_is_reported_paused_on_the_tabs_behalf() {
        let tab = Tab::new();
        let launch = tab.launch();
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut relay = PaneAutopilotRelay::default();

        let on = view("waiting", Some("next_iteration"));
        let report = Some(tab.report(1, Some(on.clone())));
        assert!(relay
            .observe(
                Some(&launch),
                report,
                Launched,
                1,
                at(0),
                "2026-11-13T14:00:00Z"
            )
            .is_some());

        // The process is gone, so its files no longer pass the trust checks.
        // The pause is built from what was last relayed, in place of "not
        // reported".
        let frame = relay
            .observe(
                Some(&launch),
                None,
                ExitedByItself,
                1,
                at(3),
                "2026-11-13T14:00:03Z",
            )
            .unwrap();
        assert_eq!(frame.support, AutopilotSupport::Supported);
        assert_eq!(frame.reported_at.as_deref(), Some("2026-11-13T14:00:00Z"));
        let state = frame.state.unwrap();
        assert!(state.reported_by_cli);
        assert_eq!((&state.last_ended, &state.omp_modes), (&None, &None));
        let paused = state.autopilot.clone().unwrap();
        assert_eq!(paused.condition.kind, autopilot_condition::PAUSED);
        assert_eq!(
            paused.condition.detail.as_deref(),
            Some(autopilot_condition::PAUSE_AGENT_EXITED)
        );
        assert_eq!(
            paused.condition.since.as_deref(),
            Some("2026-11-13T14:00:03Z")
        );
        assert!(!paused.condition.is_on());
        // Its id, prompt, timing, limits and progress are as last relayed.
        assert_eq!(
            (
                paused.autopilot_id,
                &paused.spec,
                &paused.started_at,
                paused.iterations
            ),
            (on.autopilot_id, &on.spec, &on.started_at, on.iterations)
        );
        assert_eq!(paused.next_iteration_at, None);

        // Exempt from the ninety-second rule, and sent again after each
        // registration.
        assert_eq!(
            relay.observe(Some(&launch), None, ExitedByItself, 1, at(600), "later"),
            None
        );
        let again = relay
            .observe(Some(&launch), None, ExitedByItself, 2, at(601), "later")
            .unwrap();
        assert_eq!(again.state, Some(state));

        // The agent is launched again: the pause is dropped, and the tab is
        // not reported until its own report says what became of the autopilot.
        let (_, relaunched) = install(&tab.runtime, &tab.root.path().join("sessions")).unwrap();
        let frame = relay
            .observe(Some(&relaunched), None, Launched, 2, at(700), "relaunched")
            .unwrap();
        assert_eq!(
            (frame.support, frame.state, frame.reported_at),
            (AutopilotSupport::NotReported, None, None)
        );
        // Nothing of the old launch is left to report if the new agent exits.
        assert_eq!(
            relay.observe(Some(&relaunched), None, ExitedByItself, 2, at(703), "exit"),
            None
        );
    }

    #[test]
    fn an_exit_the_cli_asked_for_or_without_an_autopilot_is_just_not_reported() {
        let tab = Tab::new();
        let launch = tab.launch();
        let now = Instant::now();

        // A close, a reboot or a project stop: the caller passes false.
        let mut relay = PaneAutopilotRelay::default();
        let report = Some(tab.report(1, Some(view("running", None))));
        relay.observe(Some(&launch), report, Launched, 1, now, "wall");
        let frame = relay
            .observe(Some(&launch), None, Launched, 1, now, "wall")
            .unwrap();
        assert_eq!(
            (frame.support, frame.state),
            (AutopilotSupport::NotReported, None)
        );

        // The last relayed state had no autopilot, or one that had ended.
        let mut relay = PaneAutopilotRelay::default();
        let mut ended = tab.report(1, None);
        ended.last_ended = Some(shared::AutopilotEnded {
            autopilot_id: Uuid::from_u128(7),
            spec: spec(),
            started_at: "2026-11-13T14:00:00Z".into(),
            iterations: 20,
            reason: autopilot_condition::END_ITERATION_LIMIT.into(),
            ended_at: "2026-11-13T15:00:00Z".into(),
            turn_since: false,
        });
        assert!(relay
            .observe(Some(&launch), Some(ended), Launched, 1, now, "wall")
            .is_some());
        let frame = relay
            .observe(Some(&launch), None, ExitedByItself, 1, now, "wall")
            .unwrap();
        assert_eq!(
            (frame.support, frame.state),
            (AutopilotSupport::NotReported, None)
        );

        // A paused autopilot is still the tab's autopilot when its agent exits.
        let mut relay = PaneAutopilotRelay::default();
        let paused = view("paused", Some(autopilot_condition::PAUSE_FAILURES));
        relay.observe(
            Some(&launch),
            Some(tab.report(1, Some(paused))),
            Launched,
            1,
            now,
            "wall",
        );
        let state = relay
            .observe(Some(&launch), None, ExitedByItself, 1, now, "wall")
            .unwrap()
            .state
            .unwrap();
        assert_eq!(
            state.autopilot.unwrap().condition.detail.as_deref(),
            Some(autopilot_condition::PAUSE_AGENT_EXITED)
        );

        // A report that went stale first does not erase what was relayed.
        let mut relay = PaneAutopilotRelay::default();
        let on = Some(tab.report(1, Some(view("running", None))));
        relay.observe(Some(&launch), on.clone(), Launched, 1, now, "wall");
        let later = now + STATE_STALE_AFTER;
        assert_eq!(
            relay
                .observe(Some(&launch), on, Launched, 1, later, "wall")
                .unwrap()
                .support,
            AutopilotSupport::NotReported
        );
        let frame = relay
            .observe(Some(&launch), None, ExitedByItself, 1, later, "wall")
            .unwrap();
        assert!(frame.state.unwrap().reported_by_cli);
    }

    fn heartbeat(tab: &Tab, link: &ServerLink, now: Instant) -> ControllerHeartbeat {
        write_heartbeat(&tab.runtime, link, now).unwrap();
        assert_eq!(mode(&tab.path(CONTROLLER_FILE)), 0o600);
        read_private_json_bounded(&tab.path(CONTROLLER_FILE), MAX_BYTES).unwrap()
    }

    #[test]
    fn the_heartbeat_says_connected_only_while_the_server_is_heard_from() {
        let tab = Tab::new();
        let link = ServerLink::new();
        let echoed = [shared::OMP_TAB_AUTOPILOT_CAPABILITY.to_string()];

        // Not registered yet. This runtime cannot say for how long the server
        // has been out of reach, so it never says "for a moment": a tab that
        // was held through an outage stays held when its CLI is replaced in
        // the middle of it.
        let first = heartbeat(&tab, &link, Instant::now() + Duration::from_secs(5));
        assert_eq!((first.v, first.launch_id), (FORMAT_VERSION, tab.launch_id));
        assert!(!first.connected && !first.server_controls_autopilot);
        assert!(first.unreachable_secs > SERVER_UNREACHABLE_HOLD_SECS);
        // The top of a reconnect attempt changes nothing about that.
        link.reconnecting();
        let waiting = heartbeat(&tab, &link, Instant::now() + Duration::from_secs(30));
        assert!(!waiting.connected);
        assert!(waiting.unreachable_secs > SERVER_UNREACHABLE_HOLD_SECS);

        link.registered(&echoed);
        let connected = heartbeat(&tab, &link, Instant::now());
        assert!(connected.connected && connected.server_controls_autopilot);
        assert_eq!(connected.unreachable_secs, 0);
        // The same writer, and a counter that changes on every write.
        assert_eq!(connected.writer, first.writer);
        assert_ne!(connected.counter, first.counter);

        // The socket may still look open, but nothing has arrived for longer
        // than the server's own bound.
        let within = heartbeat(&tab, &link, Instant::now() + Duration::from_secs(80));
        assert!(within.connected);
        let silent = heartbeat(
            &tab,
            &link,
            Instant::now() + SERVER_SILENCE_LIMIT + Duration::from_secs(1),
        );
        assert!(!silent.connected && silent.server_controls_autopilot);
        assert!(silent.unreachable_secs >= 91);
        link.frame_arrived();
        assert!(heartbeat(&tab, &link, Instant::now()).connected);

        // The top of a reconnect attempt. The outage is counted from the last
        // frame that arrived, so a short one holds nothing.
        link.reconnecting();
        let dropped = heartbeat(&tab, &link, Instant::now() + Duration::from_secs(20));
        assert!(!dropped.connected);
        assert!((20..=SERVER_UNREACHABLE_HOLD_SECS).contains(&dropped.unreachable_secs));
        // Then a server that was rolled back by itself: registered, but it
        // echoes nothing.
        link.registered(&[]);
        let rolled_back = heartbeat(&tab, &link, Instant::now());
        assert!(rolled_back.connected && !rolled_back.server_controls_autopilot);

        // A tab whose extension reads no heartbeat gets none.
        fs::remove_file(tab.path(CONTROLLER_FILE)).unwrap();
        tab.drop_the_launch_marker();
        assert!(write_heartbeat(&tab.runtime, &link, Instant::now()).is_none());
        assert!(!tab.path(CONTROLLER_FILE).exists());
    }

    #[test]
    fn the_heartbeat_keeps_being_written_while_the_outbound_queue_is_full() {
        let tab = Tab::new();
        let frame = || shared::CliToServer::PaneStatus {
            session_id: Uuid::nil(),
            pane_type: shared::PaneType::Interactive,
            pane_id: Some(7),
            status: None,
        };
        // An outage: nothing drains the project's queue, and the pane watcher
        // is stuck at its next send.
        let (outbound, drained) = tokio::sync::mpsc::channel(1);
        outbound.try_send(frame()).unwrap();
        let watcher = std::thread::spawn(move || outbound.blocking_send(frame()));

        let link = Arc::new(ServerLink::new());
        let shutdown = Arc::new(AtomicBool::new(false));
        let writer = {
            let (runtime, link, shutdown) = (tab.runtime.clone(), link.clone(), shutdown.clone());
            std::thread::spawn(move || {
                run_controller_heartbeat(
                    || vec![runtime.clone()],
                    &link,
                    &shutdown,
                    Duration::from_millis(5),
                )
            })
        };
        let read = || -> Option<ControllerHeartbeat> {
            read_private_json_bounded(&tab.path(CONTROLLER_FILE), MAX_BYTES)
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen: Vec<ControllerHeartbeat> = Vec::new();
        while seen.len() < 3 {
            assert!(Instant::now() < deadline, "the heartbeat stopped");
            match read() {
                Some(beat) if seen.last().is_none_or(|last| last.counter != beat.counter) => {
                    seen.push(beat)
                }
                _ => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        assert!(seen.iter().all(|beat| beat.writer == seen[0].writer));
        assert!(seen
            .iter()
            .all(|beat| beat.launch_id == tab.launch_id && !beat.connected));
        assert!(!watcher.is_finished(), "the queue was drained");

        shutdown.store(true, Ordering::SeqCst);
        writer.join().unwrap();
        drop(drained);
        assert!(watcher.join().unwrap().is_err());
    }
}
