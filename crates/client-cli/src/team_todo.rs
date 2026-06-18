//! Tech-Lead-driven workflow document: parse + serialize `team-todo.md`.
//!
//! Schema is documented in `docs/todo-driven-workflow.md`. Short version:
//!
//! ```markdown
//! # Team TODO
//!
//! ## Global TODOs
//!
//! ### [TODO-001] Title goes here
//! status: approved
//! origin: user
//! pr: (not yet)
//!
//! Free-form body paragraph.
//!
//! ## pane:578 — backend-engineer
//!
//! ### [TODO-001 · backend-1] Subtask title
//! status: in_progress
//! parent: TODO-001
//!
//! Subtask body.
//! ```
//!
//! Both the Tech Lead and the user are allowed to hand-edit this file.
//! The parser is intentionally forgiving: unknown fields are silently
//! folded into the body so a future Tech Lead version that adds new
//! fields doesn't silently lose data when an older one rewrites the
//! file. Round-trip stability is enforced by tests below.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const TEAM_TODO_FILENAME: &str = "team-todo.md";
static TEAM_TODO_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn team_todo_path(project_dir: &Path) -> PathBuf {
    project_dir.join(TEAM_TODO_FILENAME)
}

/// Read the team-todo doc. Returns an empty `TeamTodo` if the file is
/// missing — first-run from a project that doesn't have one yet is
/// indistinguishable from "nothing to do."
pub fn load(project_dir: &Path) -> Result<TeamTodo> {
    let path = team_todo_path(project_dir);
    if !path.exists() {
        return Ok(TeamTodo::default());
    }
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    parse(&s)
}

/// Write the team-todo doc atomically (write-tmp-then-rename) so a
/// crash mid-write doesn't truncate the user's queue.
pub fn save(project_dir: &Path, todo: &TeamTodo) -> Result<()> {
    let path = team_todo_path(project_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let body = serialize(todo);
    let tmp = team_todo_tmp_path(&path);
    std::fs::write(&tmp, body)
        .with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(err) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err)
            .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()));
    }
    Ok(())
}

fn team_todo_tmp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(TEAM_TODO_FILENAME);
    let counter = TEAM_TODO_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        "{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        counter
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TeamTodo {
    pub globals: Vec<GlobalTodo>,
    pub workers: Vec<WorkerSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalTodo {
    pub id: String,
    pub title: String,
    pub status: GlobalStatus,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// One PR per contributing worker pane. Per-worker because a Global
    /// TODO can split across multiple panes — each pane has its own
    /// branch in its own worktree, and we don't try to merge them into
    /// a single integration branch. The user reviews N PRs in GitHub.
    /// Empty `Vec` (or no `pr:` lines in the doc) means no PR opened
    /// yet. Single-worker Globals just have one entry.
    pub prs: Vec<PaneTodoPr>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneTodoPr {
    pub pane_id: u32,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalStatus {
    Proposed,
    Approved,
    InProgress,
    UnderReview,
    PrOpen,
    Done,
    Rejected,
    /// Tech-Lead-only terminal state. Tech Lead may flip a `proposed`
    /// entry to `withdrawn` when surveying surfaces evidence that the
    /// work has already landed (manual edits, parallel PRs from other
    /// panes, etc.). Distinct from `rejected` (a user verdict) so the
    /// UI can fold/distinguish them. Allowed transition: proposed →
    /// withdrawn only.
    Withdrawn,
}

impl GlobalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::InProgress => "in_progress",
            Self::UnderReview => "under_review",
            Self::PrOpen => "pr_open",
            Self::Done => "done",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "proposed" => Self::Proposed,
            "approved" => Self::Approved,
            "in_progress" => Self::InProgress,
            "under_review" => Self::UnderReview,
            "pr_open" => Self::PrOpen,
            "done" => Self::Done,
            "rejected" => Self::Rejected,
            "withdrawn" => Self::Withdrawn,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    User,
    TechLead,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::TechLead => "tech-lead",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "user" => Self::User,
            "tech-lead" | "tech_lead" => Self::TechLead,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSection {
    pub pane_id: u32,
    pub role_hint: Option<String>,
    pub subtasks: Vec<WorkerSubtask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSubtask {
    pub id: String,
    pub title: String,
    pub status: SubStatus,
    pub parent: String,
    pub body: String,
}

/// Convert our internal `TeamTodo` into the wire-format
/// `shared::TeamTodoStateMsg` (statuses as strings, web-friendly).
/// `project_dir` is used to read the agents' cursor files so the web
/// can render "Tech Lead is processing records up to X".
pub fn to_wire_with_cursors(todo: &TeamTodo, project_dir: &Path) -> shared::TeamTodoStateMsg {
    let mut msg = to_wire(todo);
    msg.tech_lead_cursor = read_cursor(project_dir, ".apas-tech-lead-cursor");
    msg.reviewer_cursor = read_cursor(project_dir, ".apas-reviewer-cursor");
    msg
}

fn read_cursor(project_dir: &Path, filename: &str) -> Option<String> {
    let path = project_dir.join(filename);
    let raw = std::fs::read_to_string(&path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn to_wire(todo: &TeamTodo) -> shared::TeamTodoStateMsg {
    shared::TeamTodoStateMsg {
        globals: todo
            .globals
            .iter()
            .map(|g| shared::TeamTodoGlobalMsg {
                id: g.id.clone(),
                title: g.title.clone(),
                status: g.status.as_str().to_string(),
                origin: g.origin.as_str().to_string(),
                prs: g
                    .prs
                    .iter()
                    .map(|p| shared::PaneTodoPrMsg {
                        pane_id: p.pane_id,
                        url: p.url.clone(),
                        annotation: p.annotation.clone(),
                    })
                    .collect(),
                body: g.body.clone(),
            })
            .collect(),
        workers: todo
            .workers
            .iter()
            .map(|w| shared::TeamTodoWorkerMsg {
                pane_id: w.pane_id,
                role_hint: w.role_hint.clone(),
                subtasks: w
                    .subtasks
                    .iter()
                    .map(|s| shared::TeamTodoSubtaskMsg {
                        id: s.id.clone(),
                        title: s.title.clone(),
                        status: s.status.as_str().to_string(),
                        parent: s.parent.clone(),
                        body: s.body.clone(),
                    })
                    .collect(),
            })
            .collect(),
        // Filled in by to_wire_with_cursors; the cursor-free helper is
        // kept for tests + callers that don't have a project_dir handy.
        tech_lead_cursor: None,
        reviewer_cursor: None,
    }
}

/// What the Tech Lead's next iteration should do, derived from the
/// current `team-todo.md`. Used by the Tech Lead loop and web surfaces
/// to summarize expansion, dispatch, and review handoff work.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NextActions {
    /// Oldest `approved` global TODO with no subtasks yet — Tech Lead
    /// should expand this into per-worker subtasks.
    pub expand_next: Option<String>,
    /// Per-worker hint: this subtask is next to dispatch (one per pane
    /// that has a `pending` subtask and no `in_progress` / `revising`).
    pub dispatch: Vec<DispatchHint>,
    /// Globals whose subtasks are all `done` / `approved` and which
    /// should now flip to `under_review` (Phase 3 spawns a Reviewer).
    pub ready_for_review: Vec<String>,
    /// Globals stuck on `proposed` waiting for user approval. Tech Lead
    /// can re-ping the Manager about these.
    pub pending_proposals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchHint {
    pub pane_id: u32,
    pub role_hint: Option<String>,
    pub subtask_id: String,
    pub subtask_title: String,
    pub subtask_body: String,
    pub parent_global: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatus {
    Pending,
    InProgress,
    Done,
    Reviewing,
    Revising,
    Approved,
}

impl SubStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Reviewing => "reviewing",
            Self::Revising => "revising",
            Self::Approved => "approved",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            "reviewing" => Self::Reviewing,
            "revising" => Self::Revising,
            "approved" => Self::Approved,
            _ => return None,
        })
    }
}

// ---------- parse ----------

/// Active section while parsing.
enum Section {
    Outside,
    Global,
    Worker { pane_id: u32, role_hint: Option<String> },
}

/// Parse a `team-todo.md` document. Missing / malformed sections are
/// tolerated by skipping the offending entry rather than failing the
/// whole parse — we'd rather lose one TODO entry than blank the user's
/// queue because of one typo.
pub fn parse(input: &str) -> Result<TeamTodo> {
    let mut todo = TeamTodo::default();
    let mut current_section = Section::Outside;
    // Currently-accumulating worker section so we can push subtasks
    // into it as we see them. Flushed at the next `## ` heading.
    let mut current_worker: Option<WorkerSection> = None;

    let mut lines = input.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("# ") {
            let _ = rest; // top-of-doc title; we ignore content
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            // Flush any in-progress worker before transitioning.
            if let Some(w) = current_worker.take() {
                todo.workers.push(w);
            }
            let rest = rest.trim();
            if rest.eq_ignore_ascii_case("global todos") {
                current_section = Section::Global;
            } else if let Some((pane_id, role_hint)) = parse_worker_heading(rest) {
                current_section = Section::Worker {
                    pane_id,
                    role_hint: role_hint.clone(),
                };
                current_worker = Some(WorkerSection {
                    pane_id,
                    role_hint,
                    subtasks: Vec::new(),
                });
            } else {
                current_section = Section::Outside;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            let Some((id, title)) = parse_item_heading(rest.trim()) else {
                continue;
            };
            // Gather key:value lines + body until the next heading.
            let mut fields: Vec<(String, String)> = Vec::new();
            let mut body_lines: Vec<&str> = Vec::new();
            let mut in_body = false;
            while let Some(&peek) = lines.peek() {
                if peek.starts_with("## ") || peek.starts_with("### ") {
                    break;
                }
                let consumed = lines.next().unwrap_or("");
                let trimmed = consumed.trim_end();
                if !in_body {
                    if trimmed.is_empty() {
                        // Blank line separates header from body.
                        in_body = true;
                        continue;
                    }
                    if let Some((k, v)) = parse_kv(trimmed) {
                        fields.push((k.to_string(), v.to_string()));
                        continue;
                    }
                    // Non-kv before blank line → switch to body mode early.
                    in_body = true;
                    body_lines.push(consumed);
                } else {
                    body_lines.push(consumed);
                }
            }
            // Trim leading/trailing blank lines from body.
            while body_lines.first().map_or(false, |l| l.trim().is_empty()) {
                body_lines.remove(0);
            }
            while body_lines.last().map_or(false, |l| l.trim().is_empty()) {
                body_lines.pop();
            }
            let body = body_lines.join("\n");

            match current_section {
                Section::Global => {
                    if let Some(item) = build_global(id, title, &fields, &body) {
                        todo.globals.push(item);
                    }
                }
                Section::Worker { .. } => {
                    if let Some(item) = build_subtask(id, title, &fields, &body) {
                        if let Some(w) = current_worker.as_mut() {
                            w.subtasks.push(item);
                        }
                    }
                }
                Section::Outside => {
                    // Item before any section header — ignore silently.
                }
            }
            continue;
        }
        // Any other line (including blank lines, free text in a section) is
        // ignored at the doc level. Bodies are owned by their `###` block.
    }
    if let Some(w) = current_worker.take() {
        todo.workers.push(w);
    }
    Ok(todo)
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    let k = k.trim();
    if k.is_empty() || k.contains(char::is_whitespace) {
        return None;
    }
    Some((k, v.trim()))
}

fn parse_item_heading(s: &str) -> Option<(String, String)> {
    // Expected: `[ID] Title` — `[` at start, `]` somewhere later.
    let s = s.trim();
    let rest = s.strip_prefix('[')?;
    let end = rest.find(']')?;
    let id = rest[..end].trim().to_string();
    let title = rest[end + 1..].trim().to_string();
    if id.is_empty() {
        return None;
    }
    Some((id, title))
}

fn parse_worker_heading(s: &str) -> Option<(u32, Option<String>)> {
    // Canonical form is `pane:NNN — role-hint` (em dash), but tolerate
    // anything the Tech Lead might write — ASCII dash, parens, colon,
    // bare number. Read the leading digits, then strip common
    // separators around whatever role-hint follows.
    let s = s.trim();
    let after = s.strip_prefix("pane:")?.trim_start();
    let digit_end = after
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    if digit_end == 0 {
        return None;
    }
    let pane_id = after[..digit_end].parse::<u32>().ok()?;
    let role_hint = after[digit_end..]
        .trim()
        .trim_start_matches(|c: char| {
            c == '\u{2014}' || c == '-' || c == ':' || c == '(' || c.is_whitespace()
        })
        .trim_end_matches(|c: char| c == ')' || c.is_whitespace())
        .trim()
        .to_string();
    let role_hint = if role_hint.is_empty() {
        None
    } else {
        Some(role_hint)
    };
    Some((pane_id, role_hint))
}

fn split_pr_url_annotation(raw: &str) -> (String, Option<String>) {
    let raw = raw.trim();
    let Some((url, rest)) = raw.split_once(char::is_whitespace) else {
        return (raw.to_string(), None);
    };
    let annotation = rest.trim();
    if annotation.is_empty() {
        (url.to_string(), None)
    } else {
        (url.to_string(), Some(annotation.to_string()))
    }
}

fn build_global(
    id: String,
    title: String,
    fields: &[(String, String)],
    body: &str,
) -> Option<GlobalTodo> {
    let get = |k: &str| {
        fields
            .iter()
            .find(|(fk, _)| fk == k)
            .map(|(_, v)| v.clone())
    };
    let status = GlobalStatus::from_str(get("status").as_deref().unwrap_or("proposed"))?;
    let origin = Origin::from_str(get("origin").as_deref().unwrap_or("tech-lead"))?;
    let notes: Vec<String> = fields
        .iter()
        .filter(|(k, _)| k == "note")
        .map(|(_, v)| v.clone())
        .collect();

    // Each `pr:` line is `<pane_id> <url>` (per-worker PR). For backward
    // compat with the older single-PR schema, lines whose value is
    // `(not yet)` / empty are skipped. A single `pr: <url>` with no
    // leading pane_id falls through to pane_id=0 (sentinel).
    let mut prs: Vec<PaneTodoPr> = Vec::new();
    for (k, v) in fields {
        if k != "pr" {
            continue;
        }
        let v = v.trim();
        if v.is_empty() || v == "(not yet)" {
            continue;
        }
        if let Some((id_part, url_part)) = v.split_once(char::is_whitespace) {
            if let Ok(pane_id) = id_part.trim().parse::<u32>() {
                let (url, annotation) = split_pr_url_annotation(url_part);
                prs.push(PaneTodoPr {
                    pane_id,
                    url,
                    annotation,
                });
                continue;
            }
        }
        // Legacy: single URL with no pane_id. Keep it so we don't lose
        // links from pre-per-worker docs.
        let (url, annotation) = split_pr_url_annotation(v);
        prs.push(PaneTodoPr {
            pane_id: 0,
            url,
            annotation,
        });
    }

    Some(GlobalTodo {
        id,
        title,
        status,
        origin,
        notes,
        prs,
        body: body.to_string(),
    })
}

fn build_subtask(
    id: String,
    title: String,
    fields: &[(String, String)],
    body: &str,
) -> Option<WorkerSubtask> {
    let get = |k: &str| {
        fields
            .iter()
            .find(|(fk, _)| fk == k)
            .map(|(_, v)| v.clone())
    };
    let status = SubStatus::from_str(get("status").as_deref().unwrap_or("pending"))?;
    let parent = get("parent").unwrap_or_default();
    if parent.is_empty() {
        return None;
    }
    Some(WorkerSubtask {
        id,
        title,
        status,
        parent,
        body: body.to_string(),
    })
}

// ---------- serialize ----------

/// Render a `TeamTodo` back to canonical Markdown. The output is
/// stable: parse → serialize → parse → serialize is a fixed point.
pub fn serialize(todo: &TeamTodo) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Team TODO");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Global TODOs");
    if todo.globals.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "(no items yet)");
    }
    for g in &todo.globals {
        let _ = writeln!(out);
        let _ = writeln!(out, "### [{}] {}", g.id, g.title);
        let _ = writeln!(out, "status: {}", g.status.as_str());
        let _ = writeln!(out, "origin: {}", g.origin.as_str());
        if g.prs.is_empty() {
            let _ = writeln!(out, "pr: (not yet)");
        } else {
            for pr in &g.prs {
                let annotation = pr
                    .annotation
                    .as_deref()
                    .map(str::trim)
                    .filter(|a| !a.is_empty());
                if pr.pane_id == 0 {
                    // Legacy entry parsed from a pre-per-worker doc.
                    match annotation {
                        Some(annotation) => {
                            let _ = writeln!(out, "pr: {} {}", pr.url, annotation);
                        }
                        None => {
                            let _ = writeln!(out, "pr: {}", pr.url);
                        }
                    }
                } else {
                    match annotation {
                        Some(annotation) => {
                            let _ = writeln!(
                                out,
                                "pr: {} {} {}",
                                pr.pane_id,
                                pr.url,
                                annotation
                            );
                        }
                        None => {
                            let _ = writeln!(out, "pr: {} {}", pr.pane_id, pr.url);
                        }
                    }
                }
            }
        }
        for note in &g.notes {
            let _ = writeln!(out, "note: {}", note);
        }
        if !g.body.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", g.body);
        }
    }
    for w in &todo.workers {
        let _ = writeln!(out);
        match &w.role_hint {
            Some(hint) => {
                let _ = writeln!(out, "## pane:{} \u{2014} {}", w.pane_id, hint);
            }
            None => {
                let _ = writeln!(out, "## pane:{}", w.pane_id);
            }
        }
        if w.subtasks.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "(no items yet)");
        }
        for s in &w.subtasks {
            let _ = writeln!(out);
            let _ = writeln!(out, "### [{}] {}", s.id, s.title);
            let _ = writeln!(out, "status: {}", s.status.as_str());
            let _ = writeln!(out, "parent: {}", s.parent);
            if !s.body.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "{}", s.body);
            }
        }
    }
    out
}

// ---------- operations ----------

impl TeamTodo {
    pub fn find_global(&self, id: &str) -> Option<&GlobalTodo> {
        self.globals.iter().find(|g| g.id == id)
    }
    pub fn find_global_mut(&mut self, id: &str) -> Option<&mut GlobalTodo> {
        self.globals.iter_mut().find(|g| g.id == id)
    }

    /// Set the status of a global TODO. Returns the previous status, or
    /// None if no item with that ID exists.
    pub fn set_global_status(&mut self, id: &str, status: GlobalStatus) -> Option<GlobalStatus> {
        let g = self.find_global_mut(id)?;
        let prev = g.status;
        g.status = status;
        Some(prev)
    }

    /// Add (or replace) a global TODO. Insertion order matches user
    /// expectation: appended at the end.
    pub fn push_global(&mut self, item: GlobalTodo) {
        if let Some(existing) = self.find_global_mut(&item.id) {
            *existing = item;
        } else {
            self.globals.push(item);
        }
    }

    pub fn worker_section(&self, pane_id: u32) -> Option<&WorkerSection> {
        self.workers.iter().find(|w| w.pane_id == pane_id)
    }

    pub fn worker_section_mut(&mut self, pane_id: u32) -> Option<&mut WorkerSection> {
        self.workers.iter_mut().find(|w| w.pane_id == pane_id)
    }

    /// Ensure a worker section exists with the given (optional) role
    /// hint. Returns a mutable reference.
    pub fn upsert_worker_section(
        &mut self,
        pane_id: u32,
        role_hint: Option<String>,
    ) -> &mut WorkerSection {
        if let Some(i) = self.workers.iter().position(|w| w.pane_id == pane_id) {
            // Update role_hint if the caller knows a newer one.
            if let Some(hint) = role_hint {
                self.workers[i].role_hint = Some(hint);
            }
            return &mut self.workers[i];
        }
        self.workers.push(WorkerSection {
            pane_id,
            role_hint,
            subtasks: Vec::new(),
        });
        self.workers.last_mut().unwrap()
    }

    /// Drop the worker section for `pane_id` and return the Global TODO
    /// ids that have NO remaining subtasks across any other worker after
    /// the removal — those globals should be reset to `approved` so the
    /// Tech Lead re-expands and reassigns them to a different pane.
    ///
    /// Globals whose other workers still have subtasks are NOT returned
    /// — the multi-worker workflow continues with the remaining panes.
    /// Globals whose only contribution from this pane was already
    /// `done` / `approved` (the work is committed and likely in a PR)
    /// are also NOT returned — no reassignment needed.
    pub fn remove_pane_subtasks(&mut self, pane_id: u32) -> Vec<String> {
        let unfinished_parents: Vec<String> = self
            .worker_section(pane_id)
            .map(|w| {
                w.subtasks
                    .iter()
                    .filter(|s| {
                        !matches!(s.status, SubStatus::Done | SubStatus::Approved)
                    })
                    .map(|s| s.parent.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.workers.retain(|w| w.pane_id != pane_id);
        let mut seen = std::collections::HashSet::new();
        let mut orphaned: Vec<String> = Vec::new();
        for parent in unfinished_parents {
            if !seen.insert(parent.clone()) {
                continue;
            }
            let still_has = self
                .workers
                .iter()
                .any(|w| w.subtasks.iter().any(|s| s.parent == parent));
            if !still_has {
                orphaned.push(parent);
            }
        }
        orphaned
    }

    pub fn push_subtask(&mut self, pane_id: u32, item: WorkerSubtask) -> Result<()> {
        let w = self
            .worker_section_mut(pane_id)
            .ok_or_else(|| anyhow!("worker section for pane:{pane_id} not found"))?;
        if let Some(existing) = w.subtasks.iter_mut().find(|s| s.id == item.id) {
            *existing = item;
        } else {
            w.subtasks.push(item);
        }
        Ok(())
    }

    pub fn set_subtask_status(
        &mut self,
        subtask_id: &str,
        status: SubStatus,
    ) -> Option<SubStatus> {
        for w in self.workers.iter_mut() {
            if let Some(s) = w.subtasks.iter_mut().find(|s| s.id == subtask_id) {
                let prev = s.status;
                s.status = status;
                return Some(prev);
            }
        }
        None
    }

    /// All subtasks (across all workers) whose `parent` matches the
    /// given global TODO id.
    pub fn subtasks_for(&self, global_id: &str) -> Vec<&WorkerSubtask> {
        self.workers
            .iter()
            .flat_map(|w| w.subtasks.iter())
            .filter(|s| s.parent == global_id)
            .collect()
    }

    /// What the Tech Lead should do this iteration. Encodes the
    /// expand → dispatch → review hand-off so the agent doesn't have
    /// to re-derive it from the doc each tick.
    pub fn next_actions(&self) -> NextActions {
        let expand_next = self
            .globals
            .iter()
            .find(|g| {
                g.status == GlobalStatus::Approved && self.subtasks_for(&g.id).is_empty()
            })
            .map(|g| g.id.clone());

        let dispatch: Vec<DispatchHint> = self
            .workers
            .iter()
            .filter_map(|w| {
                let has_active = w
                    .subtasks
                    .iter()
                    .any(|s| matches!(s.status, SubStatus::InProgress | SubStatus::Revising));
                if has_active {
                    return None;
                }
                w.subtasks
                    .iter()
                    .find(|s| s.status == SubStatus::Pending)
                    .map(|s| DispatchHint {
                        pane_id: w.pane_id,
                        role_hint: w.role_hint.clone(),
                        subtask_id: s.id.clone(),
                        subtask_title: s.title.clone(),
                        subtask_body: s.body.clone(),
                        parent_global: s.parent.clone(),
                    })
            })
            .collect();

        let ready_for_review: Vec<String> = self
            .globals
            .iter()
            .filter(|g| g.status == GlobalStatus::InProgress)
            .filter(|g| {
                let subs = self.subtasks_for(&g.id);
                !subs.is_empty()
                    && subs.iter().all(|s| {
                        matches!(s.status, SubStatus::Done | SubStatus::Approved)
                    })
            })
            .map(|g| g.id.clone())
            .collect();

        let pending_proposals: Vec<String> = self
            .globals
            .iter()
            .filter(|g| g.status == GlobalStatus::Proposed)
            .map(|g| g.id.clone())
            .collect();

        NextActions {
            expand_next,
            dispatch,
            ready_for_review,
            pending_proposals,
        }
    }

    /// Next monotonically-increasing global TODO ID, e.g. "TODO-003"
    /// when the doc already has TODO-001 and TODO-002.
    pub fn next_global_id(&self) -> String {
        let max = self
            .globals
            .iter()
            .filter_map(|g| g.id.strip_prefix("TODO-").and_then(|n| n.parse::<u32>().ok()))
            .max()
            .unwrap_or(0);
        format!("TODO-{:03}", max + 1)
    }
}

pub fn apply_todo_approval(
    todo: &mut TeamTodo,
    todo_id: &str,
    action: &str,
) -> Result<Option<GlobalStatus>> {
    let new_status = match action {
        "approve" => GlobalStatus::Approved,
        "reject" => GlobalStatus::Rejected,
        _ => return Err(anyhow!("unknown todo approval action: {action}")),
    };
    Ok(todo.set_global_status(todo_id, new_status))
}

pub fn add_user_todo(todo: &mut TeamTodo, title: &str, body: String) -> Option<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    let id = todo.next_global_id();
    todo.push_global(GlobalTodo {
        id: id.clone(),
        title: trimmed.to_string(),
        status: GlobalStatus::Approved,
        origin: Origin::User,
        notes: Vec::new(),
        prs: Vec::new(),
        body,
    });
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn team_todo_tmp_entries(dir: &Path) -> Vec<String> {
        let mut entries: Vec<String> = std::fs::read_dir(dir)
            .expect("read temp project dir")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                (name.starts_with("team-todo.md.") && name.ends_with(".tmp"))
                    .then_some(name)
            })
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn team_todo_tmp_path_is_unique_and_not_legacy_shared_name() {
        let tmp = TempDir::new().expect("temp project dir");
        let path = tmp.path().join(TEAM_TODO_FILENAME);
        let first = team_todo_tmp_path(&path);
        let second = team_todo_tmp_path(&path);

        assert_ne!(first, second);
        assert_ne!(first, path.with_extension("md.tmp"));
        assert_eq!(first.parent(), Some(tmp.path()));

        let tmp_name = first
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 temp filename");
        assert!(tmp_name.starts_with("team-todo.md."));
        assert!(tmp_name.contains(&format!(".{}.", std::process::id())));
        assert!(tmp_name.ends_with(".tmp"));
    }

    fn sample_doc() -> &'static str {
        // Includes both kinds of section, the optional role hint syntax,
        // multi-line bodies, and the "(not yet)" PR sentinel.
        "# Team TODO\n\
\n\
## Global TODOs\n\
\n\
### [TODO-001] Switch the auth middleware to JWT\n\
status: approved\n\
origin: user\n\
pr: (not yet)\n\
\n\
The current session-cookie middleware is being deprecated. Replace\n\
with the JWT validator from the shared auth crate.\n\
\n\
### [TODO-002] Add streaming-response support to /v1/chat\n\
status: proposed\n\
origin: tech-lead\n\
pr: (not yet)\n\
\n\
While auditing the API I noticed callers ask for streaming via the\n\
`stream=true` query param but we always send the full response in one\n\
shot.\n\
\n\
## pane:578 \u{2014} backend-engineer\n\
\n\
### [TODO-001 \u{00b7} backend-1] Replace middleware in src/auth/middleware.rs\n\
status: in_progress\n\
parent: TODO-001\n\
\n\
- Remove `SessionCookieMiddleware`\n\
- Wire `JwtValidator::from_env()` at the same place\n\
\n\
### [TODO-001 \u{00b7} backend-2] Update the integration test harness\n\
status: pending\n\
parent: TODO-001\n\
\n\
The test fixtures bake session cookies in; replace with a JWT minter.\n\
\n\
## pane:612 \u{2014} frontend-engineer\n\
\n\
(no items yet)\n"
    }

    #[test]
    fn parses_a_realistic_doc() {
        let t = parse(sample_doc()).unwrap();
        assert_eq!(t.globals.len(), 2);
        assert_eq!(t.workers.len(), 2);

        let g1 = &t.globals[0];
        assert_eq!(g1.id, "TODO-001");
        assert_eq!(g1.title, "Switch the auth middleware to JWT");
        assert_eq!(g1.status, GlobalStatus::Approved);
        assert_eq!(g1.origin, Origin::User);
        assert!(g1.prs.is_empty());
        assert!(g1.body.contains("session-cookie"));

        let g2 = &t.globals[1];
        assert_eq!(g2.status, GlobalStatus::Proposed);
        assert_eq!(g2.origin, Origin::TechLead);

        let w0 = &t.workers[0];
        assert_eq!(w0.pane_id, 578);
        assert_eq!(w0.role_hint.as_deref(), Some("backend-engineer"));
        assert_eq!(w0.subtasks.len(), 2);
        assert_eq!(w0.subtasks[0].id, "TODO-001 \u{00b7} backend-1");
        assert_eq!(w0.subtasks[0].status, SubStatus::InProgress);
        assert_eq!(w0.subtasks[0].parent, "TODO-001");

        let w1 = &t.workers[1];
        assert_eq!(w1.pane_id, 612);
        assert!(w1.subtasks.is_empty());
    }

    #[test]
    fn round_trip_is_a_fixed_point() {
        let original = parse(sample_doc()).unwrap();
        let rendered = serialize(&original);
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed, original, "parse(serialize(x)) must equal x");
        let rendered_again = serialize(&reparsed);
        assert_eq!(
            rendered, rendered_again,
            "serialize must be idempotent on a parsed-then-serialized doc"
        );
    }

    #[test]
    fn to_wire_with_cursors_trims_agent_cursor_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(".apas-tech-lead-cursor"),
            "  2026-06-17T10:00:00-04:00\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".apas-reviewer-cursor"),
            "\n2026-06-17T10:05:00-04:00  \n",
        )
        .unwrap();

        let got = to_wire_with_cursors(&parse(sample_doc()).unwrap(), tmp.path());

        assert_eq!(
            got.tech_lead_cursor.as_deref(),
            Some("2026-06-17T10:00:00-04:00")
        );
        assert_eq!(
            got.reviewer_cursor.as_deref(),
            Some("2026-06-17T10:05:00-04:00")
        );
    }

    #[test]
    fn to_wire_with_cursors_returns_none_for_missing_or_blank_cursor_files() {
        let tmp = TempDir::new().unwrap();
        let todo = parse(sample_doc()).unwrap();

        let missing = to_wire_with_cursors(&todo, tmp.path());
        assert_eq!(missing.tech_lead_cursor, None);
        assert_eq!(missing.reviewer_cursor, None);

        std::fs::write(tmp.path().join(".apas-tech-lead-cursor"), "  \n\t").unwrap();
        std::fs::write(tmp.path().join(".apas-reviewer-cursor"), "\n\n").unwrap();
        let blank = to_wire_with_cursors(&todo, tmp.path());
        assert_eq!(blank.tech_lead_cursor, None);
        assert_eq!(blank.reviewer_cursor, None);
    }

    #[test]
    fn to_wire_keeps_cursor_metadata_empty() {
        let got = to_wire(&parse(sample_doc()).unwrap());

        assert_eq!(got.tech_lead_cursor, None);
        assert_eq!(got.reviewer_cursor, None);
    }

    #[test]
    fn ignores_unknown_top_level_sections() {
        let s = "# Team TODO\n\n## Notes\n\nfree text here\n\n## Global TODOs\n\n### [TODO-001] Hi\nstatus: approved\norigin: user\n\nbody\n";
        let t = parse(s).unwrap();
        assert_eq!(t.globals.len(), 1);
    }

    #[test]
    fn skips_subtask_missing_required_parent_field() {
        let s = "# Team TODO\n\n## pane:5 \u{2014} role\n\n### [foo] Title\nstatus: pending\n\nbody\n";
        let t = parse(s).unwrap();
        assert_eq!(t.workers.len(), 1);
        assert_eq!(t.workers[0].subtasks.len(), 0, "must skip subtask with no parent");
    }

    #[test]
    fn empty_doc_parses_to_empty_team_todo() {
        let t = parse("").unwrap();
        assert!(t.globals.is_empty());
        assert!(t.workers.is_empty());
        // Serializing an empty doc still emits the header + an empty
        // global section so the file is recognizable.
        let s = serialize(&t);
        assert!(s.contains("# Team TODO"));
        assert!(s.contains("## Global TODOs"));
    }

    #[test]
    fn parse_worker_heading_handles_separators() {
        assert_eq!(
            parse_worker_heading("pane:218"),
            Some((218, None))
        );
        assert_eq!(
            parse_worker_heading("pane:218 \u{2014} Frontend developer"),
            Some((218, Some("Frontend developer".to_string())))
        );
        assert_eq!(
            parse_worker_heading("pane:218 - Frontend developer"),
            Some((218, Some("Frontend developer".to_string())))
        );
        assert_eq!(
            parse_worker_heading("pane:218 (Frontend developer)"),
            Some((218, Some("Frontend developer".to_string())))
        );
        assert_eq!(
            parse_worker_heading("pane:218: Frontend developer"),
            Some((218, Some("Frontend developer".to_string())))
        );
        assert_eq!(parse_worker_heading("pane:nope"), None);
        assert_eq!(parse_worker_heading("manager"), None);
    }

    #[test]
    fn remove_pane_subtasks_returns_orphaned_globals() {
        let mut t = TeamTodo::default();
        t.push_global(GlobalTodo {
            id: "TODO-001".into(),
            title: "Only one worker".into(),
            status: GlobalStatus::InProgress,
            origin: Origin::User,
            notes: Vec::new(),
            prs: vec![],
            body: String::new(),
        });
        t.push_global(GlobalTodo {
            id: "TODO-002".into(),
            title: "Two workers".into(),
            status: GlobalStatus::InProgress,
            origin: Origin::User,
            notes: Vec::new(),
            prs: vec![],
            body: String::new(),
        });
        // Pane 5: TODO-001 (in_progress) + TODO-002 (in_progress)
        t.upsert_worker_section(5, None);
        t.push_subtask(5, WorkerSubtask {
            id: "TODO-001 · a".into(),
            title: "x".into(),
            status: SubStatus::InProgress,
            parent: "TODO-001".into(),
            body: String::new(),
        }).unwrap();
        t.push_subtask(5, WorkerSubtask {
            id: "TODO-002 · a".into(),
            title: "y".into(),
            status: SubStatus::InProgress,
            parent: "TODO-002".into(),
            body: String::new(),
        }).unwrap();
        // Pane 7: TODO-002 (pending) — still has a contributor after pane 5 leaves
        t.upsert_worker_section(7, None);
        t.push_subtask(7, WorkerSubtask {
            id: "TODO-002 · b".into(),
            title: "z".into(),
            status: SubStatus::Pending,
            parent: "TODO-002".into(),
            body: String::new(),
        }).unwrap();

        let orphaned = t.remove_pane_subtasks(5);
        assert_eq!(orphaned, vec!["TODO-001".to_string()]);
        assert!(t.worker_section(5).is_none(), "pane 5 section should be gone");
        assert!(t.worker_section(7).is_some(), "pane 7 section should remain");
    }

    #[test]
    fn remove_pane_subtasks_skips_already_done() {
        let mut t = TeamTodo::default();
        t.push_global(GlobalTodo {
            id: "TODO-001".into(),
            title: "done before removal".into(),
            status: GlobalStatus::PrOpen,
            origin: Origin::User,
            notes: Vec::new(),
            prs: vec![],
            body: String::new(),
        });
        t.upsert_worker_section(5, None);
        t.push_subtask(5, WorkerSubtask {
            id: "TODO-001 · a".into(),
            title: "x".into(),
            status: SubStatus::Done,
            parent: "TODO-001".into(),
            body: String::new(),
        }).unwrap();
        let orphaned = t.remove_pane_subtasks(5);
        assert!(orphaned.is_empty(), "done subtasks don't trigger reset");
    }

    #[test]
    fn set_global_status_returns_previous_value() {
        let mut t = parse(sample_doc()).unwrap();
        let prev = t.set_global_status("TODO-002", GlobalStatus::Approved).unwrap();
        assert_eq!(prev, GlobalStatus::Proposed);
        assert_eq!(t.find_global("TODO-002").unwrap().status, GlobalStatus::Approved);
        assert!(t.set_global_status("nonexistent", GlobalStatus::Done).is_none());
    }

    #[test]
    fn apply_todo_approval_flips_only_requested_global() {
        let mut t = parse(sample_doc()).unwrap();

        let prev = apply_todo_approval(&mut t, "TODO-002", "approve")
            .unwrap()
            .unwrap();

        assert_eq!(prev, GlobalStatus::Proposed);
        assert_eq!(t.find_global("TODO-002").unwrap().status, GlobalStatus::Approved);
        assert_eq!(t.find_global("TODO-001").unwrap().status, GlobalStatus::Approved);

        let prev = apply_todo_approval(&mut t, "TODO-001", "reject")
            .unwrap()
            .unwrap();

        assert_eq!(prev, GlobalStatus::Approved);
        assert_eq!(t.find_global("TODO-001").unwrap().status, GlobalStatus::Rejected);
        assert_eq!(t.find_global("TODO-002").unwrap().status, GlobalStatus::Approved);
        assert!(apply_todo_approval(&mut t, "TODO-999", "approve").unwrap().is_none());
        assert!(apply_todo_approval(&mut t, "TODO-001", "hold").is_err());
    }

    #[test]
    fn add_user_todo_trims_title_and_creates_approved_user_global() {
        let mut t = parse(sample_doc()).unwrap();

        let id = add_user_todo(&mut t, "  Add keyboard flow  ", "body text".into()).unwrap();

        assert_eq!(id, "TODO-003");
        let added = t.find_global("TODO-003").unwrap();
        assert_eq!(added.title, "Add keyboard flow");
        assert_eq!(added.status, GlobalStatus::Approved);
        assert_eq!(added.origin, Origin::User);
        assert!(added.prs.is_empty());
        assert_eq!(added.body, "body text");
    }

    #[test]
    fn add_user_todo_rejects_empty_titles_without_mutating() {
        let mut t = parse(sample_doc()).unwrap();
        let before = t.clone();

        assert_eq!(add_user_todo(&mut t, "   ", "ignored".into()), None);

        assert_eq!(t, before);
    }

    #[test]
    fn set_subtask_status_finds_across_workers() {
        let mut t = parse(sample_doc()).unwrap();
        let prev = t
            .set_subtask_status("TODO-001 \u{00b7} backend-2", SubStatus::Done)
            .unwrap();
        assert_eq!(prev, SubStatus::Pending);
        assert_eq!(
            t.workers[0].subtasks[1].status,
            SubStatus::Done
        );
    }

    #[test]
    fn upsert_worker_section_adds_or_updates_role_hint() {
        let mut t = TeamTodo::default();
        t.upsert_worker_section(7, None);
        assert_eq!(t.workers.len(), 1);
        assert!(t.workers[0].role_hint.is_none());
        t.upsert_worker_section(7, Some("foo".into()));
        assert_eq!(t.workers.len(), 1);
        assert_eq!(t.workers[0].role_hint.as_deref(), Some("foo"));
    }

    #[test]
    fn push_subtask_replaces_existing_with_same_id() {
        let mut t = TeamTodo::default();
        t.upsert_worker_section(5, Some("backend".into()));
        let s1 = WorkerSubtask {
            id: "x".into(),
            title: "first".into(),
            status: SubStatus::Pending,
            parent: "TODO-001".into(),
            body: "old".into(),
        };
        t.push_subtask(5, s1).unwrap();
        let s2 = WorkerSubtask {
            id: "x".into(),
            title: "second".into(),
            status: SubStatus::InProgress,
            parent: "TODO-001".into(),
            body: "new".into(),
        };
        t.push_subtask(5, s2).unwrap();
        assert_eq!(t.workers[0].subtasks.len(), 1);
        assert_eq!(t.workers[0].subtasks[0].title, "second");
        assert_eq!(t.workers[0].subtasks[0].status, SubStatus::InProgress);
    }

    #[test]
    fn next_global_id_increments_past_existing_max() {
        let t = parse(sample_doc()).unwrap();
        assert_eq!(t.next_global_id(), "TODO-003");
        let empty = TeamTodo::default();
        assert_eq!(empty.next_global_id(), "TODO-001");
    }

    #[test]
    fn parses_multiple_pr_lines_per_global() {
        let s = "# Team TODO\n\n## Global TODOs\n\n\
### [TODO-001] Streaming chat\n\
status: pr_open\n\
origin: user\n\
pr: 578 https://github.com/foo/bar/pull/42\n\
pr: 612 https://github.com/foo/bar/pull/43\n\
\n\
backend + frontend split.\n";
        let t = parse(s).unwrap();
        let g = &t.globals[0];
        assert_eq!(g.prs.len(), 2);
        assert_eq!(g.prs[0].pane_id, 578);
        assert_eq!(g.prs[0].url, "https://github.com/foo/bar/pull/42");
        assert_eq!(g.prs[0].annotation, None);
        assert_eq!(g.prs[1].pane_id, 612);
        assert_eq!(g.prs[1].url, "https://github.com/foo/bar/pull/43");
        assert_eq!(g.prs[1].annotation, None);
        // Round-trip preserves order + format.
        let rendered = serialize(&t);
        assert!(rendered.contains("pr: 578 https://github.com/foo/bar/pull/42"));
        assert!(rendered.contains("pr: 612 https://github.com/foo/bar/pull/43"));
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed, t);
    }

    #[test]
    fn parses_annotated_pr_line_with_clean_url_and_round_trips_annotation() {
        let s = "# Team TODO\n\n## Global TODOs\n\n\
### [TODO-001] Annotated\n\
status: done\n\
origin: tech-lead\n\
pr: 568 https://github.com/shuaimu/apas/pull/12 (MERGED 2026-06-16T03:59:12Z 7d78b3e...)\n\
\n\
landed.\n";
        let t = parse(s).unwrap();
        let pr = &t.globals[0].prs[0];
        assert_eq!(pr.pane_id, 568);
        assert_eq!(pr.url, "https://github.com/shuaimu/apas/pull/12");
        assert_eq!(
            pr.annotation.as_deref(),
            Some("(MERGED 2026-06-16T03:59:12Z 7d78b3e...)")
        );

        let rendered = serialize(&t);
        assert!(rendered.contains(
            "pr: 568 https://github.com/shuaimu/apas/pull/12 \
(MERGED 2026-06-16T03:59:12Z 7d78b3e...)"
        ));
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed, t);
    }

    #[test]
    fn parses_and_round_trips_single_global_note() {
        let s = "# Team TODO\n\n## Global TODOs\n\n\
### [TODO-001] Audit note\n\
status: approved\n\
origin: tech-lead\n\
pr: (not yet)\n\
note: auto-approved by tech-lead at 2026-06-16T10:03:48-04:00\n\
\n\
audit body.\n";
        let t = parse(s).unwrap();
        let g = &t.globals[0];
        assert_eq!(
            g.notes,
            vec!["auto-approved by tech-lead at 2026-06-16T10:03:48-04:00".to_string()]
        );

        let rendered = serialize(&t);
        assert!(rendered.contains(
            "note: auto-approved by tech-lead at 2026-06-16T10:03:48-04:00"
        ));
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed, t);
    }

    #[test]
    fn parses_and_round_trips_multiple_global_notes() {
        let s = "# Team TODO\n\n## Global TODOs\n\n\
### [TODO-001] Audit notes\n\
status: proposed\n\
origin: tech-lead\n\
note: first survey note\n\
note: second survey note\n\
\n\
body.\n";
        let t = parse(s).unwrap();
        let g = &t.globals[0];
        assert_eq!(
            g.notes,
            vec![
                "first survey note".to_string(),
                "second survey note".to_string()
            ]
        );

        let rendered = serialize(&t);
        assert!(rendered.contains("note: first survey note\nnote: second survey note"));
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed, t);
    }

    #[test]
    fn preserves_notes_with_merged_annotated_pr_lines() {
        let s = "# Team TODO\n\n## Global TODOs\n\n\
### [TODO-001] Landed\n\
status: done\n\
origin: tech-lead\n\
pr: 568 https://github.com/shuaimu/apas/pull/12 (MERGED 2026-06-16T03:59:12Z 7d78b3e...)\n\
note: auto-approved by tech-lead at 2026-06-16T10:03:48-04:00\n\
\n\
landed body.\n";
        let t = parse(s).unwrap();
        let g = &t.globals[0];
        assert_eq!(
            g.notes,
            vec!["auto-approved by tech-lead at 2026-06-16T10:03:48-04:00".to_string()]
        );
        assert_eq!(g.prs[0].pane_id, 568);
        assert_eq!(g.prs[0].url, "https://github.com/shuaimu/apas/pull/12");
        assert_eq!(
            g.prs[0].annotation.as_deref(),
            Some("(MERGED 2026-06-16T03:59:12Z 7d78b3e...)")
        );

        let rendered = serialize(&t);
        assert!(rendered.contains(
            "pr: 568 https://github.com/shuaimu/apas/pull/12 \
(MERGED 2026-06-16T03:59:12Z 7d78b3e...)\n\
note: auto-approved by tech-lead at 2026-06-16T10:03:48-04:00"
        ));
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed, t);
    }

    #[test]
    fn parses_legacy_single_pr_line_with_pane_id_zero() {
        let s = "# Team TODO\n\n## Global TODOs\n\n\
### [TODO-001] Legacy one\n\
status: pr_open\n\
origin: user\n\
pr: https://github.com/foo/bar/pull/7\n\
\n\
old format.\n";
        let t = parse(s).unwrap();
        assert_eq!(t.globals[0].prs.len(), 1);
        // pane_id sentinel = 0 marks "we don't know which worker"
        assert_eq!(t.globals[0].prs[0].pane_id, 0);
        assert_eq!(t.globals[0].prs[0].url, "https://github.com/foo/bar/pull/7");
        assert_eq!(t.globals[0].prs[0].annotation, None);
    }

    #[test]
    fn empty_prs_serializes_as_not_yet() {
        let mut t = TeamTodo::default();
        t.push_global(GlobalTodo {
            id: "TODO-001".into(),
            title: "x".into(),
            status: GlobalStatus::Proposed,
            origin: Origin::User,
            notes: Vec::new(),
            prs: Vec::new(),
            body: String::new(),
        });
        let s = serialize(&t);
        assert!(s.contains("pr: (not yet)"));
    }

    #[test]
    fn next_actions_picks_expand_target_and_dispatch_per_worker() {
        // TODO-001 is approved + has subtasks → not an expand candidate.
        //   - pane 578 has one in_progress and one pending → no dispatch
        //     (it's busy).
        //   - pane 612 has no subtasks → no dispatch.
        // TODO-002 is proposed → goes into pending_proposals only.
        let mut t = parse(sample_doc()).unwrap();

        let n = t.next_actions();
        assert!(n.expand_next.is_none(), "approved global has subtasks already");
        assert_eq!(n.dispatch.len(), 0, "pane 578 is busy, pane 612 has no work");
        assert_eq!(n.pending_proposals, vec!["TODO-002".to_string()]);
        assert!(n.ready_for_review.is_empty());

        // Add an approved-but-unexpanded global → it shows up as expand_next.
        t.push_global(GlobalTodo {
            id: "TODO-003".into(),
            title: "fresh".into(),
            status: GlobalStatus::Approved,
            origin: Origin::User,
            notes: Vec::new(),
            prs: Vec::new(),
            body: String::new(),
        });
        let n = t.next_actions();
        assert_eq!(n.expand_next.as_deref(), Some("TODO-003"));

        // Mark pane 578's in_progress subtask as done → still no dispatch
        // there (no pending after the done), but the other pending stays.
        // Actually we set its status from in_progress→done; the SECOND
        // entry is the pending one. So pane 578's pending becomes
        // dispatchable.
        t.set_subtask_status("TODO-001 \u{00b7} backend-1", SubStatus::Done);
        let n = t.next_actions();
        assert_eq!(n.dispatch.len(), 1);
        assert_eq!(n.dispatch[0].pane_id, 578);
        assert_eq!(n.dispatch[0].subtask_id, "TODO-001 \u{00b7} backend-2");

        // Mark both subtasks done → TODO-001 (in_progress) goes to
        // ready_for_review.
        t.set_subtask_status("TODO-001 \u{00b7} backend-2", SubStatus::Done);
        t.set_global_status("TODO-001", GlobalStatus::InProgress);
        let n = t.next_actions();
        assert_eq!(n.ready_for_review, vec!["TODO-001".to_string()]);
        assert_eq!(n.dispatch.len(), 0, "no more pending work");
    }

    #[test]
    fn save_load_round_trip_through_filesystem() {
        let tmp = TempDir::new().expect("temp project dir");
        let mut t = TeamTodo::default();
        t.push_global(GlobalTodo {
            id: "TODO-001".into(),
            title: "first".into(),
            status: GlobalStatus::Proposed,
            origin: Origin::TechLead,
            notes: Vec::new(),
            prs: Vec::new(),
            body: "body".into(),
        });
        save(tmp.path(), &t).unwrap();
        let reloaded = load(tmp.path()).unwrap();
        assert_eq!(reloaded, t);
        assert!(!tmp.path().join("team-todo.md.tmp").exists());
    }

    #[test]
    fn repeated_saves_write_readable_markdown_without_stale_temp_files() {
        let tmp = TempDir::new().expect("temp project dir");
        let legacy_tmp = tmp.path().join("team-todo.md.tmp");
        let mut t = TeamTodo::default();
        t.push_global(GlobalTodo {
            id: "TODO-001".into(),
            title: "first".into(),
            status: GlobalStatus::Proposed,
            origin: Origin::TechLead,
            notes: Vec::new(),
            prs: Vec::new(),
            body: String::new(),
        });

        for idx in 0..3 {
            t.globals[0].body = format!("body {idx}");
            save(tmp.path(), &t).unwrap();

            let reloaded = load(tmp.path()).unwrap();
            assert_eq!(reloaded, t);
            assert!(
                !legacy_tmp.exists(),
                "shared team-todo.md.tmp should not remain"
            );
            let stale_tmp_entries = team_todo_tmp_entries(tmp.path());
            assert!(
                stale_tmp_entries.is_empty(),
                "stale team-todo temp files remained: {stale_tmp_entries:?}"
            );
        }
    }

    #[test]
    fn save_after_approval_and_add_todo_mutations_publishes_readable_markdown() {
        let tmp = TempDir::new().expect("temp project dir");
        let mut t = TeamTodo::default();
        t.push_global(GlobalTodo {
            id: "TODO-001".into(),
            title: "needs approval".into(),
            status: GlobalStatus::Proposed,
            origin: Origin::TechLead,
            notes: Vec::new(),
            prs: Vec::new(),
            body: "proposal body".into(),
        });

        let previous = apply_todo_approval(&mut t, "TODO-001", "approve").unwrap();
        assert_eq!(previous, Some(GlobalStatus::Proposed));
        let added_id =
            add_user_todo(&mut t, "user request", "user body".to_string()).unwrap();
        assert_eq!(added_id, "TODO-002");

        save(tmp.path(), &t).unwrap();
        let reloaded = load(tmp.path()).unwrap();
        assert_eq!(reloaded, t);
        assert_eq!(
            reloaded.find_global("TODO-001").map(|item| item.status),
            Some(GlobalStatus::Approved)
        );
        assert_eq!(
            reloaded.find_global("TODO-002").map(|item| item.origin),
            Some(Origin::User)
        );
        assert!(!tmp.path().join("team-todo.md.tmp").exists());
    }

    #[test]
    fn load_returns_empty_when_file_missing() {
        let tmp = TempDir::new().expect("temp project dir");
        let t = load(tmp.path()).unwrap();
        assert_eq!(t, TeamTodo::default());
    }

    #[test]
    fn subtasks_for_filters_by_parent() {
        let t = parse(sample_doc()).unwrap();
        let subs = t.subtasks_for("TODO-001");
        assert_eq!(subs.len(), 2);
        assert!(subs.iter().all(|s| s.parent == "TODO-001"));
        let none = t.subtasks_for("TODO-002");
        assert!(none.is_empty());
    }
}
