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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct QuestionScope {
    launch_id: Uuid,
    pid: i32,
    process_start: String,
    session_id: String,
    transcript_path: PathBuf,
    tool_call_id: String,
    dialog_id: Uuid,
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

fn pane_runtime_dir(project_id: Uuid, pane_id: u32) -> Option<PathBuf> {
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
        || process_start(pid).as_deref() != Some(scope.process_start.as_str())
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
struct Launch {
    launch_id: Uuid,
    session_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Report {
    launch_id: Uuid,
    pid: i32,
    process_start: String,
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
            env.retain(|(key, _)| key != RUNTIME_ENV && key != LAUNCH_ENV);
            env.push((
                RUNTIME_ENV.into(),
                runtime_dir.to_string_lossy().into_owned(),
            ));
            env.push((LAUNCH_ENV.into(), launch.launch_id.to_string()));
            Some(extension)
        }
        Err(error) => {
            tracing::warn!(%error, pane_id, "could not prepare OMP activity extension; using transcript status");
            None
        }
    }
}

fn install(runtime_dir: &Path, session_dir: &Path) -> Result<(PathBuf, Launch)> {
    if !private_metadata(runtime_dir, true) {
        bail!("insecure OMP runtime directory {}", runtime_dir.display());
    }
    let launch = Launch {
        launch_id: Uuid::new_v4(),
        session_dir: fs::canonicalize(session_dir)?,
    };
    // Invalidate the old generation before an old provider can publish another report.
    write_private_atomic(
        &runtime_dir.join(LAUNCH_FILE),
        &serde_json::to_vec(&launch)?,
    )?;
    for name in [REPORT_FILE, QUESTION_FILE, SUBMISSION_FILE, ACK_FILE] {
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
        || process_start(pid).as_deref() != Some(report.process_start.as_str())
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

fn read_private_json_bounded<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Option<T> {
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

fn private_metadata(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        (if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        }) && private_owner(&metadata)
    })
}

#[cfg(unix)]
fn private_owner(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private_owner(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn process_start(pid: i32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let mut fields = fields.split_whitespace();
    // Field 3 is state; field 5 must be the provider's dedicated process group.
    if matches!(fields.next()?, "Z" | "X" | "x") {
        return None;
    }
    fields.next()?;
    if fields.next()?.parse::<i32>().ok()? != pid {
        return None;
    }
    // Fields 6..21 precede starttime (22), which disambiguates PID reuse.
    Some(fields.nth(16)?.to_owned())
}

#[cfg(not(target_os = "linux"))]
fn process_start(_pid: i32) -> Option<String> {
    None
}

fn write_private_atomic(path: &Path, body: &[u8]) -> Result<()> {
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

#[cfg(all(test, target_os = "linux"))]
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
            let runtime = root.path().join("runtime");
            let session_dir = root.path().join("sessions");
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
                process_start: process_start(pid).unwrap(),
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
        question.scope.process_start.push('0');
        fixture.write_question(&question);
        assert!(read_question(&fixture.runtime, fixture.report.pid).is_none());
        question.scope.process_start = fixture.report.process_start.clone();
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
        fixture.report.process_start.push('0');
        fixture.publish();
        assert_eq!(fixture.activity(), None);
        fixture.report.process_start = process_start(fixture.report.pid).unwrap();
        fixture.publish();
        fixture.provider.kill().unwrap();
        fixture.provider.wait().unwrap();
        assert_eq!(fixture.activity(), None);
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
