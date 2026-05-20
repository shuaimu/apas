//! `apas worktree` subcommand — manage per-pane git worktrees (Phase 1.1c).
//!
//! Workflow:
//!   1. `apas worktree add <pane-id> [branch]` runs `git worktree add` and
//!      writes the resulting absolute path into `PaneConfig.worktree_path`.
//!   2. Phase 1.1b's spawn-cwd plumbing then uses that path as the agent's
//!      cwd on the next pane restart — claude's session jsonl, the auto-wake
//!      tmp dir, and the .current_dir() all agree because they key off the
//!      same string.
//!
//! Restart is not automatic — the user is told to close+reopen the tab (or
//! reboot the CLI) to pick up the new cwd. Live in-process restart can come
//! in a later leaf if it proves annoying.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use shared::PaneCleanupAction;

use crate::project;

const DEFAULT_WORKTREES_BASE: &str = ".apas-worktrees";

/// Materialize a fresh git worktree for `pane_id` under
/// `<project>/.apas-worktrees/pane-<id>` on branch `apas-pane-<id>`. Returns
/// the canonicalized absolute path on success. Used by both the `apas
/// worktree add` subcommand and the Phase 1.1e "create with isolated
/// worktree" AddPane flow.
pub fn create_for_pane(
    project_dir: &Path,
    pane_id: u32,
    branch: Option<&str>,
    custom_path: Option<&Path>,
) -> Result<String> {
    let worktree_path: PathBuf = match custom_path {
        Some(p) => p.to_path_buf(),
        None => project_dir
            .join(DEFAULT_WORKTREES_BASE)
            .join(format!("pane-{}", pane_id)),
    };

    if worktree_path.exists() {
        return Err(anyhow!(
            "worktree path {} already exists; pick a different location or remove it first",
            worktree_path.display(),
        ));
    }

    let owned_branch;
    let branch_name: &str = match branch {
        Some(b) => b,
        None => {
            owned_branch = format!("apas-pane-{}", pane_id);
            &owned_branch
        }
    };

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(project_dir)
        .arg("worktree")
        .arg("add")
        .arg(&worktree_path)
        .arg("-b")
        .arg(branch_name)
        .status()
        .context("running `git worktree add` — is this a git repo?")?;

    if !status.success() {
        return Err(anyhow!(
            "`git worktree add` failed (exit {:?})",
            status.code(),
        ));
    }

    let abs_path = worktree_path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", worktree_path.display()))?;
    Ok(abs_path.to_string_lossy().into_owned())
}

pub fn add(
    project_dir: &Path,
    pane_id: u32,
    branch: Option<String>,
    custom_path: Option<PathBuf>,
) -> Result<()> {
    let mut metadata = project::get_or_create_project(project_dir)
        .context("reading .apas")?;
    metadata.migrate_legacy();

    let pane = metadata
        .get_pane(pane_id)
        .ok_or_else(|| anyhow!("pane {} not found in this project's .apas", pane_id))?;

    if let Some(existing) = pane.worktree_path.as_deref() {
        return Err(anyhow!(
            "pane {} already has worktree_path = {}; run `apas worktree remove {}` first",
            pane_id,
            existing,
            pane_id,
        ));
    }

    let abs_path_str = create_for_pane(
        project_dir,
        pane_id,
        branch.as_deref(),
        custom_path.as_deref(),
    )?;
    let display_branch = branch
        .clone()
        .unwrap_or_else(|| format!("apas-pane-{}", pane_id));

    if let Some(pane_mut) = metadata.get_pane_mut(pane_id) {
        pane_mut.worktree_path = Some(abs_path_str.clone());
    }
    project::save_project(project_dir, &metadata).context("saving .apas")?;

    println!(
        "✓ Created worktree for pane {} at {} (branch {})",
        pane_id, abs_path_str, display_branch,
    );
    println!(
        "Restart the pane (close + re-add the tab, or reboot the apas CLI) so the next spawn picks up the new cwd.",
    );
    Ok(())
}

pub fn remove(project_dir: &Path, pane_id: u32) -> Result<()> {
    let mut metadata = project::get_or_create_project(project_dir)
        .context("reading .apas")?;

    let pane = metadata
        .get_pane_mut(pane_id)
        .ok_or_else(|| anyhow!("pane {} not found in this project's .apas", pane_id))?;

    let prev = pane.worktree_path.take();
    project::save_project(project_dir, &metadata).context("saving .apas")?;

    match prev {
        Some(p) => {
            println!("✓ Cleared worktree assignment for pane {} (was: {}).", pane_id, p);
            println!(
                "The git worktree itself was NOT removed. To delete it: `git worktree remove {}`.",
                p,
            );
        }
        None => println!("pane {} had no worktree assignment.", pane_id),
    }
    Ok(())
}

pub fn list(project_dir: &Path) -> Result<()> {
    let metadata = project::get_or_create_project(project_dir)
        .context("reading .apas")?;

    let mut found = false;
    for pane in &metadata.panes {
        if let Some(path) = pane.worktree_path.as_deref() {
            let label = pane.label.as_deref().unwrap_or("?");
            println!("pane {} ({}): {}", pane.pane_id, label, path);
            found = true;
        }
    }
    if !found {
        println!("no worktree assignments in {}", project_dir.display());
    }
    Ok(())
}

/// Run a `git -C <dir> <args…>` command and return its stdout on success.
fn run_git_cd(cwd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("running `git {}` in {}", args.join(" "), cwd))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git {} (in {}) failed: {}",
            args.join(" "),
            cwd,
            String::from_utf8_lossy(&out.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Resolve the current branch in a worktree. Returns None for detached HEAD.
fn current_branch_in(worktree: &str) -> Option<String> {
    let out = run_git_cd(worktree, &["branch", "--show-current"]).ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Called from the CloseTab handler when a pane with an isolated worktree
/// is being torn down. Returns a user-facing one-line summary to surface in
/// the chat stream (success OR safe-fallback). True errors propagate via Err.
///
/// Behaviour by action (see [`PaneCleanupAction`] doc-comments):
///   - Discard: `worktree remove --force` + `branch -D <branch>`.
///   - MergeAndRemove: merge --no-ff <branch> into project HEAD, then
///     remove worktree + delete branch. Aborts on merge conflict.
///   - LeaveAsBranch: `worktree remove` without --force. Uncommitted
///     changes => report and leave the dir on disk; branch is kept.
pub fn cleanup_on_close(
    project_dir: &str,
    worktree_path: &str,
    action: PaneCleanupAction,
) -> Result<String> {
    let branch = current_branch_in(worktree_path);

    match action {
        PaneCleanupAction::Discard => {
            run_git_cd(
                project_dir,
                &["worktree", "remove", "--force", worktree_path],
            )?;
            let branch_msg = if let Some(b) = branch.as_deref() {
                // Branch deletion is best-effort; if it fails (e.g. detached or
                // missing) we still consider the discard successful overall.
                match run_git_cd(project_dir, &["branch", "-D", b]) {
                    Ok(_) => format!(" + branch '{}' deleted", b),
                    Err(err) => format!(" (branch '{}' could not be deleted: {})", b, err),
                }
            } else {
                String::new()
            };
            Ok(format!(
                "[Worktree {} discarded (force-removed){}]",
                worktree_path, branch_msg,
            ))
        }
        PaneCleanupAction::MergeAndRemove => {
            let b = branch.as_deref().ok_or_else(|| {
                anyhow!(
                    "cannot merge: worktree {} has no current branch (detached HEAD?)",
                    worktree_path,
                )
            })?;
            run_git_cd(project_dir, &["merge", "--no-ff", b])
                .with_context(|| format!("merging '{}' into project HEAD", b))?;
            run_git_cd(
                project_dir,
                &["worktree", "remove", "--force", worktree_path],
            )?;
            run_git_cd(project_dir, &["branch", "-D", b])?;
            Ok(format!(
                "[Branch '{}' merged into HEAD; worktree {} removed]",
                b, worktree_path,
            ))
        }
        PaneCleanupAction::LeaveAsBranch => {
            // No --force: removal fails on uncommitted/untracked changes. We
            // catch that and tell the user — losing those changes silently
            // would be the worst outcome for the "safe" option.
            match run_git_cd(project_dir, &["worktree", "remove", worktree_path]) {
                Ok(_) => Ok(format!(
                    "[Worktree {} removed; branch '{}' kept for manual review]",
                    worktree_path,
                    branch.as_deref().unwrap_or("?"),
                )),
                Err(err) => Ok(format!(
                    "[Worktree {} has uncommitted changes, not removed: {}. Branch '{}' kept. Clean up manually with `git worktree remove --force {}` once you've saved any changes.]",
                    worktree_path,
                    err,
                    branch.as_deref().unwrap_or("?"),
                    worktree_path,
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Set up `<tmp>/proj` as a git repo with one commit on `main`, then run
    /// `apas worktree add 2 -b apas-pane-2` style git plumbing to materialize
    /// the worktree. Returns (tmp guard, project_dir, worktree_path).
    fn setup_repo_with_worktree() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = TempDir::new().expect("tmpdir");
        let proj = tmp.path().join("proj");
        std::fs::create_dir(&proj).unwrap();
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&proj)
                .args(args)
                .status()
                .expect("git");
            assert!(status.success(), "git {} failed", args.join(" "));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let wt = proj.join(".apas-worktrees").join("pane-2");
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        run(&[
            "worktree",
            "add",
            wt.to_str().unwrap(),
            "-b",
            "apas-pane-2",
        ]);
        (tmp, proj, wt)
    }

    #[test]
    fn cleanup_leave_as_branch_clean_worktree_removes_dir_keeps_branch() {
        let (_tmp, proj, wt) = setup_repo_with_worktree();
        let proj_str = proj.to_str().unwrap();
        let wt_str = wt.to_str().unwrap();
        let msg = cleanup_on_close(proj_str, wt_str, PaneCleanupAction::LeaveAsBranch)
            .expect("leave should not error");
        assert!(!wt.exists(), "worktree dir should be gone");
        assert!(msg.contains("removed"), "msg: {}", msg);
        // Branch should still exist
        let branches = run_git_cd(proj_str, &["branch", "--list", "apas-pane-2"]).unwrap();
        assert!(branches.contains("apas-pane-2"), "branch should remain");
    }

    #[test]
    fn cleanup_leave_as_branch_dirty_worktree_keeps_dir() {
        let (_tmp, proj, wt) = setup_repo_with_worktree();
        // Drop an uncommitted file in the worktree.
        std::fs::write(wt.join("dirty.txt"), b"work in progress").unwrap();
        let proj_str = proj.to_str().unwrap();
        let wt_str = wt.to_str().unwrap();
        let msg = cleanup_on_close(proj_str, wt_str, PaneCleanupAction::LeaveAsBranch)
            .expect("dirty path should still return Ok with a guidance message");
        assert!(wt.exists(), "dirty worktree dir should be preserved");
        assert!(
            msg.contains("uncommitted") || msg.contains("Clean up"),
            "expected guidance, got: {}",
            msg,
        );
    }

    #[test]
    fn cleanup_discard_removes_dir_and_branch() {
        let (_tmp, proj, wt) = setup_repo_with_worktree();
        // Discard MUST win even with uncommitted changes — that's its whole point.
        std::fs::write(wt.join("dirty.txt"), b"throwaway").unwrap();
        let proj_str = proj.to_str().unwrap();
        let wt_str = wt.to_str().unwrap();
        cleanup_on_close(proj_str, wt_str, PaneCleanupAction::Discard).expect("discard");
        assert!(!wt.exists(), "worktree dir gone");
        let branches = run_git_cd(proj_str, &["branch", "--list", "apas-pane-2"]).unwrap();
        assert!(!branches.contains("apas-pane-2"), "branch deleted");
    }

    #[test]
    fn cleanup_merge_and_remove_brings_commits_into_main() {
        let (_tmp, proj, wt) = setup_repo_with_worktree();
        // Add a commit in the worktree's branch.
        std::fs::write(wt.join("feature.txt"), b"hello from pane").unwrap();
        let wt_str = wt.to_str().unwrap();
        let status = Command::new("git")
            .arg("-C")
            .arg(wt_str)
            .args(["add", "feature.txt"])
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("git")
            .arg("-C")
            .arg(wt_str)
            .args([
                "-c",
                "user.email=t@e",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "feature",
            ])
            .status()
            .unwrap();
        assert!(status.success());

        let proj_str = proj.to_str().unwrap();
        // Configure user on the main repo too so merge can author the merge commit.
        let _ = Command::new("git")
            .arg("-C")
            .arg(proj_str)
            .args(["config", "user.email", "t@e"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(proj_str)
            .args(["config", "user.name", "t"])
            .status();

        cleanup_on_close(proj_str, wt_str, PaneCleanupAction::MergeAndRemove)
            .expect("merge_and_remove");
        assert!(!wt.exists());
        let branches = run_git_cd(proj_str, &["branch", "--list", "apas-pane-2"]).unwrap();
        assert!(!branches.contains("apas-pane-2"), "branch deleted post-merge");
        // The feature file should now be in main.
        assert!(proj.join("feature.txt").exists(), "merge brought in the file");
    }
}
