//! Private, process-scoped activity published by an explicitly loaded OMP extension.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::transcript::TranscriptActivity;

const RUNTIME_ENV: &str = "APAS_OMP_RUNTIME";
const LAUNCH_ENV: &str = "APAS_OMP_LAUNCH";
/// The pid of the process that spawns the provider. The extension reports
/// only from a process whose parent this is, which is how a wrapper's child
/// or a subagent run as its own process stays silent although it inherited
/// the tab's environment. It replaces the extension's "am I my own process
/// group leader" test, which needed `/proc`.
const PARENT_ENV: &str = "APAS_OMP_PARENT";
/// Recorded in the launch file beside the autopilot marker by a CLI whose
/// extension needs no `/proc`. See [`crate::omp_autopilot::has_autopilot`].
pub(crate) const PORTABLE_LAUNCH_FEATURE: &str = "portable_v1";
const LAUNCH_FILE: &str = "omp-launch.json";
const REPORT_FILE: &str = "omp-activity.json";
const EXTENSION_FILE: &str = "omp-activity.mjs";
const EXTENSION: &str = include_str!("omp_activity.mjs");
const MAX_REPORT_BYTES: u64 = 8192;
const QUESTION_FILE: &str = "omp-question.json";
const SUBMISSION_FILE: &str = "omp-question-submit.json";
const ACK_FILE: &str = "omp-question-ack.json";
const ANSWER_LOCK_FILE: &str = "omp-question.lock";
const MAX_QUESTION_BYTES: u64 = 128 * 1024;
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize, Deserialize)]
struct QuestionScope {
    launch_id: Uuid,
    pid: i32,
    /// Never compared. It is kept only to be echoed back exactly as the tab
    /// wrote it: an extension from before 2026-11-15 compares the scope of an
    /// answer request with its own, this field included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_start: Option<String>,
    session_id: String,
    transcript_path: PathBuf,
    tool_call_id: String,
    dialog_id: Uuid,
}

/// Everything but `process_start`, which nothing may require.
impl PartialEq for QuestionScope {
    fn eq(&self, other: &Self) -> bool {
        self.launch_id == other.launch_id
            && self.pid == other.pid
            && self.session_id == other.session_id
            && self.transcript_path == other.transcript_path
            && self.tool_call_id == other.tool_call_id
            && self.dialog_id == other.dialog_id
    }
}

#[derive(Serialize, Deserialize)]
struct QuestionReport {
    #[serde(flatten)]
    scope: QuestionScope,
    questions: serde_json::Value,
}

#[derive(Serialize)]
struct AnswerRequest<'a> {
    #[serde(flatten)]
    scope: &'a QuestionScope,
    nonce: Uuid,
    request_id: &'a str,
    answers: &'a [shared::OmpQuestionAnswer],
    cancel: bool,
}

#[derive(Serialize, Deserialize)]
struct AnswerAck {
    #[serde(flatten)]
    scope: QuestionScope,
    nonce: Uuid,
    request_id: String,
    accepted: bool,
    error: Option<String>,
}

pub struct OmpQuestionReport {
    pub tool_use_id: String,
    pub dialog_id: Uuid,
    pub session_id: String,
    pub transcript_path: PathBuf,
}

/// The native identity is opaque to routing; callers must never split this ID.
pub fn question_tool_use_id(pane_id: u32, session_id: &str, tool_call_id: &str) -> String {
    format!("omp:{pane_id}:{session_id}:{tool_call_id}")
}

pub fn reported_question(
    project_id: Uuid,
    pane_id: u32,
    process_group_id: Option<i32>,
) -> Option<OmpQuestionReport> {
    let report = read_question(&pane_runtime_dir(project_id, pane_id)?, process_group_id?)?;
    Some(OmpQuestionReport {
        tool_use_id: question_tool_use_id(
            pane_id,
            &report.scope.session_id,
            &report.scope.tool_call_id,
        ),
        dialog_id: report.scope.dialog_id,
        session_id: report.scope.session_id,
        transcript_path: report.scope.transcript_path,
    })
}

/// A bounded, private native-dialog exchange, not a PTY write. Success means
/// the live extension accepted the answer; only its transcript confirms it.
pub fn submit_answer(
    project_id: Uuid,
    pane_id: u32,
    process_group_id: Option<i32>,
    tool_use_id: &str,
    dialog_id: Uuid,
    request_id: &str,
    answers: &[shared::OmpQuestionAnswer],
    cancel: bool,
) -> Result<()> {
    let runtime = pane_runtime_dir(project_id, pane_id).context("OMP runtime is unavailable")?;
    submit_answer_at(
        &runtime,
        process_group_id.context("OMP process is unavailable")?,
        pane_id,
        tool_use_id,
        dialog_id,
        request_id,
        answers,
        cancel,
        ANSWER_TIMEOUT,
    )
}

pub(crate) fn pane_runtime_dir(project_id: Uuid, pane_id: u32) -> Option<PathBuf> {
    Some(
        crate::config::Config::runtime_dir()
            .ok()?
            .join("panes")
            .join(crate::pane_host::short_uuid(project_id))
            .join(pane_id.to_string()),
    )
}

fn read_question(runtime: &Path, pid: i32) -> Option<QuestionReport> {
    if pid <= 0 || !private_metadata(runtime, true) {
        return None;
    }
    let launch: Launch = read_private_json(&runtime.join(LAUNCH_FILE))?;
    let mut report: QuestionReport =
        read_private_json_bounded(&runtime.join(QUESTION_FILE), MAX_QUESTION_BYTES)?;
    let scope = &mut report.scope;
    if scope.launch_id != launch.launch_id
        || scope.pid != pid
        || scope.session_id.is_empty()
        || scope.tool_call_id.is_empty()
        || !provider_alive(pid)
        || !valid_questions(&report.questions)
    {
        return None;
    }
    scope.transcript_path = scoped_transcript(&launch.session_dir, &scope.transcript_path)?;
    Some(report)
}

fn valid_questions(value: &serde_json::Value) -> bool {
    let Some(questions) = value.as_array().filter(|questions| !questions.is_empty()) else {
        return false;
    };
    let mut ids = std::collections::HashSet::with_capacity(questions.len());
    for question in questions {
        let Some(id) = question.get("id").and_then(serde_json::Value::as_str) else {
            return false;
        };
        let Some(options) = question
            .get("options")
            .and_then(serde_json::Value::as_array)
        else {
            return false;
        };
        if !ids.insert(id)
            || question
                .get("question")
                .and_then(serde_json::Value::as_str)
                .is_none()
            || question
                .get("multi")
                .is_some_and(|multi| !multi.is_boolean())
        {
            return false;
        }
        let mut labels = std::collections::HashSet::with_capacity(options.len());
        for option in options {
            let Some(label) = option.get("label").and_then(serde_json::Value::as_str) else {
                return false;
            };
            if !labels.insert(label) {
                return false;
            }
        }
    }
    true
}

fn validate_answers(
    questions: &serde_json::Value,
    answers: &[shared::OmpQuestionAnswer],
    cancel: bool,
) -> Result<()> {
    if cancel {
        if !answers.is_empty() {
            bail!("cancellation cannot include answers");
        }
        return Ok(());
    }
    let questions = questions
        .as_array()
        .context("invalid native OMP questions")?;
    if questions.len() != answers.len() {
        bail!("answer every OMP question exactly once");
    }
    for (question, answer) in questions.iter().zip(answers) {
        if question["id"].as_str() != Some(answer.id.as_str()) {
            bail!("OMP answers must match the native question IDs and order");
        }
        let options = question["options"]
            .as_array()
            .context("invalid native OMP options")?;
        for (index, selected) in answer.selected_options.iter().enumerate() {
            if !options
                .iter()
                .any(|option| option["label"].as_str() == Some(selected.as_str()))
                || answer.selected_options[..index].contains(selected)
            {
                bail!("invalid or repeated OMP option");
            }
        }
        if answer
            .custom_input
            .as_ref()
            .is_some_and(|text| text.trim().is_empty())
        {
            bail!("OMP custom input must not be blank");
        }
        if question["multi"].as_bool() != Some(true)
            && (answer.selected_options.len() > 1
                || (answer.selected_options.is_empty() && answer.custom_input.is_none()))
        {
            bail!("a single-select OMP question requires one option or custom input");
        }
    }
    Ok(())
}

struct AnswerLock(fs::File);

impl Drop for AnswerLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // APAS has other threads spawning providers. A concurrent fork
            // can briefly inherit this open file description, so closing our
            // descriptor alone does not necessarily release its flock.
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn lock_answers(runtime: &Path) -> Result<AnswerLock> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options.open(runtime.join(ANSWER_LOCK_FILE))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || !private_owner(&metadata) {
            bail!("insecure OMP answer lock");
        }
        // The inode remains in place. Removing a lock file would allow a
        // second submitter to lock a new inode while this exchange still runs.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another OMP answer is already being submitted");
        }
        Ok(AnswerLock(file))
    }
    #[cfg(not(unix))]
    {
        let _ = (options, runtime);
        bail!("the OMP native bridge requires a Unix runtime");
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_answer_at(
    runtime: &Path,
    pid: i32,
    pane_id: u32,
    tool_use_id: &str,
    dialog_id: Uuid,
    request_id: &str,
    answers: &[shared::OmpQuestionAnswer],
    cancel: bool,
    timeout: Duration,
) -> Result<()> {
    if request_id.is_empty() || request_id.len() > 1024 || !private_metadata(runtime, true) {
        bail!("invalid OMP request or private runtime");
    }
    let _lock = lock_answers(runtime)?;
    let report =
        read_question(runtime, pid).context("the native OMP question is no longer available")?;
    if report.scope.dialog_id != dialog_id
        || question_tool_use_id(
            pane_id,
            &report.scope.session_id,
            &report.scope.tool_call_id,
        ) != tool_use_id
    {
        bail!("this OMP question is no longer current");
    }
    validate_answers(&report.questions, answers, cancel)?;
    let nonce = Uuid::new_v4();
    let request = AnswerRequest {
        scope: &report.scope,
        nonce,
        request_id,
        answers,
        cancel,
    };
    let body = serde_json::to_vec(&request)?;
    if body.len() as u64 > MAX_QUESTION_BYTES {
        bail!("OMP answer exceeds the private bridge size limit");
    }
    let submission = runtime.join(SUBMISSION_FILE);
    let ack_path = runtime.join(ACK_FILE);
    // An old acknowledgement can never satisfy a fresh random nonce.
    let _ = fs::remove_file(&ack_path);
    write_private_atomic(&submission, &body)?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        if let Some(ack) = read_private_json::<AnswerAck>(&ack_path) {
            if ack.scope == report.scope && ack.nonce == nonce && ack.request_id == request_id {
                let _ = fs::remove_file(&ack_path);
                break if ack.accepted {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(ack
                        .error
                        .unwrap_or_else(|| "native OMP rejected the answer".into())))
                };
            }
        }
        if Instant::now() >= deadline {
            break Err(anyhow::anyhow!(
                "OMP did not acknowledge the answer; delivery is uncertain, use the terminal and do not automatically resend"
            ));
        }
        // Do not fail early when the question report disappears: the native
        // picker may just have settled and its acknowledgement is in flight.
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    };
    let _ = fs::remove_file(&submission);
    result
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Launch {
    pub(crate) launch_id: Uuid,
    session_dir: PathBuf,
    /// What the extension written beside this record can do. A record from
    /// before autopilot has none, and its agent needs a reboot to gain it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) features: Vec<String>,
}

/// The record of the pane's current provider launch, when the runtime
/// directory and the record are private to this account.
pub(crate) fn read_launch(runtime_dir: &Path) -> Option<Launch> {
    if !private_metadata(runtime_dir, true) {
        return None;
    }
    read_private_json(&runtime_dir.join(LAUNCH_FILE))
}

#[derive(Serialize, Deserialize)]
struct Report {
    launch_id: Uuid,
    pid: i32,
    /// Written by the extension where it can read `/proc`, for CLIs from
    /// before 2026-11-15, which required it. Never compared here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_start: Option<String>,
    session_id: String,
    transcript_path: PathBuf,
    activity: TranscriptActivity,
}

pub struct OmpActivityReport {
    pub activity: TranscriptActivity,
    pub transcript_path: PathBuf,
}

/// Fail open: a reporting failure must never prevent the interactive provider starting.
pub fn prepare(
    provider: &shared::Provider,
    project_id: Uuid,
    pane_id: u32,
    conversation_id: Uuid,
    env: &mut Vec<(String, String)>,
) -> Option<PathBuf> {
    if !matches!(provider, shared::Provider::Omp) {
        return None;
    }
    let prepared = (|| -> Result<_> {
        let runtime_dir = crate::claude_session_hook::pane_runtime_dir(project_id, pane_id)?;
        let home = dirs::home_dir().context("no home directory for OMP sessions")?;
        let session_dir = crate::transcript::omp_session_dir(&home, conversation_id);
        fs::create_dir_all(&session_dir)?;
        install(&runtime_dir, &session_dir)
            .map(|(extension, launch)| (runtime_dir, extension, launch))
    })();
    match prepared {
        Ok((runtime_dir, extension, launch)) => {
            set_launch_env(env, &runtime_dir, &launch);
            Some(extension)
        }
        Err(error) => {
            tracing::warn!(%error, pane_id, "could not prepare OMP activity extension; using transcript status");
            None
        }
    }
}

/// What the extension reads from its environment, replacing anything the
/// caller inherited from a tab it runs in itself.
fn set_launch_env(env: &mut Vec<(String, String)>, runtime_dir: &Path, launch: &Launch) {
    env.retain(|(key, _)| key != RUNTIME_ENV && key != LAUNCH_ENV && key != PARENT_ENV);
    env.push((
        RUNTIME_ENV.into(),
        runtime_dir.to_string_lossy().into_owned(),
    ));
    env.push((LAUNCH_ENV.into(), launch.launch_id.to_string()));
    // This runs in the process that spawns the provider: the pane host for a
    // hosted tab, the project CLI for a direct PTY.
    env.push((PARENT_ENV.into(), std::process::id().to_string()));
}

pub(crate) fn install(runtime_dir: &Path, session_dir: &Path) -> Result<(PathBuf, Launch)> {
    if !private_metadata(runtime_dir, true) {
        bail!("insecure OMP runtime directory {}", runtime_dir.display());
    }
    let launch = Launch {
        launch_id: Uuid::new_v4(),
        session_dir: fs::canonicalize(session_dir)?,
        features: vec![
            crate::omp_autopilot::LAUNCH_FEATURE.to_string(),
            PORTABLE_LAUNCH_FEATURE.to_string(),
        ],
    };
    // Invalidate the old generation before an old provider can publish another report.
    write_private_atomic(
        &runtime_dir.join(LAUNCH_FILE),
        &serde_json::to_vec(&launch)?,
    )?;
    // An unapplied pause or stop is not carried over to the new provider:
    // its autopilot comes back paused anyway.
    for name in [
        REPORT_FILE,
        QUESTION_FILE,
        SUBMISSION_FILE,
        ACK_FILE,
        crate::omp_autopilot::CONTROLLER_FILE,
        crate::omp_autopilot::REQUEST_FILE,
        crate::omp_autopilot::STOP_FILE,
        crate::omp_autopilot::ACK_FILE,
        crate::omp_autopilot::STATE_FILE,
    ] {
        match fs::remove_file(runtime_dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let extension = runtime_dir.join(EXTENSION_FILE);
    write_private_atomic(&extension, EXTENSION.as_bytes())?;
    Ok((extension, launch))
}

/// The live provider identifies its transcript even before OMP first creates the file.
pub fn reported_activity(
    project_id: Uuid,
    pane_id: u32,
    process_group_id: Option<i32>,
) -> Option<OmpActivityReport> {
    read_activity(&pane_runtime_dir(project_id, pane_id)?, process_group_id?)
}

fn read_activity(runtime_dir: &Path, pid: i32) -> Option<OmpActivityReport> {
    if pid <= 0 || !private_metadata(runtime_dir, true) {
        return None;
    }
    let launch: Launch = read_private_json(&runtime_dir.join(LAUNCH_FILE))?;
    let report: Report = read_private_json(&runtime_dir.join(REPORT_FILE))?;
    if report.launch_id != launch.launch_id
        || report.pid != pid
        || report.session_id.is_empty()
        || !provider_alive(pid)
    {
        return None;
    }
    // Reject symlinks and nested subagent sessions, even if their environment was inherited.
    let transcript_path = scoped_transcript(&launch.session_dir, &report.transcript_path)?;
    Some(OmpActivityReport {
        activity: report.activity,
        transcript_path,
    })
}

fn scoped_transcript(session_dir: &Path, transcript_path: &Path) -> Option<PathBuf> {
    if !transcript_path.is_absolute() || transcript_path.extension()? != "jsonl" {
        return None;
    }
    let parent = fs::canonicalize(transcript_path.parent()?).ok()?;
    if parent != session_dir {
        return None;
    }
    match fs::symlink_metadata(transcript_path) {
        Ok(metadata) if metadata.is_file() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return None,
    }
    Some(parent.join(transcript_path.file_name()?))
}

fn read_private_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    read_private_json_bounded(path, MAX_REPORT_BYTES)
}

pub(crate) fn read_private_json_bounded<T: serde::de::DeserializeOwned>(
    path: &Path,
    limit: u64,
) -> Option<T> {
    if !private_metadata(path, false) {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit || !private_owner(&metadata) {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn private_metadata(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        (if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        }) && private_owner(&metadata)
    })
}

#[cfg(unix)]
pub(crate) fn private_owner(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
}

#[cfg(not(unix))]
pub(crate) fn private_owner(_metadata: &fs::Metadata) -> bool {
    false
}

/// Whether the provider this CLI launched as `pid` is still running and
/// still leads its own process group, as the direct child of a PTY does.
///
/// Not a trust check. It only keeps a file a dead provider left behind from
/// being read as current, on Linux and macOS alike. It used to be a
/// comparison of the file's `process_start` with `/proc/<pid>/stat`, which
/// also guarded against a reused pid; the owner dropped that on 2026-11-15
/// (the user's own environment is trusted), and with it the need for
/// `/proc`.
///
/// A provider that exited while one of its descendants keeps the PTY open is
/// not reaped until the PTY closes (both PTY owners wait for that), and on
/// Linux such a zombie still leads its group. Where `/proc` can be read it is
/// told apart and does not count as running; where it cannot, it counts
/// until the PTY closes.
#[cfg(unix)]
pub(crate) fn provider_alive(pid: i32) -> bool {
    crate::process_info::leads_own_group(pid) && !crate::process_info::exited_unreaped(pid)
}

#[cfg(not(unix))]
pub(crate) fn provider_alive(_pid: i32) -> bool {
    false
}

/// How long a tab's agent may run without its extension reporting anything
/// before the CLI says so. A real OMP reports within a few seconds of
/// starting; the margin is for a slow start on a large conversation.
const SILENT_EXTENSION_AFTER: Duration = Duration::from_secs(60);

/// Notices a tab whose extension never reported, once per provider process.
///
/// The extension turns itself off without a word when it is not the direct
/// child of the process that started it (`APAS_OMP_PARENT`), for example
/// behind a launcher that forks. The tab then looks healthy and quietly has
/// transcript status only, questions answered in the terminal only, and an
/// autopilot that reads "not reported". This is the one place that says so.
#[derive(Default)]
pub(crate) struct SilentExtensionWatch {
    pid: Option<i32>,
    running_since: Option<Instant>,
    settled: bool,
}

impl SilentExtensionWatch {
    /// One poll of the tab. True at most once per provider process: when it
    /// has been running for [`SILENT_EXTENSION_AFTER`], was launched with an
    /// extension that needs no `/proc`, and no report of it was ever read.
    pub(crate) fn observe(
        &mut self,
        runtime_dir: Option<&Path>,
        pid: Option<i32>,
        running: bool,
        reported: bool,
        now: Instant,
    ) -> bool {
        if self.pid != pid {
            *self = Self {
                pid,
                ..Self::default()
            };
        }
        if self.settled || pid.is_none() {
            return false;
        }
        if reported {
            self.settled = true;
            return false;
        }
        if !running {
            self.running_since = None;
            return false;
        }
        let since = *self.running_since.get_or_insert(now);
        if now.duration_since(since) < SILENT_EXTENSION_AFTER {
            return false;
        }
        self.settled = true;
        // An extension from before 2026-11-15 is silent off Linux by design,
        // and its tab already asks for an agent reboot.
        runtime_dir.and_then(read_launch).is_some_and(|launch| {
            launch
                .features
                .iter()
                .any(|feature| feature == PORTABLE_LAUNCH_FEATURE)
        })
    }
}

pub(crate) fn write_private_atomic(path: &Path, body: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(body)?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    struct Fixture {
        root: tempfile::TempDir,
        runtime: PathBuf,
        session_dir: PathBuf,
        transcript: PathBuf,
        provider: Child,
        report: Report,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            // The CLI hands back canonical transcript paths, and a temporary
            // directory is behind a symlink on macOS (/var/folders).
            let base = fs::canonicalize(root.path()).unwrap();
            let runtime = base.join("runtime");
            let session_dir = base.join("sessions");
            crate::pane_host::ensure_private_dir(&runtime).unwrap();
            fs::create_dir(&session_dir).unwrap();
            let (_, launch) = install(&runtime, &session_dir).unwrap();
            let transcript = session_dir.join("parent.jsonl");
            fs::write(&transcript, b"{}").unwrap();
            let provider = Command::new("sleep")
                .arg("60")
                .process_group(0)
                .spawn()
                .unwrap();
            let pid = provider.id() as i32;
            let report = Report {
                launch_id: launch.launch_id,
                pid,
                // The shape a tab without `/proc` writes. The tests that
                // need the field set it themselves.
                process_start: None,
                session_id: "parent".into(),
                transcript_path: transcript.clone(),
                activity: TranscriptActivity::PendingAnswer,
            };
            Self {
                root,
                runtime,
                session_dir,
                transcript,
                provider,
                report,
            }
        }

        fn publish(&self) {
            write_private_atomic(
                &self.runtime.join(REPORT_FILE),
                &serde_json::to_vec(&self.report).unwrap(),
            )
            .unwrap();
        }

        fn activity(&self) -> Option<TranscriptActivity> {
            read_activity(&self.runtime, self.provider.id() as i32).map(|report| report.activity)
        }

        fn publish_question(&self) -> QuestionReport {
            let report = QuestionReport {
                scope: QuestionScope {
                    launch_id: self.report.launch_id,
                    pid: self.report.pid,
                    process_start: self.report.process_start.clone(),
                    session_id: self.report.session_id.clone(),
                    transcript_path: self.transcript.clone(),
                    tool_call_id: "native:call".into(),
                    dialog_id: Uuid::new_v4(),
                },
                questions: serde_json::json!([
                    {"id":"db", "question":"Database?", "recommended":1,
                     "options":[{"label":"SQLite"},{"label":"Postgres","description":"server"}]},
                    {"id":"features", "question":"Features?", "multi":true, "options":[{"label":"Auth"}]},
                    {"id":"note", "question":"Note?", "options":[]}
                ]),
            };
            self.write_question(&report);
            report
        }

        fn write_question(&self, report: &QuestionReport) {
            write_private_atomic(
                &self.runtime.join(QUESTION_FILE),
                &serde_json::to_vec(report).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.provider.kill();
            let _ = self.provider.wait();
        }
    }

    fn answers() -> Vec<shared::OmpQuestionAnswer> {
        vec![
            shared::OmpQuestionAnswer {
                id: "db".into(),
                selected_options: vec!["Postgres".into()],
                custom_input: None,
            },
            shared::OmpQuestionAnswer {
                id: "features".into(),
                selected_options: vec![],
                custom_input: None,
            },
            shared::OmpQuestionAnswer {
                id: "note".into(),
                selected_options: vec![],
                custom_input: Some("  exact text\nand spaces  ".into()),
            },
        ]
    }

    #[test]
    fn question_reports_preserve_native_identity_and_reject_stale_or_nested_scopes() {
        let fixture = Fixture::new();
        let mut question = fixture.publish_question();
        let current = read_question(&fixture.runtime, fixture.report.pid).unwrap();
        assert_eq!(current.scope.dialog_id, question.scope.dialog_id);
        assert_eq!(current.questions, question.questions);
        assert_eq!(
            question_tool_use_id(7, "native-session", "call:opaque"),
            "omp:7:native-session:call:opaque"
        );
        question.scope.pid += 1;
        fixture.write_question(&question);
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
        question.scope.pid -= 1;
        // The start time is not part of the identity: whatever it says, and
        // whether it is there at all, the question is read.
        for process_start in [Some("1".to_string()), None] {
            question.scope.process_start = process_start;
            fixture.write_question(&question);
            assert!(read_question(&fixture.runtime, fixture.report.pid).is_some());
        }
        let nested = fixture.session_dir.join("subagents");
        fs::create_dir(&nested).unwrap();
        question.scope.transcript_path = nested.join("child.jsonl");
        fixture.write_question(&question);
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
        question.scope.transcript_path = fixture.transcript.clone();
        fixture.write_question(&question);
        install(&fixture.runtime, &fixture.session_dir).unwrap();
        fixture.write_question(&question);
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
    }

    #[test]
    fn question_files_are_private_bounded_and_never_follow_symlinks() {
        let fixture = Fixture::new();
        let question = fixture.publish_question();
        let file = fixture.runtime.join(QUESTION_FILE);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
        fs::remove_file(&file).unwrap();
        let target = fixture.root.path().join("foreign-question.json");
        write_private_atomic(&target, &serde_json::to_vec(&question).unwrap()).unwrap();
        symlink(&target, &file).unwrap();
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
        fs::remove_file(&file).unwrap();
        write_private_atomic(&file, &vec![b' '; MAX_QUESTION_BYTES as usize + 1]).unwrap();
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
    }

    #[test]
    fn answers_validate_native_ids_labels_order_empty_multi_and_lossless_text() {
        let fixture = Fixture::new();
        let question = fixture.publish_question();
        assert!(validate_answers(&question.questions, &answers(), false).is_ok());
        assert!(validate_answers(&question.questions, &[], true).is_ok());
        assert!(validate_answers(&question.questions, &answers(), true).is_err());
        assert!(validate_answers(&question.questions, &[], false).is_err());
        let mut invalid = answers();
        invalid.swap(0, 1);
        assert!(validate_answers(&question.questions, &invalid, false).is_err());
        let mut invalid = answers();
        invalid[0].selected_options = vec!["fabricated".into()];
        assert!(validate_answers(&question.questions, &invalid, false).is_err());
        invalid[0].selected_options = vec!["SQLite".into(), "Postgres".into()];
        assert!(validate_answers(&question.questions, &invalid, false).is_err());
        invalid[0].selected_options.clear();
        assert!(validate_answers(&question.questions, &invalid, false).is_err());
        let mut invalid = answers();
        invalid[1].selected_options = vec!["Auth".into(), "Auth".into()];
        assert!(validate_answers(&question.questions, &invalid, false).is_err());
        let mut invalid = answers();
        invalid[2].custom_input = Some(" \n ".into());
        assert!(validate_answers(&question.questions, &invalid, false).is_err());
        let mut duplicate_questions = question.questions.clone();
        duplicate_questions[1]["id"] = serde_json::json!("db");
        assert!(!valid_questions(&duplicate_questions));
        let mut duplicate_labels = question.questions.clone();
        duplicate_labels[0]["options"][1]["label"] = serde_json::json!("SQLite");
        assert!(!valid_questions(&duplicate_labels));
    }

    #[test]
    fn writing_a_request_is_not_acceptance_and_delivery_wait_is_bounded() {
        let fixture = Fixture::new();
        let question = fixture.publish_question();
        let result = submit_answer_at(
            &fixture.runtime,
            fixture.report.pid,
            7,
            &question_tool_use_id(7, &question.scope.session_id, &question.scope.tool_call_id),
            question.scope.dialog_id,
            "../../not-a-filename",
            &answers(),
            false,
            Duration::ZERO,
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("delivery is uncertain"));
        assert!(!fixture.runtime.join(SUBMISSION_FILE).exists());
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_some());
    }

    #[test]
    fn native_ack_is_required_for_acceptance_rejection_and_cancel() {
        for (accepted, cancel, matching) in [
            (true, false, true),
            (false, false, true),
            (true, true, true),
            (true, false, false),
        ] {
            let fixture = Fixture::new();
            let question = fixture.publish_question();
            let runtime = fixture.runtime.clone();
            let expected_scope = question.scope.clone();
            let responder = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                let request = loop {
                    if let Some(request) = read_private_json_bounded::<serde_json::Value>(
                        &runtime.join(SUBMISSION_FILE),
                        MAX_QUESTION_BYTES,
                    ) {
                        break request;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "no answer request reached the native transport"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                };
                assert_eq!(request["cancel"], cancel);
                if cancel {
                    assert_eq!(request["answers"], serde_json::json!([]));
                } else {
                    assert_eq!(
                        request["answers"][2]["customInput"],
                        "  exact text\nand spaces  "
                    );
                }
                if accepted && matching {
                    // Native settle removes the report before Rust reads the ack.
                    fs::remove_file(runtime.join(QUESTION_FILE)).unwrap();
                }
                let ack = AnswerAck {
                    scope: expected_scope,
                    nonce: if matching {
                        serde_json::from_value(request["nonce"].clone()).unwrap()
                    } else {
                        Uuid::new_v4()
                    },
                    request_id: "answer-request".into(),
                    accepted,
                    error: (!accepted).then(|| "native rejection".into()),
                };
                write_private_atomic(&runtime.join(ACK_FILE), &serde_json::to_vec(&ack).unwrap())
                    .unwrap();
            });
            let sent_answers = if cancel { vec![] } else { answers() };
            let result = submit_answer_at(
                &fixture.runtime,
                fixture.report.pid,
                7,
                &question_tool_use_id(7, &question.scope.session_id, &question.scope.tool_call_id),
                question.scope.dialog_id,
                "answer-request",
                &sent_answers,
                cancel,
                Duration::from_secs(3),
            );
            responder.join().unwrap();
            assert_eq!(result.is_ok(), accepted && matching);
            if !accepted {
                assert!(result.unwrap_err().to_string().contains("native rejection"));
                assert!(read_question(&fixture.runtime, fixture.report.pid).is_some());
            }
            assert!(!fixture.runtime.join(SUBMISSION_FILE).exists());
        }
    }

    #[test]
    fn stale_and_concurrent_submissions_cannot_overwrite_the_native_mailbox() {
        let fixture = Fixture::new();
        let question = fixture.publish_question();
        let id = question_tool_use_id(7, &question.scope.session_id, &question.scope.tool_call_id);
        for (tool_id, dialog_id) in [
            ("other", question.scope.dialog_id),
            (id.as_str(), Uuid::new_v4()),
        ] {
            assert!(submit_answer_at(
                &fixture.runtime,
                fixture.report.pid,
                7,
                tool_id,
                dialog_id,
                "answer-request",
                &answers(),
                false,
                Duration::ZERO,
            )
            .is_err());
            assert!(!fixture.runtime.join(SUBMISSION_FILE).exists());
        }
        let lock = lock_answers(&fixture.runtime).unwrap();
        assert!(submit_answer_at(
            &fixture.runtime,
            fixture.report.pid,
            7,
            &id,
            question.scope.dialog_id,
            "answer-request",
            &answers(),
            false,
            Duration::ZERO,
        )
        .unwrap_err()
        .to_string()
        .contains("already being submitted"));
        drop(lock);
        let path = fixture.runtime.join(ANSWER_LOCK_FILE);
        fs::remove_file(&path).unwrap();
        symlink(&fixture.transcript, &path).unwrap();
        assert!(lock_answers(&fixture.runtime).is_err());
    }

    #[test]
    fn only_the_current_live_process_generation_is_accepted() {
        let mut fixture = Fixture::new();
        fixture.publish();
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
        fixture.report.pid += 1;
        fixture.publish();
        assert_eq!(fixture.activity(), None);
        fixture.report.pid -= 1;
        fixture.publish();
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
        fixture.provider.kill().unwrap();
        fixture.provider.wait().unwrap();
        assert_eq!(fixture.activity(), None);
    }

    #[test]
    fn a_report_with_another_launch_id_is_rejected() {
        let mut fixture = Fixture::new();
        fixture.publish();
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
        // A file left by an earlier launch of the same tab.
        fixture.report.launch_id = Uuid::new_v4();
        fixture.publish();
        assert_eq!(fixture.activity(), None);
        let mut question = fixture.publish_question();
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
        question.scope.launch_id = read_launch(&fixture.runtime).unwrap().launch_id;
        fixture.write_question(&question);
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_some());
    }

    /// The subagent case: a live process that inherited the tab's environment
    /// and reports under its own pid is not the provider this CLI started.
    #[test]
    fn a_report_whose_pid_is_not_the_providers_is_rejected() {
        let mut fixture = Fixture::new();
        let mut subagent = Command::new("sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .unwrap();
        let provider = fixture.report.pid;
        assert!(provider_alive(subagent.id() as i32));
        fixture.report.pid = subagent.id() as i32;
        fixture.publish();
        let mut question = fixture.publish_question();
        let rejected = (
            read_activity(&fixture.runtime, provider).is_none(),
            read_question(&fixture.runtime, provider).is_none(),
        );
        fixture.report.pid = provider;
        fixture.publish();
        question.scope.pid = provider;
        fixture.write_question(&question);
        let accepted = (
            read_activity(&fixture.runtime, provider).is_some(),
            read_question(&fixture.runtime, provider).is_some(),
        );
        subagent.kill().unwrap();
        subagent.wait().unwrap();
        assert_eq!(rejected, (true, true));
        assert_eq!(accepted, (true, true));
    }

    #[test]
    fn a_report_with_a_missing_or_different_process_start_is_accepted() {
        let mut fixture = Fixture::new();
        // No field at all: what a tab writes where there is no `/proc`.
        fixture.publish();
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.runtime.join(REPORT_FILE)).unwrap()).unwrap();
        assert!(written.get("process_start").is_none());
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
        // Any value: an extension from before the check was dropped writes
        // the provider's real start time, and it is not compared either.
        fixture.report.process_start = Some("not-this-process".into());
        fixture.publish();
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
        // A null reads as absent.
        let mut odd = serde_json::to_value(&fixture.report).unwrap();
        odd["process_start"] = serde_json::Value::Null;
        write_private_atomic(
            &fixture.runtime.join(REPORT_FILE),
            &serde_json::to_vec(&odd).unwrap(),
        )
        .unwrap();
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
    }

    #[test]
    fn a_report_from_a_provider_that_is_no_longer_running_is_not_current() {
        let mut fixture = Fixture::new();
        fixture.report.process_start = Some("12345".into());
        fixture.publish();
        fixture.publish_question();
        let pid = fixture.report.pid;
        assert!(provider_alive(pid));
        assert!(read_activity(&fixture.runtime, pid).is_some());
        assert!(read_question(&fixture.runtime, pid).is_some());
        fixture.provider.kill().unwrap();
        fixture.provider.wait().unwrap();
        // Both files are still there, with the right launch id and pid.
        assert!(fixture.runtime.join(REPORT_FILE).exists());
        assert!(fixture.runtime.join(QUESTION_FILE).exists());
        assert!(!provider_alive(pid));
        assert!(read_activity(&fixture.runtime, pid).is_none());
        assert!(read_question(&fixture.runtime, pid).is_none());
    }

    /// A provider that died while a descendant keeps the PTY open is not
    /// reaped until the PTY closes. On Linux it still leads its process
    /// group meanwhile, and its last files must not read as current.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_provider_that_exited_and_is_not_reaped_yet_is_not_running() {
        let mut fixture = Fixture::new();
        fixture.publish();
        fixture.publish_question();
        let pid = fixture.report.pid;
        assert!(read_activity(&fixture.runtime, pid).is_some());
        fixture.provider.kill().unwrap();
        // Wait for the exit without reaping it, as nothing reaps a provider
        // whose PTY is still open.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        assert_eq!(waited, 0);
        // Still there for the process table, and still its group's leader.
        assert!(crate::process_info::alive(pid as u32));
        assert!(crate::process_info::leads_own_group(pid));
        assert!(!provider_alive(pid));
        assert!(read_activity(&fixture.runtime, pid).is_none());
        assert!(read_question(&fixture.runtime, pid).is_none());
        fixture.provider.wait().unwrap();
        assert!(!provider_alive(pid));
    }

    #[test]
    fn a_live_process_that_does_not_lead_its_own_group_is_not_a_provider() {
        // A PTY's direct child leads its own group. This one stays in the
        // test's group, as a wrapper script's child stays in the wrapper's.
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let alive = provider_alive(child.id() as i32);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!alive);
        assert!(!provider_alive(0));
        assert!(!provider_alive(-1));
    }

    /// An extension from before the check was dropped compares the scope of
    /// an answer request with its own, `process_start` included, so the CLI
    /// hands the field back exactly as it read it and adds none of its own.
    #[test]
    fn an_answer_request_echoes_process_start_exactly_as_the_tab_wrote_it() {
        for process_start in [Some("4242424242".to_string()), None] {
            let fixture = Fixture::new();
            let mut question = fixture.publish_question();
            question.scope.process_start = process_start.clone();
            fixture.write_question(&question);
            let runtime = fixture.runtime.clone();
            let scope = question.scope.clone();
            let responder = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                let request = loop {
                    if let Some(request) = read_private_json_bounded::<serde_json::Value>(
                        &runtime.join(SUBMISSION_FILE),
                        MAX_QUESTION_BYTES,
                    ) {
                        break request;
                    }
                    assert!(Instant::now() < deadline, "no answer request was written");
                    std::thread::sleep(Duration::from_millis(1));
                };
                // The acknowledgement's own value is not compared: a tab that
                // cannot read `/proc` at that moment leaves it out.
                let ack = AnswerAck {
                    scope: QuestionScope {
                        process_start: None,
                        ..scope
                    },
                    nonce: serde_json::from_value(request["nonce"].clone()).unwrap(),
                    request_id: "answer-request".into(),
                    accepted: true,
                    error: None,
                };
                write_private_atomic(&runtime.join(ACK_FILE), &serde_json::to_vec(&ack).unwrap())
                    .unwrap();
                request
            });
            let result = submit_answer_at(
                &fixture.runtime,
                fixture.report.pid,
                7,
                &question_tool_use_id(7, &question.scope.session_id, &question.scope.tool_call_id),
                question.scope.dialog_id,
                "answer-request",
                &answers(),
                false,
                Duration::from_secs(3),
            );
            let request = responder.join().unwrap();
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(
                request
                    .get("process_start")
                    .and_then(|value| value.as_str()),
                process_start.as_deref()
            );
            assert_eq!(
                request.as_object().unwrap().contains_key("process_start"),
                process_start.is_some()
            );
        }
    }

    #[test]
    fn a_running_tab_whose_extension_never_reported_is_noticed_once() {
        let fixture = Fixture::new();
        let runtime = Some(fixture.runtime.as_path());
        let pid = Some(fixture.report.pid);
        let start = Instant::now();
        let after = |seconds: u64| start + Duration::from_secs(seconds);
        let late = SILENT_EXTENSION_AFTER.as_secs();

        let mut watch = SilentExtensionWatch::default();
        assert!(!watch.observe(runtime, pid, true, false, after(0)));
        assert!(!watch.observe(runtime, pid, true, false, after(late - 1)));
        assert!(watch.observe(runtime, pid, true, false, after(late)));
        // Once per provider process.
        assert!(!watch.observe(runtime, pid, true, false, after(late * 3)));
        // The tab's next agent is watched afresh.
        let next = pid.map(|pid| pid + 1);
        assert!(!watch.observe(runtime, next, true, false, after(late * 3)));
        assert!(watch.observe(runtime, next, true, false, after(late * 4)));

        // One report is enough, whatever follows: a tab that left its
        // session directory stops reporting and is not a silent extension.
        let mut watch = SilentExtensionWatch::default();
        assert!(!watch.observe(runtime, pid, true, true, after(0)));
        assert!(!watch.observe(runtime, pid, true, false, after(late * 2)));

        // Only time spent running counts, and a tab without a provider or
        // one that is not running is never noticed.
        let mut watch = SilentExtensionWatch::default();
        assert!(!watch.observe(runtime, pid, true, false, after(0)));
        assert!(!watch.observe(runtime, pid, false, false, after(late)));
        assert!(!watch.observe(runtime, pid, true, false, after(late + 1)));
        assert!(watch.observe(runtime, pid, true, false, after(late * 2 + 1)));
        let mut watch = SilentExtensionWatch::default();
        assert!(!watch.observe(runtime, None, true, false, after(0)));
        assert!(!watch.observe(runtime, None, true, false, after(late)));

        // A launch record from before the extension ran without `/proc`:
        // off Linux that extension is silent by design.
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.runtime.join(LAUNCH_FILE)).unwrap()).unwrap();
        record["features"] = serde_json::json!(["autopilot_v1"]);
        write_private_atomic(
            &fixture.runtime.join(LAUNCH_FILE),
            &serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let mut watch = SilentExtensionWatch::default();
        assert!(!watch.observe(runtime, pid, true, false, after(0)));
        assert!(!watch.observe(runtime, pid, true, false, after(late)));
        let mut watch = SilentExtensionWatch::default();
        assert!(!watch.observe(None, pid, true, false, after(0)));
        assert!(!watch.observe(None, pid, true, false, after(late)));
    }

    #[test]
    fn the_provider_is_told_which_process_spawned_it() {
        let fixture = Fixture::new();
        let launch = read_launch(&fixture.runtime).unwrap();
        // A CLI that itself runs inside an OMP tab inherited that tab's values.
        let mut env = vec![
            (RUNTIME_ENV.to_string(), "/elsewhere".to_string()),
            (LAUNCH_ENV.to_string(), Uuid::new_v4().to_string()),
            (PARENT_ENV.to_string(), "1".to_string()),
            ("KEPT".to_string(), "yes".to_string()),
        ];
        set_launch_env(&mut env, &fixture.runtime, &launch);
        let value = |name: &str| {
            let mut values = env.iter().filter(|(key, _)| key == name);
            let value = values.next().map(|(_, value)| value.clone());
            assert!(values.next().is_none(), "{name} is set twice");
            value
        };
        assert_eq!(value(PARENT_ENV), Some(std::process::id().to_string()));
        assert_eq!(value(LAUNCH_ENV), Some(launch.launch_id.to_string()));
        assert_eq!(
            value(RUNTIME_ENV),
            Some(fixture.runtime.to_string_lossy().into_owned())
        );
        assert_eq!(value("KEPT"), Some("yes".to_string()));
        // A spawned child sees this process as its parent, which is the
        // comparison the extension makes.
        let output = Command::new("sh")
            .args(["-c", "test \"$PPID\" = \"$APAS_OMP_PARENT\""])
            .envs(env.iter().map(|(key, value)| (key, value)))
            .status()
            .unwrap();
        assert!(output.success());
    }

    #[test]
    fn each_launch_records_the_autopilot_marker_and_clears_the_autopilot_files() {
        use crate::omp_autopilot as autopilot;

        let fixture = Fixture::new();
        let launch = read_launch(&fixture.runtime).unwrap();
        assert_eq!(launch.launch_id, fixture.report.launch_id);
        assert_eq!(
            launch.features,
            [autopilot::LAUNCH_FEATURE, PORTABLE_LAUNCH_FEATURE]
        );
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.runtime.join(LAUNCH_FILE)).unwrap()).unwrap();
        // An extension reads only `launch_id` and `session_dir` from this
        // record, and an older CLI looks for "autopilot_v1" among the
        // features, so a second one changes nothing for either.
        assert_eq!(
            record["features"],
            serde_json::json!(["autopilot_v1", "portable_v1"])
        );

        // An unapplied pause or stop is among them: it is not carried over.
        let files = [
            autopilot::CONTROLLER_FILE,
            autopilot::REQUEST_FILE,
            autopilot::STOP_FILE,
            autopilot::ACK_FILE,
            autopilot::STATE_FILE,
        ];
        for name in files {
            write_private_atomic(&fixture.runtime.join(name), b"{}").unwrap();
        }
        let (_, relaunched) = install(&fixture.runtime, &fixture.session_dir).unwrap();
        assert_ne!(relaunched.launch_id, launch.launch_id);
        for name in files {
            assert!(
                !fixture.runtime.join(name).exists(),
                "{name} survived a launch"
            );
        }
    }

    #[test]
    fn a_launch_record_from_before_autopilot_still_reads_and_has_no_marker() {
        let fixture = Fixture::new();
        let old = serde_json::json!({
            "launch_id": fixture.report.launch_id,
            "session_dir": fs::canonicalize(&fixture.session_dir).unwrap(),
        });
        write_private_atomic(
            &fixture.runtime.join(LAUNCH_FILE),
            &serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        let launch = read_launch(&fixture.runtime).unwrap();
        assert_eq!(launch.launch_id, fixture.report.launch_id);
        assert!(launch.features.is_empty());
        // Activity reporting, which predates the marker, is unaffected.
        fixture.publish();
        assert_eq!(fixture.activity(), Some(TranscriptActivity::PendingAnswer));
    }

    #[test]
    fn relaunch_invalidates_an_old_provider_report_even_if_it_reappears() {
        let fixture = Fixture::new();
        fixture.publish();
        install(&fixture.runtime, &fixture.session_dir).unwrap();
        assert_eq!(fixture.activity(), None);
        fixture.publish();
        assert_eq!(fixture.activity(), None);
    }

    #[test]
    fn a_live_report_identifies_a_new_session_before_its_first_message() {
        let mut fixture = Fixture::new();
        fs::remove_file(&fixture.transcript).unwrap();
        fixture.report.activity = TranscriptActivity::Working;
        fixture.publish();
        let report = read_activity(&fixture.runtime, fixture.report.pid).unwrap();
        assert_eq!(report.activity, TranscriptActivity::Working);
        assert_eq!(report.transcript_path, fixture.transcript);
        fixture.report.transcript_path = fixture.session_dir.join("new-session.jsonl");
        fixture.report.session_id = "new-session".into();
        fixture.report.activity = TranscriptActivity::Idle;
        fixture.publish();
        let report = read_activity(&fixture.runtime, fixture.report.pid).unwrap();
        assert_eq!(report.activity, TranscriptActivity::Idle);
        assert_eq!(report.transcript_path, fixture.report.transcript_path);
    }

    #[test]
    fn sibling_and_subagent_directories_cannot_claim_the_pane() {
        let mut fixture = Fixture::new();
        let subagents = fixture.session_dir.join("subagents");
        fs::create_dir(&subagents).unwrap();
        let nested = subagents.join("child.jsonl");
        fs::write(&nested, b"{}").unwrap();
        fixture.report.transcript_path = nested.clone();
        fixture.publish();
        assert_eq!(fixture.activity(), None);
        let outside = fixture.root.path().join("outside.jsonl");
        fs::write(&outside, b"{}").unwrap();
        fixture.report.transcript_path = outside.clone();
        fixture.publish();
        assert_eq!(fixture.activity(), None);
    }

    #[test]
    fn runtime_and_launch_permissions_are_part_of_report_authentication() {
        let fixture = Fixture::new();
        fixture.publish();
        fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(fixture.activity(), None);
        fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let alias = fixture.root.path().join("runtime-alias");
        symlink(&fixture.runtime, &alias).unwrap();
        assert!(read_activity(&alias, fixture.report.pid).is_none());
        fs::set_permissions(
            fixture.runtime.join(LAUNCH_FILE),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(fixture.activity(), None);
    }

    #[test]
    fn insecure_and_symlinked_reports_are_rejected() {
        let fixture = Fixture::new();
        fixture.publish();
        let path = fixture.runtime.join(REPORT_FILE);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(fixture.activity(), None);
        fs::remove_file(&path).unwrap();
        let outside = fixture.root.path().join("report.json");
        write_private_atomic(&outside, &serde_json::to_vec(&fixture.report).unwrap()).unwrap();
        symlink(&outside, &path).unwrap();
        assert_eq!(fixture.activity(), None);
    }

    #[test]
    fn symlinked_transcripts_and_corrupt_reports_are_rejected() {
        let fixture = Fixture::new();
        fixture.publish();
        fs::remove_file(&fixture.transcript).unwrap();
        let outside = fixture.root.path().join("outside.jsonl");
        fs::write(&outside, b"{}").unwrap();
        symlink(&outside, &fixture.transcript).unwrap();
        assert_eq!(fixture.activity(), None);
        fs::remove_file(&fixture.transcript).unwrap();
        fs::write(&fixture.transcript, b"{}").unwrap();
        write_private_atomic(&fixture.runtime.join(REPORT_FILE), b"{broken").unwrap();
        assert_eq!(fixture.activity(), None);
        write_private_atomic(&fixture.runtime.join(REPORT_FILE), &vec![b' '; 8193]).unwrap();
        assert_eq!(fixture.activity(), None);
    }
}
