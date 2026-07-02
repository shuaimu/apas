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

    let remote_base = resolve_remote_worktree_base(project_dir)?;

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
        .arg(&remote_base)
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

/// Resolve the current commit SHA at the tip of a worktree's branch
/// (`git rev-parse HEAD` in the worktree). Returns None when the call
/// fails for any reason — callers treat that as "no change to report".
fn current_head_sha(worktree_path: &str) -> Option<String> {
    let out = run_git_cd(worktree_path, &["rev-parse", "HEAD"]).ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Snapshot used by the diff poll loop. Holds the last-seen SHA per
/// pane so we only re-emit `PaneDiff` when the branch tip moves.
pub struct DiffPollState {
    last_seen: std::collections::HashMap<u32, String>,
}

impl DiffPollState {
    pub fn new() -> Self {
        Self {
            last_seen: std::collections::HashMap::new(),
        }
    }
}

impl Default for DiffPollState {
    fn default() -> Self {
        Self::new()
    }
}

/// One iteration of the auto-refresh poller. Given a snapshot of
/// (pane_id, worktree_path) entries, run `rev-parse HEAD` on each one;
/// if the SHA differs from the previous tick, recompute the diff and
/// emit it via the returned vector of (pane_id, branch, base, diff).
/// Also reaps `last_seen` entries for panes that disappeared so the
/// state doesn't grow unboundedly. Phase 1.2b.
pub fn poll_changed_diffs(
    project_dir: &Path,
    state: &mut DiffPollState,
    panes_with_worktrees: &[(u32, String)],
) -> Vec<(u32, String, String, String)> {
    let mut out = Vec::new();
    let live_ids: std::collections::HashSet<u32> =
        panes_with_worktrees.iter().map(|(id, _)| *id).collect();
    state.last_seen.retain(|id, _| live_ids.contains(id));

    for (pane_id, wt) in panes_with_worktrees {
        let sha = match current_head_sha(wt) {
            Some(s) => s,
            None => continue,
        };
        let changed = match state.last_seen.get(pane_id) {
            Some(prev) => prev != &sha,
            None => true,
        };
        if !changed {
            continue;
        }
        state.last_seen.insert(*pane_id, sha);
        match compute_pane_diff(project_dir, Some(wt)) {
            Ok((branch, base, diff)) => out.push((*pane_id, branch, base, diff)),
            Err(err) => {
                tracing::warn!(
                    pane_id,
                    error = %err,
                    "poll_changed_diffs: compute_pane_diff failed; skipping emission this tick",
                );
            }
        }
    }
    out
}

/// Compute the diff for a pane's isolated worktree branch against the
/// project's HEAD. Returns (branch_name, base_ref, diff_text). Phase 1.2a.
///
/// Both args use string paths so the WebSocket-receiving call site doesn't
/// have to juggle Path types. `worktree_path` is None when the pane has no
/// isolated worktree — that's a "polite error" case.
pub fn compute_pane_diff(
    project_dir: &Path,
    worktree_path: Option<&str>,
) -> Result<(String, String, String)> {
    let worktree = worktree_path.ok_or_else(|| {
        anyhow!("pane has no isolated worktree — nothing to diff. Use the \"Isolated git worktree\" checkbox when creating the pane, or `apas worktree add <pane-id>`.")
    })?;
    let project_str = project_dir.to_str().ok_or_else(|| {
        anyhow!("project dir is not valid UTF-8: {}", project_dir.display())
    })?;
    let branch = current_branch_in(worktree).ok_or_else(|| {
        anyhow!("worktree at {} is on detached HEAD; nothing to diff", worktree)
    })?;
    // Diff three-dot syntax (A...B) shows what's on the worktree branch
    // since it diverged from the project's HEAD — the right semantics for
    // "what did this pane change?".
    let base_ref = "HEAD".to_string();
    let diff = run_git_cd(
        project_str,
        &["diff", &format!("{}...{}", base_ref, branch)],
    )?;
    Ok((branch, base_ref, diff))
}

/// Push the pane's branch to `origin` and open a GitHub PR via
/// `gh pr create --fill`. Returns the new PR URL.
///
/// Preconditions: the pane has an isolated worktree, the worktree is on a
/// branch (not detached HEAD), the project has an `origin` remote, and
/// `gh` is installed + authenticated on the CLI host.
pub fn create_pr_for_pane(worktree_path: Option<&str>) -> Result<String> {
    let worktree = worktree_path.ok_or_else(|| {
        anyhow!("pane has no isolated worktree — nothing to PR. Add a worktree to this pane first.")
    })?;
    let branch = current_branch_in(worktree).ok_or_else(|| {
        anyhow!("worktree at {} is on detached HEAD; cannot push or open a PR", worktree)
    })?;

    // 1. Push the branch to origin (creating the remote ref if needed). The
    //    `-u` keeps subsequent pushes from this worktree simple.
    run_git_cd(worktree, &["push", "-u", "origin", &branch])?;

    // 2. gh pr create --fill auto-fills the title and body from commits. We
    //    intentionally don't specify --base — gh picks the repo's default
    //    branch, which is what the user expects.
    let out = Command::new("gh")
        .args(["pr", "create", "--fill"])
        .current_dir(worktree)
        .output()
        .with_context(|| {
            format!(
                "running `gh pr create --fill` in {} (is `gh` installed and authenticated? run `gh auth login` on the CLI host)",
                worktree
            )
        })?;
    if !out.status.success() {
        return Err(anyhow!(
            "gh pr create failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // gh prints the PR URL on its last non-empty line of stdout.
    let url = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find(|l| l.starts_with("http"))
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow!(
                "gh pr create succeeded but did not print a URL: {}",
                String::from_utf8_lossy(&out.stdout).trim()
            )
        })?;
    Ok(url)
}

fn resolve_remote_worktree_base(project_dir: &Path) -> Result<String> {
    run_git_path(project_dir, &["fetch", "origin"])
        .context("fetching origin before creating isolated worktree")?;

    if run_git_path(
        project_dir,
        &["rev-parse", "--verify", "--quiet", "origin/HEAD^{commit}"],
    )
    .is_ok()
    {
        return Ok("origin/HEAD".to_string());
    }

    if run_git_path(
        project_dir,
        &["rev-parse", "--verify", "--quiet", "origin/master^{commit}"],
    )
    .is_ok()
    {
        return Ok("origin/master".to_string());
    }

    Err(anyhow!(
        "could not resolve remote base after fetching origin: neither origin/HEAD nor origin/master exists",
    ))
}

/// Run a `git -C <dir> <args…>` command and return its stdout on success.
fn run_git_path(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("running `git {}` in {}", args.join(" "), cwd.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git {} (in {}) failed: {}",
            args.join(" "),
            cwd.display(),
            String::from_utf8_lossy(&out.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run a `git -C <dir> <args…>` command and return its stdout on success.
fn run_git_cd(cwd: &str, args: &[&str]) -> Result<String> {
    run_git_path(Path::new(cwd), args)
}

/// Canonical `host/owner/repo` grouping key for a project's `origin` remote,
/// used by the web sidebar to group projects that belong to the same repo.
///
/// Returns `None` when `working_dir` is not a git repo, has no `origin`
/// remote, or `git` is unavailable — all of which map to the sidebar's
/// "(no remote)" group rather than an error.
pub fn normalized_git_remote(working_dir: &Path) -> Option<String> {
    let raw = run_git_path(working_dir, &["remote", "get-url", "origin"]).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(normalize_remote_url(trimmed))
}

/// Pure canonicalization of a git remote URL into a lowercase `host/owner/repo`
/// key. The three common shapes for the same repo collapse to one string:
///   `git@github.com:Owner/Repo.git`     -> `github.com/owner/repo`
///   `https://github.com/owner/repo.git` -> `github.com/owner/repo`
///   `ssh://git@github.com/owner/repo`   -> `github.com/owner/repo`
fn normalize_remote_url(raw: &str) -> String {
    // Trim trailing slashes, strip a single `.git`, then trim slashes again so
    // `repo`, `repo/`, `repo.git`, and `repo.git/` all collapse to one key.
    let s = raw.trim().trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s).trim_end_matches('/');

    let (host, path) = if let Some(idx) = s.find("://") {
        // scheme://[user@]authority[:port]/owner/repo...
        let rest = &s[idx + 3..];
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        // An IPv6 literal authority is `[addr]:port`; the address itself
        // contains colons, so pull it out of the brackets before the port split.
        let host = if let Some(rest) = authority.strip_prefix('[') {
            rest.split_once(']').map_or(authority, |(addr, _)| addr)
        } else {
            authority.split_once(':').map_or(authority, |(h, _)| h)
        };
        (host.to_string(), path.to_string())
    } else if let Some((before, after)) = s.split_once(':') {
        // scp-like `[user@]host:owner/repo` — only when the host part has no `/`.
        if before.contains('/') {
            (String::new(), s.to_string())
        } else {
            let host = before.rsplit_once('@').map_or(before, |(_, h)| h);
            (host.to_string(), after.to_string())
        }
    } else {
        // Bare `owner/repo`, a local path, or anything unparseable.
        (String::new(), s.to_string())
    };

    let mut combined = if host.is_empty() {
        path
    } else {
        format!("{host}/{path}")
    };
    while combined.contains("//") {
        combined = combined.replace("//", "/");
    }
    combined.trim_matches('/').to_lowercase()
}

/// The raw `origin` URL (scheme/user/auth preserved) for `working_dir`, or None
/// when there's no origin / no git. Unlike `normalized_git_remote` (a lossy
/// grouping key) this keeps the exact URL so the repo can be cloned elsewhere.
pub fn raw_git_remote(working_dir: &Path) -> Option<String> {
    let raw = run_git_path(working_dir, &["remote", "get-url", "origin"]).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Clone `url` into `dest` (which must not already exist). Runs
/// non-interactively (`GIT_TERMINAL_PROMPT=0`) so a missing credential fails
/// fast with an error instead of hanging on a terminal prompt.
pub fn clone_repo(url: &str, dest: &Path) -> Result<()> {
    let out = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("clone")
        .arg(url)
        .arg(dest)
        .output()
        .with_context(|| format!("running `git clone` into {}", dest.display()))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Create and switch to a fresh branch in the freshly-cloned repo at `dest`,
/// auto-suffixing (`-2`, `-3`…) when the desired name already exists (e.g. the
/// user picked the repo's default branch). Returns the branch actually created.
pub fn checkout_unique_branch(dest: &Path, desired: &str) -> Result<String> {
    if run_git_path(dest, &["checkout", "-b", desired]).is_ok() {
        return Ok(desired.to_string());
    }
    // Bounded retry: a sane collision needs only a few suffixes, and an
    // intrinsically-invalid ref would otherwise spawn git hundreds of times.
    for n in 2..30 {
        let candidate = format!("{desired}-{n}");
        if run_git_path(dest, &["checkout", "-b", &candidate]).is_ok() {
            return Ok(candidate);
        }
    }
    Err(anyhow!("could not create a unique branch from '{desired}'"))
}

/// Return `desired` if free, else the first `<name>-N` (N≥2) that doesn't
/// exist — the directory auto-suffix used when an instance name collides.
pub fn unique_dir(desired: &Path) -> PathBuf {
    if !desired.exists() {
        return desired.to_path_buf();
    }
    let parent = desired.parent().unwrap_or_else(|| Path::new("."));
    let stem = desired
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("instance");
    for n in 2..10_000 {
        let candidate = parent.join(format!("{stem}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    desired.to_path_buf()
}

/// Sanitize a user-supplied instance name into a safe single path component:
/// alphanumerics plus `-_.`, with any other char folded to `-`, and leading/
/// trailing `-`/`.` stripped so `..`/`.` can't escape the projects root.
pub fn sanitize_instance_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '-' || c == '.').to_string();
    if cleaned.is_empty() {
        "instance".to_string()
    } else {
        cleaned
    }
}

/// Sanitize a user-supplied branch name into a valid-ish git ref: allow
/// `-_/.` plus alphanumerics, collapse `..`, and strip leading/trailing
/// `-`/`/`/`.` (which git refs disallow).
pub fn sanitize_branch_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut cleaned = cleaned.replace("..", "-");
    // Collapse `//` (an empty ref component) and `/.` (a component can't start
    // with `.`); git rejects both, and `checkout -b` would then fail for every
    // suffix candidate.
    while cleaned.contains("//") {
        cleaned = cleaned.replace("//", "/");
    }
    while cleaned.contains("/.") {
        cleaned = cleaned.replace("/.", "/");
    }
    let cleaned = cleaned
        .trim_matches(|c| c == '-' || c == '/' || c == '.')
        .to_string();
    if cleaned.is_empty() {
        "apas-instance".to_string()
    } else {
        cleaned
    }
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

    #[test]
    fn normalize_remote_url_canonicalizes_common_forms() {
        // All three shapes for the same repo collapse to one key.
        assert_eq!(
            normalize_remote_url("git@github.com:Owner/Repo.git"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("https://github.com/owner/repo.git"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("ssh://git@github.com/owner/repo"),
            "github.com/owner/repo"
        );
        // Trailing slash, case-folding, and ports.
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo/"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("ssh://git@github.com:22/owner/repo.git"),
            "github.com/owner/repo"
        );
        // Multi-segment paths (self-hosted GitLab groups) keep their slashes.
        assert_eq!(
            normalize_remote_url("git@gitlab.example.com:team/sub/proj.git"),
            "gitlab.example.com/team/sub/proj"
        );
        // A trailing slash AFTER `.git` must still strip the `.git` so the repo
        // collapses with its slash-less form.
        assert_eq!(
            normalize_remote_url("https://github.com/owner/repo.git/"),
            "github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("git@github.com:owner/repo.git/"),
            "github.com/owner/repo"
        );
        // IPv6-literal hosts: the bracketed address is the host, not `[`, and
        // distinct addresses stay in distinct groups.
        assert_eq!(
            normalize_remote_url("ssh://git@[::1]:22/owner/repo.git"),
            "::1/owner/repo"
        );
        assert_ne!(
            normalize_remote_url("ssh://git@[::1]:22/owner/repo"),
            normalize_remote_url("ssh://git@[::2]:22/owner/repo")
        );
    }

    #[test]
    fn sanitize_instance_name_strips_traversal_and_separators() {
        assert_eq!(sanitize_instance_name("my repo!"), "my-repo");
        assert_eq!(sanitize_instance_name("../etc"), "etc");
        assert_eq!(sanitize_instance_name(".."), "instance");
        assert_eq!(sanitize_instance_name("  ok-1.2  "), "ok-1.2");
        assert_eq!(sanitize_instance_name(""), "instance");
    }

    #[test]
    fn sanitize_branch_name_makes_valid_ref() {
        assert_eq!(sanitize_branch_name("feature/foo bar"), "feature/foo-bar");
        assert_eq!(sanitize_branch_name("/leading/"), "leading");
        assert_eq!(sanitize_branch_name("a..b"), "a-b");
        assert_eq!(sanitize_branch_name(""), "apas-instance");
        // `//` and `/.` collapse to valid single separators (git rejects them).
        assert_eq!(sanitize_branch_name("feature//foo"), "feature/foo");
        assert_eq!(sanitize_branch_name("feature/.hidden"), "feature/hidden");
    }

    #[test]
    fn unique_dir_suffixes_on_collision() {
        let tmp = TempDir::new().expect("tmpdir");
        let want = tmp.path().join("proj");
        // Free name returns unchanged.
        assert_eq!(unique_dir(&want), want);
        // Occupied name auto-suffixes.
        std::fs::create_dir(&want).unwrap();
        assert_eq!(unique_dir(&want), tmp.path().join("proj-2"));
    }

    #[test]
    fn normalized_git_remote_is_none_without_origin() {
        // A fresh git repo with no `origin` remote -> None (the "(no remote)" group).
        let tmp = TempDir::new().expect("tmpdir");
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(args)
                .status()
                .expect("git");
        };
        run(&["init", "-q", "-b", "main"]);
        assert_eq!(normalized_git_remote(tmp.path()), None);
    }

    #[test]
    fn normalized_git_remote_reads_origin() {
        let tmp = TempDir::new().expect("tmpdir");
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(args)
                .status()
                .expect("git");
            assert!(status.success(), "git {} failed", args.join(" "));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&[
            "remote",
            "add",
            "origin",
            "git@github.com:Shuaimu/APAS.git",
        ]);
        assert_eq!(
            normalized_git_remote(tmp.path()).as_deref(),
            Some("github.com/shuaimu/apas")
        );
    }

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

    fn run_git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn run_git_stdout(cwd: &Path, args: &[&str]) -> String {
        run_git_path(cwd, args).expect("git stdout").trim().to_string()
    }

    #[test]
    fn create_for_pane_fetches_origin_and_bases_branch_on_remote_tip() {
        let tmp = TempDir::new().expect("tmpdir");
        let origin = tmp.path().join("origin.git");
        let seed = tmp.path().join("seed");
        let proj = tmp.path().join("proj");

        let status = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg("-q")
            .arg("-b")
            .arg("master")
            .arg(&origin)
            .status()
            .expect("git init bare");
        assert!(status.success());

        let status = Command::new("git")
            .arg("clone")
            .arg("-q")
            .arg(&origin)
            .arg(&seed)
            .status()
            .expect("git clone seed");
        assert!(status.success());
        run_git(&seed, &["config", "user.email", "t@e"]);
        run_git(&seed, &["config", "user.name", "t"]);
        std::fs::write(seed.join("initial.txt"), b"initial").unwrap();
        run_git(&seed, &["add", "initial.txt"]);
        run_git(&seed, &["commit", "-q", "-m", "initial"]);
        run_git(&seed, &["push", "-q", "origin", "master"]);

        let status = Command::new("git")
            .arg("clone")
            .arg("-q")
            .arg(&origin)
            .arg(&proj)
            .status()
            .expect("git clone proj");
        assert!(status.success());
        let stale_local_head = run_git_stdout(&proj, &["rev-parse", "master"]);

        std::fs::write(seed.join("origin-only.txt"), b"new remote tip").unwrap();
        run_git(&seed, &["add", "origin-only.txt"]);
        run_git(&seed, &["commit", "-q", "-m", "origin only"]);
        run_git(&seed, &["push", "-q", "origin", "master"]);
        let remote_tip = run_git_stdout(&seed, &["rev-parse", "HEAD"]);
        assert_ne!(stale_local_head, remote_tip, "local master must be stale");

        let wt = create_for_pane(&proj, 7, Some("apas-pane-7"), None).expect("create worktree");
        let created_head = run_git_cd(&wt, &["rev-parse", "HEAD"])
            .expect("created worktree head")
            .trim()
            .to_string();
        assert_eq!(created_head, remote_tip);
        assert!(
            Path::new(&wt).join("origin-only.txt").exists(),
            "worktree should include the remote-only commit",
        );
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
    fn poll_changed_diffs_only_fires_on_sha_change_and_reaps_gone_panes() {
        let (_tmp, proj, wt) = setup_repo_with_worktree();
        let wt_str = wt.to_str().unwrap().to_string();
        let mut state = DiffPollState::new();

        let panes = vec![(2u32, wt_str.clone())];
        // First poll: SHA hasn't been seen, so we emit even with no
        // new commits (the diff text will just be empty).
        let first = poll_changed_diffs(&proj, &mut state, &panes);
        assert_eq!(first.len(), 1, "first tick should emit baseline");
        assert_eq!(first[0].0, 2);

        // Second poll without changes: nothing should fire.
        let second = poll_changed_diffs(&proj, &mut state, &panes);
        assert!(second.is_empty(), "no SHA change should produce no emissions");

        // Commit something on the branch — third poll should fire.
        std::fs::write(wt.join("a.txt"), b"a").unwrap();
        assert!(Command::new("git")
            .arg("-C").arg(&wt_str)
            .args(["add", "a.txt"]).status().unwrap().success());
        assert!(Command::new("git")
            .arg("-C").arg(&wt_str)
            .args(["-c", "user.email=t@e", "-c", "user.name=t", "commit", "-m", "a"])
            .status().unwrap().success());
        let third = poll_changed_diffs(&proj, &mut state, &panes);
        assert_eq!(third.len(), 1, "branch tip moved → should re-emit");
        assert!(third[0].3.contains("a.txt"));

        // Pane disappears: last_seen entry should be reaped.
        assert!(state.last_seen.contains_key(&2));
        let _ = poll_changed_diffs(&proj, &mut state, &[]);
        assert!(!state.last_seen.contains_key(&2), "stale entry should be dropped");
    }

    #[test]
    fn compute_pane_diff_returns_branch_changes() {
        let (_tmp, proj, wt) = setup_repo_with_worktree();
        // Put a commit on the worktree branch.
        std::fs::write(wt.join("feature.txt"), b"hello").unwrap();
        let wt_str = wt.to_str().unwrap();
        assert!(Command::new("git")
            .arg("-C").arg(wt_str)
            .args(["add", "feature.txt"])
            .status().unwrap().success());
        assert!(Command::new("git")
            .arg("-C").arg(wt_str)
            .args(["-c", "user.email=t@e", "-c", "user.name=t", "commit", "-m", "feature"])
            .status().unwrap().success());

        let (branch, base, diff) = compute_pane_diff(&proj, Some(wt_str)).expect("diff");
        assert_eq!(branch, "apas-pane-2");
        assert_eq!(base, "HEAD");
        assert!(diff.contains("feature.txt"), "diff should mention the new file: {}", diff);
        assert!(diff.contains("+hello"), "diff should show the addition: {}", diff);
    }

    #[test]
    fn compute_pane_diff_errors_without_worktree() {
        let (_tmp, proj, _wt) = setup_repo_with_worktree();
        let err = compute_pane_diff(&proj, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no isolated worktree"), "{}", msg);
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
