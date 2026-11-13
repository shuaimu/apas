use super::*;
use shared::{JobDiffFile, JobStopEvidence};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;

/// Only the positively stopped worker's ordinary source tree is inspected. Its
/// .git directory is deliberately neither opened nor copied.
pub async fn freeze_candidate(
    ctx: &DeliveryContext,
    stopped: &JobStopEvidence,
    fence: &DeliveryFence,
) -> Result<JobReview> {
    let _lock = ctx.lock()?;
    let mut record = ctx.load_or_create()?;
    ensure_retained(&record)?;
    if let Some(candidate) = &record.candidate {
        let sha = candidate.candidate_sha.clone();
        candidate::validate_candidate(ctx, candidate, fence).await?;
        build_diffs(ctx, &mut record, &sha, fence).await?;
        if record.review.verification.is_none() {
            record.review.reason = None;
        }
        ctx.save(&record)?;
        return Ok(record.review);
    }
    if !stopped.descendants_stopped || stopped.container_id.is_empty() || stopped.boot_id.is_empty()
    {
        bail!("candidate_requires_positive_worker_stop");
    }
    DateTime::parse_from_rfc3339(&stopped.stopped_at).context("invalid_worker_stop_time")?;
    let result = capture(ctx, &mut record, fence).await;
    if let Err(error) = &result {
        record.review.reason = Some(format!("candidate_capture_failed: {error}"));
        ctx.save(&record)?;
    }
    result?;
    ctx.save(&record)?;
    Ok(record.review)
}

async fn capture(
    ctx: &DeliveryContext,
    record: &mut DeliveryRecord,
    fence: &DeliveryFence,
) -> Result<()> {
    fence.check()?;
    if !valid_sha(&ctx.task.base_sha) {
        bail!("invalid_admitted_base_sha");
    }
    let output = fs::canonicalize(&ctx.worker_output).context("worker_output_missing")?;
    let root_meta = fs::symlink_metadata(&ctx.worker_output)?;
    if !root_meta.is_dir() || root_meta.uid() != unsafe { libc::geteuid() } {
        bail!("unsafe_worker_output_root");
    }
    let source = fs::canonicalize(&ctx.trusted_source_repo).context("trusted_source_missing")?;
    if source.starts_with(&output) || output.starts_with(&source) && source == output {
        bail!("trusted_source_is_worker_output");
    }
    if ctx.root().starts_with(&output) || ctx.data_root().starts_with(&output) {
        bail!("evidence_inside_worker_boundary");
    }
    let snapshot = ctx.data_root().join("frozen");
    private_dir(&snapshot)?;
    let mut entries = Vec::new();
    let mut total = 0;
    capture_directory(
        &output,
        &output,
        &snapshot,
        &mut entries,
        &mut total,
        ctx.limits.workspace_bytes,
        root_meta.dev(),
        fence,
    )?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    if ctx.repo().exists() {
        // A prior interrupted capture may leave a trusted partial repository.
        // It contains no accepted candidate and can be recreated without touching
        // the worker output or a previously accepted result.
        fs::remove_dir_all(ctx.repo())?;
    }
    let mut init = git_command(ctx, None);
    init.args(["init", "--bare", "--template="]).arg(ctx.repo());
    run_git(init, Vec::new(), 4096, fence).await?;
    let source_text = source.to_str().context("non_utf8_source_path")?;
    git(
        ctx,
        &[
            "fetch",
            "--depth=1",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--",
            source_text,
            &ctx.task.base_sha,
        ],
        Vec::new(),
        4096,
        fence,
    )
    .await?;
    let base = git_text(
        git(
            ctx,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", ctx.task.base_sha),
            ],
            Vec::new(),
            128,
            fence,
        )
        .await?,
    )?;
    if base != ctx.task.base_sha {
        bail!("admitted_base_object_mismatch");
    }
    let base_tree = git(
        ctx,
        &["ls-tree", "-r", "-z", &base],
        Vec::new(),
        8 * 1024 * 1024,
        fence,
    )
    .await?;
    if base_tree.truncated {
        bail!("base_manifest_exceeds_bound");
    }
    for entry in base_tree
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
    {
        if entry.starts_with(b"160000 ") {
            bail!("submodule_candidate_not_supported");
        }
    }
    let mut index = Vec::new();
    for entry in &mut entries {
        fence.check()?;
        let bytes = frozen_bytes(&snapshot, entry)?;
        entry.blob = git_text(
            git(
                ctx,
                &["hash-object", "-w", "--stdin", "--no-filters"],
                bytes,
                128,
                fence,
            )
            .await?,
        )?;
        if !valid_sha(&entry.blob) {
            bail!("invalid_candidate_blob");
        }
        index.extend_from_slice(
            format!("{} {}\t{}\0", entry.mode, entry.blob, entry.path).as_bytes(),
        );
    }
    git(ctx, &["read-tree", "--empty"], Vec::new(), 4096, fence).await?;
    git(
        ctx,
        &["update-index", "-z", "--index-info"],
        index,
        4096,
        fence,
    )
    .await?;
    let tree = git_text(git(ctx, &["write-tree"], Vec::new(), 128, fence).await?)?;
    let content_digest = digest(&serde_json::to_vec(&entries)?);
    let message = format!(
        "APAS immutable candidate\n\nBase: {}\nContent-SHA256: {}\n",
        base, content_digest
    );
    let sha = git_text(
        git(
            ctx,
            &["commit-tree", &tree, "-p", &base],
            message.into_bytes(),
            128,
            fence,
        )
        .await?,
    )?;
    if !valid_sha(&sha) {
        bail!("invalid_candidate_commit");
    }
    git(
        ctx,
        &["update-ref", "refs/heads/candidate", &sha],
        Vec::new(),
        4096,
        fence,
    )
    .await?;
    let manifest = CandidateManifest {
        base_sha: base, candidate_sha: sha.clone(), tree_sha: tree, content_digest,
        files: entries,
        exclusions: vec!["Root .git administrative metadata is excluded; all other regular files and safe relative symlinks, including ignored/untracked files, are captured.".into()],
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    if manifest_bytes.len() as u64 > ctx.limits.output_bytes {
        bail!("candidate_manifest_exceeds_output_allowance");
    }
    if manifest_bytes.len() > shared::JOB_MAX_FRAME_BYTES / 2 {
        bail!("candidate_manifest_exceeds_transport_bound");
    }
    ctx.retain(
        record,
        JobArtifactKind::Candidate,
        &manifest_bytes,
        "application/json",
        false,
    )?;
    record.review.candidate_sha = Some(sha.clone());
    record.candidate = Some(manifest);
    // Persist identity before optional diff generation so cancellation cannot
    // discard an already constructed immutable candidate.
    ctx.save(record)?;
    build_diffs(ctx, record, &sha, fence).await?;
    record.review.reason = None;
    Ok(())
}

fn capture_directory(
    root: &Path,
    directory: &Path,
    snapshot: &Path,
    entries: &mut Vec<CandidateEntry>,
    total: &mut u64,
    max_bytes: u64,
    device: u64,
    fence: &DeliveryFence,
) -> Result<()> {
    for child in fs::read_dir(directory)? {
        fence.check()?;
        let child = child?;
        let path = child.path();
        let relative = path.strip_prefix(root)?;
        if relative == Path::new(".git") {
            continue;
        }
        let name = safe_path(relative)?;
        if relative.components().count() > 128 {
            bail!("candidate_directory_depth_exceeded");
        }
        let meta = fs::symlink_metadata(&path)?;
        if meta.dev() != device {
            bail!("candidate_crosses_filesystem: {name}");
        }
        if meta.is_dir() {
            private_dir(&snapshot.join(relative))?;
            capture_directory(
                root, &path, snapshot, entries, total, max_bytes, device, fence,
            )?;
            continue;
        }
        if entries.len() >= MAX_FILES {
            bail!("candidate_file_count_exceeded");
        }
        let (mode, bytes) = if meta.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            validate_symlink(relative, &target)?;
            if let Ok(resolved) = fs::canonicalize(&path) {
                if !resolved.starts_with(root) {
                    bail!("candidate_symlink_escape: {name}");
                }
            }
            ("120000", target.as_os_str().as_bytes().to_vec())
        } else if meta.is_file() {
            if meta.nlink() != 1 {
                bail!("candidate_hardlink_rejected: {name}");
            }
            if meta.len() > max_bytes.saturating_sub(*total) {
                bail!("candidate_workspace_bound_exceeded: {name}");
            }
            if meta.len() > 64 * 1024 * 1024 {
                bail!("candidate_file_exceeds_64_mib_bound: {name}");
            }
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?;
            let opened = file.metadata()?;
            if opened.ino() != meta.ino() || opened.dev() != meta.dev() {
                bail!("candidate_mutated_during_capture: {name}");
            }
            let mut bytes = Vec::with_capacity(meta.len().min(1024 * 1024) as usize);
            (&mut file).take(meta.len() + 1).read_to_end(&mut bytes)?;
            let after = file.metadata()?;
            if bytes.len() as u64 != meta.len()
                || after.mtime() != meta.mtime()
                || after.mtime_nsec() != meta.mtime_nsec()
                || after.ctime_nsec() != meta.ctime_nsec()
            {
                bail!("candidate_mutated_during_capture: {name}");
            }
            (
                if meta.mode() & 0o111 != 0 {
                    "100755"
                } else {
                    "100644"
                },
                bytes,
            )
        } else {
            bail!("candidate_special_file_rejected: {name}");
        };
        *total = total
            .checked_add(bytes.len() as u64)
            .context("candidate_size_overflow")?;
        if *total > max_bytes {
            bail!("candidate_workspace_bound_exceeded: {name}");
        }
        // Symlinks are stored as ordinary files in the evidence snapshot; no
        // subsequent host read can follow a worker-selected target.
        atomic_bytes(&snapshot.join(relative), &bytes)?;
        entries.push(CandidateEntry {
            path: name,
            mode: mode.into(),
            bytes: bytes.len() as u64,
            sha256: digest(&bytes),
            blob: String::new(),
        });
    }
    Ok(())
}

fn safe_path(path: &Path) -> Result<String> {
    use std::path::Component;
    if path.as_os_str().len() > 4096 {
        bail!("candidate_path_too_long");
    }
    for component in path.components() {
        let Component::Normal(part) = component else {
            bail!("candidate_unsafe_path");
        };
        let part = part.to_str().context("candidate_non_utf8_path")?;
        if part.eq_ignore_ascii_case(".git")
            || part.contains('\\')
            || part.bytes().any(|b| b < 32 || b == 127)
        {
            bail!("candidate_unsafe_path");
        }
    }
    let text = path.to_str().context("candidate_non_utf8_path")?;
    if text.is_empty() {
        bail!("candidate_empty_path");
    }
    Ok(text.to_owned())
}

fn validate_symlink(path: &Path, target: &Path) -> Result<()> {
    use std::path::Component;
    if target.as_os_str().is_empty() || target.is_absolute() {
        bail!("candidate_symlink_escape");
    }
    let mut depth = path.parent().map_or(0, |p| p.components().count());
    for component in target.components() {
        match component {
            Component::Normal(name) => {
                if name
                    .to_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(".git"))
                {
                    bail!("candidate_symlink_git_metadata");
                }
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => bail!("candidate_symlink_escape"),
        }
    }
    Ok(())
}

fn frozen_bytes(snapshot: &Path, entry: &CandidateEntry) -> Result<Vec<u8>> {
    safe_path(Path::new(&entry.path))?;
    let file = private_open(&snapshot.join(&entry.path), false)?;
    let mut bytes = Vec::new();
    file.take(entry.bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != entry.bytes || digest(&bytes) != entry.sha256 {
        bail!("candidate_content_digest_mismatch");
    }
    Ok(bytes)
}

pub(super) async fn validate_candidate(
    ctx: &DeliveryContext,
    manifest: &CandidateManifest,
    fence: &DeliveryFence,
) -> Result<()> {
    if manifest.base_sha != ctx.task.base_sha
        || digest(&serde_json::to_vec(&manifest.files)?) != manifest.content_digest
    {
        bail!("candidate_manifest_binding_mismatch");
    }
    let commit = git_text(
        git(
            ctx,
            &["cat-file", "commit", &manifest.candidate_sha],
            Vec::new(),
            8192,
            fence,
        )
        .await?,
    )?;
    let expected = format!("tree {}\nparent {}\n", manifest.tree_sha, manifest.base_sha);
    if !commit.starts_with(&expected)
        || commit.lines().filter(|l| l.starts_with("parent ")).count() != 1
    {
        bail!("candidate_fixed_parent_mismatch");
    }
    let actual = git_text(
        git(
            ctx,
            &["rev-parse", "--verify", "refs/heads/candidate"],
            Vec::new(),
            128,
            fence,
        )
        .await?,
    )?;
    if actual != manifest.candidate_sha {
        bail!("candidate_ref_changed");
    }
    let tree = git(
        ctx,
        &["ls-tree", "-r", "-z", &manifest.candidate_sha],
        Vec::new(),
        8 * 1024 * 1024,
        fence,
    )
    .await?;
    if tree.truncated {
        bail!("candidate_tree_exceeds_bound");
    }
    let expected: Vec<String> = manifest
        .files
        .iter()
        .map(|e| format!("{} blob {}\t{}", e.mode, e.blob, e.path))
        .collect();
    let mut actual: Vec<&[u8]> = tree
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    actual.sort_unstable();
    let mut expected: Vec<&[u8]> = expected.iter().map(|s| s.as_bytes()).collect();
    expected.sort_unstable();
    if actual != expected {
        bail!("candidate_tree_manifest_mismatch");
    }
    for entry in &manifest.files {
        frozen_bytes(&ctx.data_root().join("frozen"), entry)?;
    }
    Ok(())
}

pub(super) fn materialize(
    ctx: &DeliveryContext,
    manifest: &CandidateManifest,
    checkout: &Path,
) -> Result<()> {
    private_dir(checkout)?;
    for entry in &manifest.files {
        let bytes = frozen_bytes(&ctx.data_root().join("frozen"), entry)?;
        let path = checkout.join(&entry.path);
        private_dir(path.parent().context("candidate_path_parent_missing")?)?;
        if entry.mode == "120000" {
            let target = Path::new(std::ffi::OsStr::from_bytes(&bytes));
            validate_symlink(Path::new(&entry.path), target)?;
            symlink(target, &path)?;
        } else {
            atomic_bytes(&path, &bytes)?;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if entry.mode == "100755" { 0o700 } else { 0o600 }),
            )?;
        }
    }
    Ok(())
}

async fn build_diffs(
    ctx: &DeliveryContext,
    record: &mut DeliveryRecord,
    sha: &str,
    fence: &DeliveryFence,
) -> Result<()> {
    let output = git(
        ctx,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--numstat",
            "-z",
            &ctx.task.base_sha,
            sha,
            "--",
        ],
        Vec::new(),
        8 * 1024 * 1024,
        fence,
    )
    .await?;
    if output.truncated {
        bail!("candidate_diff_manifest_exceeds_bound");
    }
    let previous = std::mem::take(&mut record.review.files);
    for row in output.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let row = std::str::from_utf8(row)?;
        let mut fields = row.splitn(3, '\t');
        let added = fields.next().context("invalid_git_numstat")?;
        let deleted = fields.next().context("invalid_git_numstat")?;
        let path = fields.next().context("invalid_git_numstat")?;
        safe_path(Path::new(path))?;
        if let Some(existing) = previous.iter().find(|file| file.path == path) {
            record.review.files.push(existing.clone());
            continue;
        }
        let binary = added == "-" || deleted == "-";
        let used = record.artifacts.iter().map(|a| a.size_bytes).sum::<u64>();
        // Leave half the artifact allowance for independent check logs and
        // publication evidence; per-file truncation remains visible.
        let available = (ctx.limits.output_bytes / 2)
            .saturating_sub(used)
            .min(2 * 1024 * 1024) as usize;
        let diff = git(
            ctx,
            &[
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--no-color",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--unified=3",
                &ctx.task.base_sha,
                sha,
                "--",
                path,
            ],
            Vec::new(),
            available,
            fence,
        )
        .await?;
        let text = String::from_utf8_lossy(&diff.stdout);
        let artifact = ctx.retain(
            record,
            JobArtifactKind::Diff,
            text.as_bytes(),
            "text/x-diff; charset=utf-8",
            diff.truncated,
        )?;
        record.review.files.push(JobDiffFile {
            path: path.into(),
            additions: if binary { 0 } else { added.parse()? },
            deletions: if binary { 0 } else { deleted.parse()? },
            binary,
            artifact_id: Some(artifact.id),
            unavailable_reason: artifact
                .truncated
                .then(|| "diff_truncated_by_output_allowance".into()),
        });
        ctx.save(record)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_git_metadata_and_symlink_escapes_are_rejected() {
        for path in [
            "../outside",
            "/outside",
            ".git/config",
            "nested/.GiT/hooks/x",
            "a\\b",
            "a\nb",
        ] {
            assert!(safe_path(Path::new(path)).is_err(), "{path}");
        }
        assert!(validate_symlink(Path::new("src/link"), Path::new("../../secret")).is_err());
        assert!(validate_symlink(Path::new("link"), Path::new("/etc/passwd")).is_err());
        assert!(validate_symlink(Path::new("src/link"), Path::new("../.git/config")).is_err());
        assert!(validate_symlink(Path::new("src/link"), Path::new("../README")).is_ok());
    }

    #[test]
    fn untracked_files_survive_capture_but_git_hooks_do_not() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let frozen = tmp.path().join("frozen");
        private_dir(&source).unwrap();
        private_dir(&frozen).unwrap();
        fs::write(source.join("new-untracked.rs"), "pub fn added() {}\n").unwrap();
        fs::create_dir(source.join(".git")).unwrap();
        fs::write(source.join(".git/config"), "[core]\n hooksPath=/host\n").unwrap();
        let fence = DeliveryFence::new(
            Arc::new(AtomicBool::new(false)),
            Utc::now() + ChronoDuration::minutes(1),
        );
        let mut entries = Vec::new();
        let mut total = 0;
        capture_directory(
            &source,
            &source,
            &frozen,
            &mut entries,
            &mut total,
            1024,
            fs::metadata(&source).unwrap().dev(),
            &fence,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "new-untracked.rs");
        assert_eq!(
            fs::read(frozen.join("new-untracked.rs")).unwrap(),
            b"pub fn added() {}\n"
        );
        assert!(!frozen.join(".git").exists());
    }

    #[test]
    fn special_files_and_out_of_boundary_links_block_the_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        let frozen = tmp.path().join("frozen");
        private_dir(&source).unwrap();
        private_dir(&frozen).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(source.join("socket")).unwrap();
        let fence = DeliveryFence::new(
            Arc::new(AtomicBool::new(false)),
            Utc::now() + ChronoDuration::minutes(1),
        );
        let mut entries = Vec::new();
        let mut total = 0;
        assert!(capture_directory(
            &source,
            &source,
            &frozen,
            &mut entries,
            &mut total,
            1024,
            fs::metadata(&source).unwrap().dev(),
            &fence
        )
        .is_err());
        fs::remove_file(source.join("socket")).unwrap();
        symlink("../../outside", source.join("link")).unwrap();
        assert!(capture_directory(
            &source,
            &source,
            &frozen,
            &mut entries,
            &mut total,
            1024,
            fs::metadata(&source).unwrap().dev(),
            &fence
        )
        .is_err());
    }

    async fn source_git(
        ctx: &DeliveryContext,
        args: &[&str],
        input: &[u8],
        fence: &DeliveryFence,
    ) -> String {
        let mut command = git_command(ctx, Some(&ctx.trusted_source_repo));
        command.args(args);
        git_text(run_git(command, input.to_vec(), 8192, fence).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn complete_candidate_is_deterministic_fixed_parent_and_survives_worker_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ctx = DeliveryContext {
            identity: JobRunIdentity {
                project_id: "project".into(),
                job_id: Uuid::new_v4(),
                run_id: Uuid::new_v4(),
                cluster_owner_user_id: Uuid::new_v4(),
                machine_id: Uuid::new_v4(),
                epoch: 1,
            },
            task: JobTaskSpec {
                worker: None,
                instruction: "change files".into(),
                acceptance_criteria: vec![],
                source_url: None,
                repository: "owner/repo".into(),
                base_ref: "main".into(),
                base_sha: String::new(),
                worker_profile: "worker".into(),
                worker_profile_digest: "w".into(),
                verification_profile: "checks".into(),
                verification_profile_digest: "v".into(),
                model_profile: "model".into(),
            },
            limits: JobLimits::default(),
            run_dir: tmp.path().join("run-one"),
            workspace_root: tmp.path().join("slot-one"),
            trusted_source_repo: tmp.path().join("trusted.git"),
            worker_output: tmp.path().join("worker"),
        };
        private_dir(&ctx.root()).unwrap();
        private_dir(&ctx.worker_output).unwrap();
        let fence = DeliveryFence::new(
            Arc::new(AtomicBool::new(false)),
            Utc::now() + ChronoDuration::minutes(2),
        );
        let mut init = git_command(&ctx, None);
        init.args(["init", "--bare", "--template="])
            .arg(&ctx.trusted_source_repo);
        run_git(init, Vec::new(), 8192, &fence).await.unwrap();
        let blob = source_git(&ctx, &["hash-object", "-w", "--stdin"], b"old\n", &fence).await;
        let tree = source_git(
            &ctx,
            &["mktree"],
            format!("100644 blob {blob}\tREADME\n100644 blob {blob}\tremoved.txt\n").as_bytes(),
            &fence,
        )
        .await;
        ctx.task.base_sha = source_git(&ctx, &["commit-tree", &tree], b"base\n", &fence).await;
        source_git(
            &ctx,
            &["update-ref", "refs/heads/main", &ctx.task.base_sha],
            b"",
            &fence,
        )
        .await;
        fs::write(ctx.worker_output.join("README"), "changed\n").unwrap();
        fs::write(
            ctx.worker_output.join("untracked.rs"),
            "pub fn new_source() {}\n",
        )
        .unwrap();
        let git_dir = ctx.worker_output.join(".git");
        fs::create_dir_all(git_dir.join("hooks")).unwrap();
        let sentinel = tmp.path().join("worker-hook-executed");
        let hook = format!("#!/bin/sh\ntouch '{}'\n", sentinel.display());
        fs::write(git_dir.join("hooks/pre-commit"), hook).unwrap();
        fs::set_permissions(
            git_dir.join("hooks/pre-commit"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::write(
            git_dir.join("config"),
            "[core]\n hooksPath=hooks\n[remote \"origin\"]\n url=ssh://untrusted.invalid/repo\n",
        )
        .unwrap();
        let stopped = JobStopEvidence {
            boot_id: "boot".into(),
            container_id: "worker".into(),
            stopped_at: Utc::now().to_rfc3339(),
            reason: "settled".into(),
            descendants_stopped: true,
        };
        let first = freeze_candidate(&ctx, &stopped, &fence).await.unwrap();
        assert_eq!(
            first
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            ["README", "removed.txt", "untracked.rs"]
        );
        assert!(!sentinel.exists());
        let commit = git_text(
            git(
                &ctx,
                &[
                    "cat-file",
                    "commit",
                    first.candidate_sha.as_deref().unwrap(),
                ],
                Vec::new(),
                8192,
                &fence,
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert!(commit.contains(&format!("\nparent {}\n", ctx.task.base_sha)));
        let mut second_ctx = ctx.clone();
        second_ctx.run_dir = tmp.path().join("run-two");
        second_ctx.workspace_root = tmp.path().join("slot-two");
        let second = freeze_candidate(&second_ctx, &stopped, &fence)
            .await
            .unwrap();
        assert_eq!(first.candidate_sha, second.candidate_sha);
        fs::remove_dir_all(&ctx.worker_output).unwrap();
        let record = ctx.load().unwrap();
        let JobWorkerReport::ArtifactChunk { data_base64, .. } =
            read_artifact_chunk(&ctx, record.review.files[0].artifact_id.unwrap(), 0, 4096)
                .unwrap()
        else {
            panic!("expected retained diff chunk")
        };
        let diff = base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .unwrap();
        assert!(String::from_utf8(diff).unwrap().contains("+changed"));
        fs::write(ctx.data_root().join("frozen/README"), "tampered").unwrap();
        assert!(
            validate_candidate(&ctx, record.candidate.as_ref().unwrap(), &fence)
                .await
                .is_err()
        );
    }
}
