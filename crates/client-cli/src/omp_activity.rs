//! Private, process-scoped activity published by an explicitly loaded OMP extension.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::transcript::TranscriptActivity;

const RUNTIME_ENV: &str = "APAS_OMP_RUNTIME";
const LAUNCH_ENV: &str = "APAS_OMP_LAUNCH";
const LAUNCH_FILE: &str = "omp-launch.json";
const REPORT_FILE: &str = "omp-activity.json";
const EXTENSION_FILE: &str = "omp-activity.mjs";
const EXTENSION: &str = include_str!("omp_activity.mjs");
const MAX_REPORT_BYTES: u64 = 8192;

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
    match fs::remove_file(runtime_dir.join(REPORT_FILE)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
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
    let runtime_dir = crate::config::Config::runtime_dir()
        .ok()?
        .join("panes")
        .join(crate::pane_host::short_uuid(project_id))
        .join(pane_id.to_string());
    read_activity(&runtime_dir, process_group_id?)
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
    if !metadata.is_file() || metadata.len() > MAX_REPORT_BYTES || !private_owner(&metadata) {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
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
        options.mode(0o600);
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
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.provider.kill();
            let _ = self.provider.wait();
        }
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
