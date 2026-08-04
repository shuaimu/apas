//! `apas mcp-server` — the typed team-mode tool surface (Phase 3.1 follow-up).
//!
//! Historically every managed pane drove team mode by hand-assembling
//! `.apas-team.jsonl` records in bash and tracking its own cursor file:
//!
//! ```text
//! LAST=$(cat .apas-tech-lead-cursor 2>/dev/null || echo "")
//! if [ -z "$LAST" ]; then tail -n 50 .apas-team.jsonl
//! else jq -c "select(.ts > \"$LAST\")" .apas-team.jsonl; fi
//! ```
//!
//! That works, but it puts the protocol in prose: tag spelling, timestamp
//! format, and cursor bookkeeping are all the agent's problem, and a typo
//! fails silently. This module exposes the same protocol as MCP tools with
//! schemas derived from Rust types, so a malformed call is rejected at the
//! tool boundary instead of landing as an unroutable record.
//!
//! **The scratchpad file remains the source of truth.** Every write here goes
//! through [`crate::scratchpad`] / [`crate::team_todo`] / [`crate::manager`]
//! and lands on disk in exactly the shape it always had. That keeps three
//! properties the JSONL-first design bought us:
//!
//!  * the CLI's scratchpad watcher still *observes* writes rather than
//!    trusting agents to report them,
//!  * every delegation stays visible in the web Team modal for free, and
//!  * team state survives a machine loss — after the 2026-08-02 NFS crash
//!    the scratchpad was the only durable artifact of it, because the server
//!    persists none of this.
//!
//! One process is spawned per pane, so `pane_id` is supplied by the CLI at
//! spawn time and stamped server-side. An agent cannot publish as another
//! pane, and cannot forget to identify itself.

use anyhow::{Context, Result};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::scratchpad::{self, TeamRecord};
use crate::team_todo::{self, GlobalStatus};

/// Default number of scratchpad records returned when a pane has no cursor
/// yet. Mirrors the `tail -n 50` the prompts used to run on first read.
const DEFAULT_COLD_READ: usize = 50;

/// Hard cap on records returned in one call, so a pane that passes a stale
/// cursor into a busy project can't blow its own context window.
const MAX_RECORDS_PER_READ: usize = 200;

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

/// Lock file guarding read-modify-write of the project's team files.
const LOCK_FILENAME: &str = ".apas-team.lock";

/// Advisory whole-project lock held across a load → mutate → save cycle.
///
/// This is **cross-process** by necessity: every pane runs its own
/// `apas mcp-server`, and rmcp additionally serves tool calls concurrently
/// within one server. Without it, two panes that touch `team-todo.md` in the
/// same instant both read the old file and the second save silently discards
/// the first change — a lost update with no error on either side. That was
/// reproducible the moment two mutating tool calls were issued together.
///
/// `flock` is released automatically when the fd closes, including on panic
/// or a killed pane, so a crashed agent cannot wedge the project.
struct ProjectLock {
    #[cfg(unix)]
    file: std::fs::File,
}

impl ProjectLock {
    fn acquire(project_dir: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(project_dir.join(LOCK_FILENAME))?;
            // Blocking exclusive lock: contention here is between a handful of
            // panes doing sub-millisecond file edits, so queueing is fine and
            // far better than failing the tool call.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self { file })
        }
        #[cfg(not(unix))]
        {
            let _ = project_dir;
            Ok(Self {})
        }
    }
}

#[cfg(unix)]
impl Drop for ProjectLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn internal(msg: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(msg.into(), None)
}

fn json_result<T: Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::json(value)?]))
}

// ---------------------------------------------------------------------------
// Tool argument types. Each derives JsonSchema, so the wire schema advertised
// in `tools/list` is generated from these definitions and cannot drift.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PublishRecordArgs {
    /// Record category: "diff", "review", "decision", "status", "escalation".
    /// Convention only — not validated, matching the file format.
    pub kind: String,
    /// Free-form body. Long is fine.
    pub body: String,
    /// Extra tags, e.g. ["task:TODO-011"]. `pane_id` is stamped for you and
    /// must not be encoded here.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DelegateArgs {
    /// Pane that should pick up this work. Use `list_panes` to find targets.
    pub target_pane_id: u32,
    /// Global TODO id this delegation belongs to, e.g. "TODO-011".
    pub task_id: String,
    /// The instruction routed into the target pane's input queue.
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadRecordsArgs {
    /// Cursor from a previous `read_records` call. Omit on the first read to
    /// get the most recent records.
    #[serde(default)]
    pub since: Option<String>,
    /// Only return records carrying all of these tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Max records to return (default 50, capped at 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ReadRecordsResult {
    records: Vec<TeamRecord>,
    /// Pass back as `since` on the next call. Advances only when records were
    /// returned, so an empty poll is safe to repeat.
    next_cursor: Option<String>,
    /// True when `limit` truncated the result; call again with `next_cursor`.
    has_more: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeTodoArgs {
    /// One-line title, e.g. "Stabilize DeepSeek provider integration".
    pub title: String,
    /// Body: scope, acceptance criteria, files in play.
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTodoStatusArgs {
    /// Global TODO id, e.g. "TODO-011".
    pub todo_id: String,
    /// One of: proposed, approved, rejected, in_progress, under_review,
    /// pr_open, done.
    pub status: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteProjectGoalArgs {
    /// Full replacement contents for `project_goal.md`.
    pub content: String,
}

#[derive(Debug, Serialize)]
struct PaneSummary {
    pane_id: u32,
    label: Option<String>,
    role: Option<String>,
    provider: String,
    managed: bool,
    /// Terminal panes host a real TUI and publish no stream events; they are
    /// never valid delegation targets.
    kind: String,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ApasMcpServer {
    project_dir: PathBuf,
    /// Supplied by the CLI at spawn time; stamped onto every published record
    /// so an agent can neither spoof another pane nor omit its own id.
    pane_id: u32,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ApasMcpServer {
    pub fn new(project_dir: PathBuf, pane_id: u32) -> Self {
        Self {
            project_dir,
            pane_id,
            tool_router: Self::tool_router(),
        }
    }

    fn dir(&self) -> &Path {
        &self.project_dir
    }

    #[tool(
        description = "Publish a record to the team scratchpad (.apas-team.jsonl). \
                       Your pane_id and timestamp are stamped automatically. Use \
                       kind \"diff\" to publish work for review, \"review\" to \
                       approve/reject another pane's diff, \"decision\" for \
                       outcomes such as an opened PR, \"status\" for progress or \
                       blockers."
    )]
    async fn publish_record(
        &self,
        Parameters(args): Parameters<PublishRecordArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        // Reject a hand-rolled delegate-to: tag — routing a delegation has its
        // own tool, and letting it in here would bypass the target-pane check.
        if args.tags.iter().any(|t| t.starts_with("delegate-to:")) {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "Use the `delegate` tool to route work to a pane; \
                 delegate-to: tags are not accepted here.",
            )]));
        }

        let record = TeamRecord {
            ts: now_rfc3339(),
            pane_id: Some(self.pane_id),
            tags: args.tags,
            kind: args.kind,
            body: args.body,
        };
        {
            // Bodies can be a full diff — well past the size at which an
            // O_APPEND write is atomic, so two panes publishing at once could
            // otherwise interleave into a corrupt JSONL line.
            let _lock = ProjectLock::acquire(self.dir()).map_err(|e| internal(e.to_string()))?;
            scratchpad::append(self.dir(), &record)
                .map_err(|e| internal(format!("failed to append scratchpad record: {e:#}")))?;
        }
        json_result(&record)
    }

    #[tool(
        description = "Delegate work to another pane. Appends a delegation record \
                       that the CLI routes into the target pane's input queue. \
                       Call list_panes first to pick a valid target."
    )]
    async fn delegate(
        &self,
        Parameters(args): Parameters<DelegateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.target_pane_id == self.pane_id {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "Cannot delegate to yourself.",
            )]));
        }

        // Validate the target against `.apas` rather than letting the record
        // land unroutable. Under the old tag convention a typo'd pane id
        // produced a record nothing ever picked up, and no error anywhere.
        let panes = self.load_panes().map_err(|e| internal(e.to_string()))?;
        let Some(target) = panes.iter().find(|p| p.pane_id == args.target_pane_id) else {
            let known: Vec<String> = panes.iter().map(|p| p.pane_id.to_string()).collect();
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "No pane {} in this project. Known panes: {}.",
                args.target_pane_id,
                known.join(", ")
            ))]));
        };
        if target.kind == "terminal" {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Pane {} is a terminal pane hosting an interactive TUI; it \
                 cannot take delegations.",
                args.target_pane_id
            ))]));
        }

        let record = TeamRecord {
            ts: now_rfc3339(),
            pane_id: Some(self.pane_id),
            tags: vec![
                format!("delegate-to:{}", args.target_pane_id),
                format!("task:{}", args.task_id),
            ],
            kind: "delegation".to_string(),
            body: args.body,
        };
        {
            let _lock = ProjectLock::acquire(self.dir()).map_err(|e| internal(e.to_string()))?;
            scratchpad::append(self.dir(), &record)
                .map_err(|e| internal(format!("failed to append delegation: {e:#}")))?;
        }
        json_result(&record)
    }

    #[tool(
        description = "Read team scratchpad records newer than a cursor. Omit \
                       `since` on your first call to get recent history; pass the \
                       returned next_cursor on later calls. Replaces manual \
                       tail/jq cursor handling."
    )]
    async fn read_records(
        &self,
        Parameters(args): Parameters<ReadRecordsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let all = scratchpad::read_all(self.dir())
            .map_err(|e| internal(format!("failed to read scratchpad: {e:#}")))?;

        let mut matching: Vec<TeamRecord> = all
            .into_iter()
            .filter(|r| match args.since.as_deref() {
                // Lexicographic compare is correct here because timestamps are
                // RFC3339 and the file is append-ordered.
                Some(cursor) => r.ts.as_str() > cursor,
                None => true,
            })
            .filter(|r| args.tags.iter().all(|want| r.tags.iter().any(|t| t == want)))
            .collect();

        let limit = args
            .limit
            .unwrap_or(DEFAULT_COLD_READ)
            .min(MAX_RECORDS_PER_READ)
            .max(1);

        let has_more;
        if args.since.is_none() {
            // Cold read: the useful window is the newest records, not the
            // oldest — take from the tail.
            has_more = matching.len() > limit;
            let skip = matching.len().saturating_sub(limit);
            matching = matching.split_off(skip);
        } else {
            has_more = matching.len() > limit;
            matching.truncate(limit);
        }

        // Only advance on a non-empty page, so an idle poll can't skip a
        // record that lands a moment later within the same timestamp.
        let next_cursor = matching
            .last()
            .map(|r| r.ts.clone())
            .or_else(|| args.since.clone());

        json_result(&ReadRecordsResult {
            records: matching,
            next_cursor,
            has_more,
        })
    }

    #[tool(description = "Read the current team-todo.md state: Global TODOs, their \
                          statuses, and per-pane subtasks.")]
    async fn read_team_todo(&self) -> Result<CallToolResult, ErrorData> {
        let todo = team_todo::load(self.dir())
            .map_err(|e| internal(format!("failed to load team-todo.md: {e:#}")))?;
        json_result(&team_todo::to_wire(&todo))
    }

    #[tool(
        description = "Propose a new Global TODO. Lands as status: proposed, \
                       origin: tech-lead, awaiting human approval in the Overview \
                       (unless the project has auto_approve_todos enabled)."
    )]
    async fn propose_todo(
        &self,
        Parameters(args): Parameters<ProposeTodoArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let _lock = ProjectLock::acquire(self.dir()).map_err(|e| internal(e.to_string()))?;
        let mut todo = team_todo::load(self.dir())
            .map_err(|e| internal(format!("failed to load team-todo.md: {e:#}")))?;
        // next_global_id() reads the current max id, so it must be computed
        // under the same lock as the save or two panes get the same id.
        let id = todo.next_global_id();
        todo.push_global(team_todo::GlobalTodo {
            id: id.clone(),
            title: args.title,
            status: GlobalStatus::Proposed,
            origin: team_todo::Origin::TechLead,
            notes: Vec::new(),
            // A freshly proposed TODO has no PR yet; the owning Developer
            // appends one when it opens the PR after Reviewer approval.
            prs: Vec::new(),
            body: args.body,
        });
        team_todo::save(self.dir(), &todo)
            .map_err(|e| internal(format!("failed to save team-todo.md: {e:#}")))?;
        json_result(&serde_json::json!({ "todo_id": id, "status": "proposed" }))
    }

    #[tool(description = "Move a Global TODO to a new status (proposed, approved, \
                          rejected, in_progress, under_review, pr_open, done).")]
    async fn update_todo_status(
        &self,
        Parameters(args): Parameters<UpdateTodoStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(status) = GlobalStatus::from_str(&args.status) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Unknown status {:?}. Valid: proposed, approved, rejected, \
                 in_progress, under_review, pr_open, done.",
                args.status
            ))]));
        };
        let _lock = ProjectLock::acquire(self.dir()).map_err(|e| internal(e.to_string()))?;
        let mut todo = team_todo::load(self.dir())
            .map_err(|e| internal(format!("failed to load team-todo.md: {e:#}")))?;
        let Some(previous) = todo.set_global_status(&args.todo_id, status) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "No Global TODO {:?} in team-todo.md.",
                args.todo_id
            ))]));
        };
        team_todo::save(self.dir(), &todo)
            .map_err(|e| internal(format!("failed to save team-todo.md: {e:#}")))?;
        json_result(&serde_json::json!({
            "todo_id": args.todo_id,
            "from": previous.as_str(),
            "to": status.as_str(),
        }))
    }

    #[tool(description = "Read project_goal.md, the human-facing statement of what \
                          the team is building.")]
    async fn read_project_goal(&self) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![ContentBlock::text(
            crate::manager::read_project_goal(self.dir()),
        )]))
    }

    #[tool(
        description = "Replace project_goal.md. Manager role only — other panes \
                       should propose changes to the Manager instead of writing \
                       here."
    )]
    async fn write_project_goal(
        &self,
        Parameters(args): Parameters<WriteProjectGoalArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let _lock = ProjectLock::acquire(self.dir()).map_err(|e| internal(e.to_string()))?;
        crate::manager::write_project_goal(self.dir(), &args.content)
            .map_err(|e| internal(format!("failed to write project_goal.md: {e:#}")))?;
        json_result(&serde_json::json!({
            "written": true,
            "bytes": args.content.len(),
        }))
    }

    #[tool(description = "List the project's panes, so you can pick a valid \
                          delegation target and see which roles are staffed.")]
    async fn list_panes(&self) -> Result<CallToolResult, ErrorData> {
        let panes = self.load_panes().map_err(|e| internal(e.to_string()))?;
        json_result(&panes)
    }

    fn load_panes(&self) -> Result<Vec<PaneSummary>> {
        // Read `.apas` directly rather than via `get_or_create_project`: that
        // helper *creates* the project when the file is absent and registers
        // it in the user's `~/.config/apas/projects.json`. An MCP server only
        // ever serves a project that already exists, and minting one as a side
        // effect of `list_panes` polluted the real registry with an entry per
        // test temp dir (43 dead paths before this was caught) -- which the
        // daemon then treats as projects to spawn.
        let apas_path = crate::project::get_apas_path(self.dir());
        if !apas_path.exists() {
            return Ok(Vec::new());
        }
        let metadata: crate::project::ProjectMetadata =
            serde_json::from_str(&std::fs::read_to_string(&apas_path)?)
                .with_context(|| format!("parsing {}", apas_path.display()))?;
        Ok(metadata
            .panes
            .iter()
            .map(|p| PaneSummary {
                pane_id: p.pane_id,
                label: p.label.clone(),
                role: p.role.clone(),
                provider: format!("{:?}", p.provider).to_lowercase(),
                managed: p.managed,
                kind: if p.kind.is_terminal() {
                    "terminal".to_string()
                } else {
                    "agent".to_string()
                },
            })
            .collect())
    }
}

#[tool_handler]
impl ServerHandler for ApasMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "APAS team-mode tools. You are pane {pane}. Publish work with \
             publish_record, route work with delegate, and poll for inbound work \
             with read_records (carry next_cursor forward). Do not hand-edit \
             .apas-team.jsonl or team-todo.md — these tools keep both consistent."
                .replace("{pane}", &self.pane_id.to_string()),
        )
    }
}

/// Name the MCP server registers under. Tools appear to the agent as
/// `mcp__apas__<tool>` (claude) / `apas.<tool>` (codex), so keep it short and
/// stable — role prompts reference these names.
pub const SERVER_NAME: &str = "apas";

/// Provider-specific CLI flags that point a spawned pane at its own
/// `apas mcp-server` child.
///
/// Both providers get the same stdio server; only the way it's declared
/// differs. Codex reuses the `-c` override channel that already carries
/// `model_reasoning_effort`, so nothing new is needed on its side.
///
/// `pane_id` is baked into the args here rather than discovered by the agent:
/// that is what makes published records unspoofable.
pub fn mcp_server_flags(
    provider: &shared::Provider,
    apas_bin: &str,
    project_dir: &str,
    pane_id: u32,
) -> Vec<String> {
    let args = serde_json::json!([
        "mcp-server",
        "--project-dir",
        project_dir,
        "--pane-id",
        pane_id.to_string(),
    ]);

    match provider {
        // MiniMax / GLM / DeepSeek are the claude binary pointed at another
        // backend, so they take the claude flag shape too.
        shared::Provider::Claude
        | shared::Provider::Minimax
        | shared::Provider::Glm
        | shared::Provider::Deepseek => {
            let config = serde_json::json!({
                "mcpServers": {
                    SERVER_NAME: { "command": apas_bin, "args": args }
                }
            });
            vec!["--mcp-config".to_string(), config.to_string()]
        }
        shared::Provider::Codex => vec![
            "-c".to_string(),
            format!("mcp_servers.{SERVER_NAME}.command=\"{apas_bin}\""),
            "-c".to_string(),
            format!("mcp_servers.{SERVER_NAME}.args={args}"),
        ],
        // Not verified against these runtimes; a bad flag would break the
        // spawn outright, so stay out of their way.
        shared::Provider::Opencode | shared::Provider::CursorAgent => Vec::new(),
    }
}

/// Serve the tool surface on stdio until the client disconnects.
pub async fn run(project_dir: PathBuf, pane_id: u32) -> Result<()> {
    let service = ApasMcpServer::new(project_dir, pane_id)
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Provider;
    use std::collections::HashSet;
    use tempfile::TempDir;

    const SEED_TODO: &str = "# Team TODO\n\n## Global TODOs\n\n\
        ### [TODO-001] Seed item\nstatus: approved\norigin: user\n\nBody.\n";

    fn project() -> TempDir {
        let dir = TempDir::new().expect("temp project");
        std::fs::write(dir.path().join("team-todo.md"), SEED_TODO).unwrap();
        std::fs::write(dir.path().join(".apas-team.jsonl"), "").unwrap();
        dir
    }

    fn server(dir: &TempDir, pane_id: u32) -> ApasMcpServer {
        ApasMcpServer::new(dir.path().to_path_buf(), pane_id)
    }

    /// Pull the text payload out of a tool result (every tool returns either a
    /// JSON string or a plain message in a single text block).
    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|b| b.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default()
    }

    fn is_err(result: &CallToolResult) -> bool {
        result.is_error.unwrap_or(false)
    }

    // -----------------------------------------------------------------
    // The lock. This is the regression that matters most: the bug it fixes
    // produced NO error on either side -- one pane's change was simply gone.
    // Verified by hand across two processes when the fix landed, but that
    // evidence lived in a scratch dir; this keeps it in the suite.
    //
    // `flock` is per open-file-description, so two `ProjectLock::acquire`
    // calls conflict even inside one process, which is what makes this
    // testable without spawning children.
    // -----------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_proposals_do_not_lose_updates() {
        let dir = project();
        const N: u32 = 8;

        let mut handles = Vec::new();
        for i in 0..N {
            // Distinct pane ids: this is the multi-pane shape, where each pane
            // has its own server process in production.
            let srv = server(&dir, 100 + i);
            handles.push(tokio::spawn(async move {
                srv.propose_todo(Parameters(ProposeTodoArgs {
                    title: format!("Proposal {i}"),
                    body: "body".to_string(),
                }))
                .await
            }));
        }
        for h in handles {
            let result = h.await.expect("task panicked").expect("tool errored");
            assert!(!is_err(&result), "propose_todo reported an error");
        }

        let todo = team_todo::load(dir.path()).expect("reload team-todo.md");
        assert_eq!(
            todo.globals.len() as u32,
            N + 1,
            "expected the seed entry plus {N} proposals; a shorter list means a \
             concurrent save clobbered an earlier one"
        );
        let ids: HashSet<&str> = todo.globals.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(
            ids.len() as u32,
            N + 1,
            "duplicate TODO ids: next_global_id() was computed outside the lock"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_status_update_survives_parallel_proposals() {
        // The exact interleaving that lost an update before the fix: a status
        // flip racing a batch of proposals, all read-modify-writing one file.
        let dir = project();

        let updater = server(&dir, 7);
        let update = tokio::spawn(async move {
            updater
                .update_todo_status(Parameters(UpdateTodoStatusArgs {
                    todo_id: "TODO-001".to_string(),
                    status: "done".to_string(),
                }))
                .await
        });
        let mut handles = vec![];
        for i in 0..6u32 {
            let srv = server(&dir, 200 + i);
            handles.push(tokio::spawn(async move {
                srv.propose_todo(Parameters(ProposeTodoArgs {
                    title: format!("P{i}"),
                    body: "b".to_string(),
                }))
                .await
            }));
        }
        update.await.unwrap().unwrap();
        for h in handles {
            h.await.unwrap().unwrap();
        }

        let todo = team_todo::load(dir.path()).unwrap();
        let seed = todo.find_global("TODO-001").expect("seed entry survived");
        assert_eq!(
            seed.status,
            GlobalStatus::Done,
            "the status flip was silently discarded by a concurrent save"
        );
        assert_eq!(todo.globals.len(), 7, "seed + 6 proposals");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_publishes_all_land_as_valid_jsonl() {
        // Record bodies can be a full diff, past the size where an O_APPEND
        // write is atomic -- and this project's files live on NFS, where that
        // guarantee is weaker still. Interleaving would corrupt the line.
        let dir = project();
        let big = "x".repeat(64 * 1024);

        let mut handles = vec![];
        for i in 0..6u32 {
            let srv = server(&dir, 300 + i);
            let body = big.clone();
            handles.push(tokio::spawn(async move {
                srv.publish_record(Parameters(PublishRecordArgs {
                    kind: "diff".to_string(),
                    body,
                    tags: vec![],
                }))
                .await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }

        let raw = std::fs::read_to_string(dir.path().join(".apas-team.jsonl")).unwrap();
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 6, "one line per publish");
        for line in lines {
            serde_json::from_str::<TeamRecord>(line).expect("each line is a complete record");
        }
    }

    // -----------------------------------------------------------------
    // Tool handlers
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn publish_record_stamps_pane_id_server_side() {
        let dir = project();
        let result = server(&dir, 42)
            .publish_record(Parameters(PublishRecordArgs {
                kind: "diff".to_string(),
                body: "implemented X".to_string(),
                tags: vec!["task:TODO-001".to_string()],
            }))
            .await
            .unwrap();
        assert!(!is_err(&result));

        let stored = scratchpad::read_all(dir.path()).unwrap();
        assert_eq!(stored.len(), 1);
        // The agent never supplied this; the server did.
        assert_eq!(stored[0].pane_id, Some(42));
        assert_eq!(stored[0].tags, vec!["task:TODO-001".to_string()]);
        assert!(!stored[0].ts.is_empty(), "ts stamped at append time");
    }

    #[tokio::test]
    async fn publish_record_refuses_hand_rolled_delegation_tags() {
        // Routing has its own tool, which validates the target. Allowing a raw
        // delegate-to: tag here would let an agent bypass that check.
        let dir = project();
        let result = server(&dir, 7)
            .publish_record(Parameters(PublishRecordArgs {
                kind: "status".to_string(),
                body: "sneaky".to_string(),
                tags: vec!["delegate-to:9".to_string()],
            }))
            .await
            .unwrap();

        assert!(is_err(&result));
        assert!(text_of(&result).contains("delegate"));
        assert!(
            scratchpad::read_all(dir.path()).unwrap().is_empty(),
            "the rejected record must not reach the file"
        );
    }

    #[tokio::test]
    async fn delegate_rejects_unknown_target_instead_of_writing_a_dead_record() {
        // Under the old tag convention a typo'd pane id produced a record that
        // nothing ever routed, and reported success.
        let dir = project();
        let result = server(&dir, 7)
            .delegate(Parameters(DelegateArgs {
                target_pane_id: 999,
                task_id: "TODO-001".to_string(),
                body: "do it".to_string(),
            }))
            .await
            .unwrap();

        assert!(is_err(&result));
        let msg = text_of(&result);
        assert!(msg.contains("999"), "names the bad id: {msg}");
        assert!(msg.contains("Known panes"), "lists valid targets: {msg}");
        assert!(scratchpad::read_all(dir.path()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn delegate_refuses_to_target_self() {
        let dir = project();
        let result = server(&dir, 7)
            .delegate(Parameters(DelegateArgs {
                target_pane_id: 7,
                task_id: "TODO-001".to_string(),
                body: "loop".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_err(&result));
        assert!(scratchpad::read_all(dir.path()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn read_records_cursor_returns_only_newer_records() {
        let dir = project();
        let srv = server(&dir, 7);

        srv.publish_record(Parameters(PublishRecordArgs {
            kind: "status".to_string(),
            body: "first".to_string(),
            tags: vec![],
        }))
        .await
        .unwrap();

        let cold = srv
            .read_records(Parameters(ReadRecordsArgs {
                since: None,
                tags: vec![],
                limit: None,
            }))
            .await
            .unwrap();
        let cold: serde_json::Value = serde_json::from_str(&text_of(&cold)).unwrap();
        assert_eq!(cold["records"].as_array().unwrap().len(), 1);
        let cursor = cold["next_cursor"].as_str().unwrap().to_string();

        srv.publish_record(Parameters(PublishRecordArgs {
            kind: "status".to_string(),
            body: "second".to_string(),
            tags: vec![],
        }))
        .await
        .unwrap();

        let warm = srv
            .read_records(Parameters(ReadRecordsArgs {
                since: Some(cursor),
                tags: vec![],
                limit: None,
            }))
            .await
            .unwrap();
        let warm: serde_json::Value = serde_json::from_str(&text_of(&warm)).unwrap();
        let bodies: Vec<&str> = warm["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["body"].as_str().unwrap())
            .collect();
        assert_eq!(bodies, vec!["second"], "cursor must exclude the first record");
    }

    #[tokio::test]
    async fn read_records_empty_poll_keeps_the_cursor_put() {
        // An idle poll must not advance past a record that lands moments later
        // inside the same timestamp.
        let dir = project();
        let srv = server(&dir, 7);
        srv.publish_record(Parameters(PublishRecordArgs {
            kind: "status".to_string(),
            body: "only".to_string(),
            tags: vec![],
        }))
        .await
        .unwrap();

        let first = srv
            .read_records(Parameters(ReadRecordsArgs {
                since: None,
                tags: vec![],
                limit: None,
            }))
            .await
            .unwrap();
        let first: serde_json::Value = serde_json::from_str(&text_of(&first)).unwrap();
        let cursor = first["next_cursor"].as_str().unwrap().to_string();

        let idle = srv
            .read_records(Parameters(ReadRecordsArgs {
                since: Some(cursor.clone()),
                tags: vec![],
                limit: None,
            }))
            .await
            .unwrap();
        let idle: serde_json::Value = serde_json::from_str(&text_of(&idle)).unwrap();
        assert!(idle["records"].as_array().unwrap().is_empty());
        assert_eq!(idle["next_cursor"].as_str().unwrap(), cursor);
    }

    #[tokio::test]
    async fn read_records_filters_by_tag() {
        let dir = project();
        let srv = server(&dir, 7);
        for (kind, tag) in [("diff", "task:TODO-001"), ("status", "task:TODO-002")] {
            srv.publish_record(Parameters(PublishRecordArgs {
                kind: kind.to_string(),
                body: kind.to_string(),
                tags: vec![tag.to_string()],
            }))
            .await
            .unwrap();
        }

        let filtered = srv
            .read_records(Parameters(ReadRecordsArgs {
                since: None,
                tags: vec!["task:TODO-002".to_string()],
                limit: None,
            }))
            .await
            .unwrap();
        let filtered: serde_json::Value = serde_json::from_str(&text_of(&filtered)).unwrap();
        let recs = filtered["records"].as_array().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["body"].as_str().unwrap(), "status");
    }

    #[tokio::test]
    async fn update_todo_status_reports_unknown_id_and_unknown_status() {
        let dir = project();
        let srv = server(&dir, 7);

        let bad_status = srv
            .update_todo_status(Parameters(UpdateTodoStatusArgs {
                todo_id: "TODO-001".to_string(),
                status: "banana".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_err(&bad_status));
        assert!(text_of(&bad_status).contains("Valid:"), "lists valid statuses");

        let bad_id = srv
            .update_todo_status(Parameters(UpdateTodoStatusArgs {
                todo_id: "TODO-404".to_string(),
                status: "done".to_string(),
            }))
            .await
            .unwrap();
        assert!(is_err(&bad_id));

        // Neither failure may have mutated the file.
        let todo = team_todo::load(dir.path()).unwrap();
        assert_eq!(
            todo.find_global("TODO-001").unwrap().status,
            GlobalStatus::Approved
        );
    }

    #[tokio::test]
    async fn project_goal_round_trips_through_the_tools() {
        let dir = project();
        let srv = server(&dir, 151);
        srv.write_project_goal(Parameters(WriteProjectGoalArgs {
            content: "# Project Goal\n\nShip the thing.\n".to_string(),
        }))
        .await
        .unwrap();

        let read = srv.read_project_goal().await.unwrap();
        assert!(text_of(&read).contains("Ship the thing."));
    }

    #[test]
    fn claude_flags_carry_a_parseable_mcp_config() {
        let flags = mcp_server_flags(&Provider::Claude, "/usr/local/bin/apas", "/proj", 42);
        assert_eq!(flags[0], "--mcp-config");
        let cfg: serde_json::Value = serde_json::from_str(&flags[1]).expect("valid JSON");
        let server = &cfg["mcpServers"]["apas"];
        assert_eq!(server["command"], "/usr/local/bin/apas");
        assert_eq!(
            server["args"],
            serde_json::json!(["mcp-server", "--project-dir", "/proj", "--pane-id", "42"])
        );
    }

    #[test]
    fn codex_flags_use_the_existing_c_override_channel() {
        let flags = mcp_server_flags(&Provider::Codex, "/usr/local/bin/apas", "/proj", 7);
        // Same `-c key=value` shape already used for model_reasoning_effort.
        assert_eq!(flags.iter().filter(|f| *f == "-c").count(), 2);
        assert!(flags
            .iter()
            .any(|f| f == r#"mcp_servers.apas.command="/usr/local/bin/apas""#));
        let args_flag = flags
            .iter()
            .find(|f| f.starts_with("mcp_servers.apas.args="))
            .expect("args override present");
        let json = args_flag.trim_start_matches("mcp_servers.apas.args=");
        let parsed: serde_json::Value = serde_json::from_str(json).expect("valid JSON array");
        assert_eq!(parsed[4], "7", "pane id must be baked into the args");
    }

    #[test]
    fn claude_backed_providers_all_get_the_claude_shape() {
        // MiniMax/GLM/DeepSeek run the claude binary behind different env, so
        // they must take `--mcp-config`, not codex's `-c` overrides.
        for p in [Provider::Minimax, Provider::Glm, Provider::Deepseek] {
            let flags = mcp_server_flags(&p, "/bin/apas", "/proj", 1);
            assert_eq!(flags[0], "--mcp-config", "{p:?} should use claude flags");
        }
    }

    #[test]
    fn unverified_providers_get_no_flags() {
        // A wrong flag here would break the spawn outright rather than just
        // omitting the tools.
        for p in [Provider::Opencode, Provider::CursorAgent] {
            assert!(
                mcp_server_flags(&p, "/bin/apas", "/proj", 1).is_empty(),
                "{p:?} must not receive unverified flags"
            );
        }
    }

    #[test]
    fn pane_id_is_not_taken_from_agent_input() {
        // The whole point of stamping server-side: the id in the spawn args is
        // the only place a pane identity is declared.
        let flags = mcp_server_flags(&Provider::Claude, "/bin/apas", "/proj", 151);
        assert!(flags[1].contains("\"151\""));
    }
}
