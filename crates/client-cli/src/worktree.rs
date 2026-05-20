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

use crate::project;

const DEFAULT_WORKTREES_BASE: &str = ".apas-worktrees";

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

    let worktree_path = match custom_path {
        Some(p) => p,
        None => project_dir
            .join(DEFAULT_WORKTREES_BASE)
            .join(format!("pane-{}", pane_id)),
    };

    if worktree_path.exists() {
        return Err(anyhow!(
            "worktree path {} already exists; pick --path elsewhere or remove the existing dir",
            worktree_path.display(),
        ));
    }

    let branch_name = branch.unwrap_or_else(|| format!("apas-pane-{}", pane_id));

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
        .arg(&branch_name)
        .status()
        .context("running `git worktree add` — is this a git repo?")?;

    if !status.success() {
        return Err(anyhow!(
            "`git worktree add` failed (exit {:?})",
            status.code(),
        ));
    }

    // Canonicalize so the saved path is absolute and survives the user cd-ing
    // elsewhere. Phase 1.1b's spawn site passes this verbatim to .current_dir,
    // so any caller-side cd would otherwise break it.
    let abs_path = worktree_path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", worktree_path.display()))?;
    let abs_path_str = abs_path.to_string_lossy().to_string();

    if let Some(pane_mut) = metadata.get_pane_mut(pane_id) {
        pane_mut.worktree_path = Some(abs_path_str.clone());
    }
    project::save_project(project_dir, &metadata).context("saving .apas")?;

    println!(
        "✓ Created worktree for pane {} at {} (branch {})",
        pane_id, abs_path_str, branch_name,
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
