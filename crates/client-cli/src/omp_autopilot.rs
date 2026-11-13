//! Autopilot's private files between the project CLI and the OMP extension.
//!
//! The loop runs inside the tab's OMP process, in `omp_activity.mjs`. The CLI
//! never drives it directly: it leaves requests in the pane's runtime
//! directory (`…/apas/panes/<project>/<pane>/`), reads what the extension
//! reports there, and writes a heartbeat that says a controller is present.
//! The directory and every file are private to the OS account (0700/0600),
//! as for the question bridge. The design record is
//! `openspec/changes/add-omp-tab-autopilot/design.md`, decisions 5 to 8.
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

use serde::{Deserialize, Serialize};
use std::time::Duration;
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
