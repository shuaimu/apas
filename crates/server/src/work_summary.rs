use crate::{
    config::SummaryConfig,
    db::Database,
    session::SessionManager,
    storage::{FileStorage, PaneWorkSummaryDocument, StoredMessage},
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use dashmap::DashMap;
use regex::Regex;
use sha2::{Digest, Sha256};
use shared::{
    PaneWorkSummary, PaneWorkSummaryAvailability, PaneWorkSummaryGenerationJob,
    PaneWorkSummaryGenerationResult, PaneWorkSummaryResultKind, PaneWorkSummaryStage,
    PaneWorkSummaryStatus, PaneWorkSummaryWindowKind, Provider, ServerToCli, ServerToWeb,
    PANE_WORK_SUMMARY_CAPABILITY, PANE_WORK_SUMMARY_PROTOCOL_VERSION,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use uuid::Uuid;

pub const SUMMARY_WINDOW_HOURS: i64 = 3;
const MAX_FIELD_BYTES: usize = 2 * 1024;
const MAX_TOOL_RESULT_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRecord {
    pub timestamp: DateTime<Utc>,
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceWindow {
    pub session_id: Uuid,
    pub pane_id: u32,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub records: Vec<CanonicalRecord>,
    pub source_digest: String,
    pub source_through: DateTime<Utc>,
    pub source_through_id: String,
}

impl SourceWindow {
    pub fn canonical_source(&self) -> String {
        self.records
            .iter()
            .map(|record| {
                format!(
                    "[{} id={}] {}",
                    record.timestamp.to_rfc3339(),
                    record.id,
                    record.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkManifest {
    pub chunks: Vec<String>,
    /// True means dispatch must fail explicitly; chunks retain the newest
    /// accepted material only to make diagnostics deterministic.
    pub overflowed: bool,
    pub total_chunks: usize,
}

pub fn window_bounds(timestamp: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let seconds = SUMMARY_WINDOW_HOURS * 60 * 60;
    let start_seconds = timestamp.timestamp().div_euclid(seconds) * seconds;
    let start = Utc
        .timestamp_opt(start_seconds, 0)
        .single()
        .expect("valid UTC summary boundary");
    (start, start + Duration::hours(SUMMARY_WINDOW_HOURS))
}

pub fn parse_pane_id(
    raw_pane_type: Option<&str>,
    single_pane_fallback: Option<u32>,
) -> Option<u32> {
    let raw = match raw_pane_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => raw,
        None => return single_pane_fallback,
    };
    if raw.eq_ignore_ascii_case("deadloop") {
        return Some(shared::PANE_ID_DEADLOOP);
    }
    if raw.eq_ignore_ascii_case("interactive") {
        return Some(shared::PANE_ID_INTERACTIVE);
    }
    if let Ok(id) = raw.parse::<u32>() {
        return Some(id);
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("deadloop") {
        return Some(shared::PANE_ID_DEADLOOP);
    }
    if lower.contains("interactive") {
        return Some(shared::PANE_ID_INTERACTIVE);
    }
    let digits = lower
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    digits.parse().ok().or(single_pane_fallback)
}

pub fn build_source_windows(
    session_id: Uuid,
    messages: &[StoredMessage],
    single_pane_fallback: Option<u32>,
) -> Vec<SourceWindow> {
    let mut grouped: BTreeMap<(u32, DateTime<Utc>), Vec<CanonicalRecord>> = BTreeMap::new();

    for message in messages {
        let Some(pane_id) = parse_pane_id(message.pane_type.as_deref(), single_pane_fallback)
        else {
            continue;
        };
        let Ok(timestamp) = DateTime::parse_from_rfc3339(&message.created_at) else {
            continue;
        };
        let timestamp = timestamp.with_timezone(&Utc);
        let Some(text) = normalize_message(message) else {
            continue;
        };
        let (window_start, _) = window_bounds(timestamp);
        grouped
            .entry((pane_id, window_start))
            .or_default()
            .push(CanonicalRecord {
                timestamp,
                id: clip_utf8(&message.id, 256),
                text,
            });
    }

    grouped
        .into_iter()
        .map(|((pane_id, window_start), mut records)| {
            records.sort_by(|left, right| {
                left.timestamp
                    .cmp(&right.timestamp)
                    .then_with(|| left.id.cmp(&right.id))
            });
            let last = records.last().expect("group contains a record");
            let source_through = last.timestamp;
            let source_through_id = last.id.clone();
            let window_end = window_start + Duration::hours(SUMMARY_WINDOW_HOURS);
            let source_digest =
                digest_records(session_id, pane_id, window_start, window_end, &records);
            SourceWindow {
                session_id,
                pane_id,
                window_start,
                window_end,
                records,
                source_digest,
                source_through,
                source_through_id,
            }
        })
        .collect()
}

fn normalize_message(message: &StoredMessage) -> Option<String> {
    let kind = message.message_type.trim().to_ascii_lowercase();
    let role = message.role.trim().to_ascii_lowercase();
    let prefix = match role.as_str() {
        "user" => "USER",
        "assistant" => "ASSISTANT",
        _ => "EVENT",
    };

    match kind.as_str() {
        "text" | "result" | "error" | "status" => {
            let content = redact_secrets(&clip_utf8(message.content.trim(), MAX_FIELD_BYTES));
            (!content.is_empty()).then(|| format!("{prefix}: {content}"))
        }
        "tool_use" => {
            let value: serde_json::Value = serde_json::from_str(&message.content).ok()?;
            let name = value.get("name")?.as_str()?.trim();
            (!name.is_empty()).then(|| format!("TOOL: {}", clip_utf8(name, 128)))
        }
        "tool_result" => {
            let value: serde_json::Value = serde_json::from_str(&message.content).ok()?;
            let failed = value
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let content = value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let concise = redact_secrets(&clip_utf8(content.trim(), MAX_TOOL_RESULT_BYTES));
            Some(if concise.is_empty() {
                format!(
                    "TOOL RESULT: {}",
                    if failed { "failed" } else { "succeeded" }
                )
            } else {
                format!(
                    "TOOL RESULT {}: {concise}",
                    if failed { "FAILED" } else { "SUCCEEDED" }
                )
            })
        }
        // PTY output, transport state, heartbeats, and usage-only envelopes
        // are deliberately excluded from the summary source.
        _ => None,
    }
}

fn digest_records(
    session_id: Uuid,
    pane_id: u32,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    records: &[CanonicalRecord],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pane-work-summary-v1\0");
    hasher.update(session_id.as_bytes());
    hasher.update(pane_id.to_be_bytes());
    hasher.update(window_start.timestamp().to_be_bytes());
    hasher.update(window_end.timestamp().to_be_bytes());
    for record in records {
        hasher.update(record.timestamp.timestamp_micros().to_be_bytes());
        hasher.update((record.id.len() as u64).to_be_bytes());
        hasher.update(record.id.as_bytes());
        hasher.update((record.text.len() as u64).to_be_bytes());
        hasher.update(record.text.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn chunk_window(
    window: &SourceWindow,
    max_chunk_bytes: usize,
    max_chunks: usize,
) -> ChunkManifest {
    let max_chunk_bytes = max_chunk_bytes.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    for record in &window.records {
        let line = format!(
            "[{} id={}] {}",
            record.timestamp.to_rfc3339(),
            record.id,
            record.text
        );
        let line = clip_utf8(&line, max_chunk_bytes);
        let extra = line.len() + usize::from(!current.is_empty());
        if !current.is_empty() && current.len() + extra > max_chunk_bytes {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(&line);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    let total_chunks = chunks.len();
    let overflowed = total_chunks > max_chunks;
    if overflowed {
        chunks = chunks.split_off(total_chunks - max_chunks.max(1));
    }
    ChunkManifest {
        chunks,
        overflowed,
        total_chunks,
    }
}

pub fn cached_digest_is_stale(cached_digest: &str, window: &SourceWindow) -> bool {
    !cached_digest.is_empty() && cached_digest != window.source_digest
}

fn secret_regex() -> &'static Regex {
    static SECRET: OnceLock<Regex> = OnceLock::new();
    SECRET.get_or_init(|| {
        Regex::new(
            r"(?ix)(?:bearer\s+)[a-z0-9._~+/=-]{8,}|(?:sk|ghp|github_pat)_[a-z0-9_-]{8,}|\bAKIA[A-Z0-9]{12,}\b|(?:api[_-]?key|token|password|secret)\s*[:=]\s*[^\s,;]{6,}",
        )
        .expect("valid secret redaction regex")
    })
}

pub fn redact_secrets(value: &str) -> String {
    secret_regex().replace_all(value, "[REDACTED]").into_owned()
}

fn clip_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…[clipped]", &value[..boundary])
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LogicalJobKey {
    session_id: Uuid,
    pane_id: u32,
    window_start: DateTime<Utc>,
    source_digest: String,
    stage: PaneWorkSummaryStage,
    chunk_index: Option<u32>,
}

#[derive(Debug, Clone)]
struct GenerationTask {
    job: PaneWorkSummaryGenerationJob,
    window_kind: PaneWorkSummaryWindowKind,
    source_message_count: u32,
    source_through: DateTime<Utc>,
    source_through_id: String,
    attempt: u32,
}

impl GenerationTask {
    fn logical_key(&self) -> LogicalJobKey {
        LogicalJobKey {
            session_id: self.job.session_id,
            pane_id: self.job.pane_id,
            window_start: self.job.window_start,
            source_digest: self.job.source_digest.clone(),
            stage: self.job.stage,
            chunk_index: self.job.chunk_index,
        }
    }
}

#[derive(Debug, Clone)]
struct InFlightJob {
    task: GenerationTask,
    cli_id: Uuid,
    started_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ReductionState {
    base: GenerationTask,
    notes: Vec<Option<String>>,
}

#[derive(Default)]
struct SummaryRuntime {
    queued: VecDeque<GenerationTask>,
    logical_jobs: HashSet<LogicalJobKey>,
    in_flight: HashMap<Uuid, InFlightJob>,
    busy_clis: HashSet<Uuid>,
    reductions: HashMap<(Uuid, u32, DateTime<Utc>, String), ReductionState>,
}

#[derive(Debug, Clone, Copy)]
enum ReconcileScope {
    Completed,
    Current,
    Target(DateTime<Utc>),
}

impl ReconcileScope {
    fn includes(self, window: &SourceWindow, now: DateTime<Utc>) -> bool {
        match self {
            Self::Completed => window.window_end <= now,
            Self::Current => window.window_end > now,
            Self::Target(window_start) => window.window_start == window_start,
        }
    }
}

#[derive(Debug, Default)]
pub struct SummaryMetrics {
    scans: std::sync::atomic::AtomicU64,
    scanned_bytes: std::sync::atomic::AtomicU64,
    dispatched: std::sync::atomic::AtomicU64,
    retries: std::sync::atomic::AtomicU64,
    failures: std::sync::atomic::AtomicU64,
    unavailable: std::sync::atomic::AtomicU64,
}

/// Server-owned summary orchestration. The CLI is only an isolated generation
/// worker; scope, source, authorization, cache writes, and result correlation
/// remain here.
pub struct PaneWorkSummaryService {
    db: Database,
    sessions: Arc<SessionManager>,
    storage: FileStorage,
    config: SummaryConfig,
    runtime: Mutex<SummaryRuntime>,
    availability: DashMap<Uuid, PaneWorkSummaryAvailability>,
    refreshes: DashMap<(Uuid, u32, DateTime<Utc>), DateTime<Utc>>,
    recovered_sessions: DashMap<Uuid, ()>,
    recovered_windows: DashMap<(Uuid, u32, DateTime<Utc>), ()>,
    pub metrics: SummaryMetrics,
}

impl PaneWorkSummaryService {
    pub fn new(
        db: Database,
        sessions: Arc<SessionManager>,
        storage: FileStorage,
        config: SummaryConfig,
    ) -> Self {
        Self {
            db,
            sessions,
            storage,
            config,
            runtime: Mutex::new(SummaryRuntime::default()),
            availability: DashMap::new(),
            refreshes: DashMap::new(),
            recovered_sessions: DashMap::new(),
            recovered_windows: DashMap::new(),
            metrics: SummaryMetrics::default(),
        }
    }

    pub async fn reconcile_all(self: &Arc<Self>) {
        if !self.config.enabled {
            return;
        }
        let started = std::time::Instant::now();
        let sessions = match self.storage.list_sessions_with_messages().await {
            Ok(mut sessions) => {
                sessions.sort_unstable();
                sessions.truncate(self.config.max_sessions_per_scan);
                sessions
            }
            Err(error) => {
                tracing::warn!(%error, "Pane summary session scan failed");
                return;
            }
        };
        let mut scanned_bytes = 0_u64;
        for session_id in sessions {
            match self
                .reconcile_session(session_id, None, ReconcileScope::Completed, None)
                .await
            {
                Ok(bytes) => scanned_bytes = scanned_bytes.saturating_add(bytes as u64),
                Err(error) => {
                    tracing::warn!(%session_id, %error, "Pane summary reconciliation failed")
                }
            }
        }
        use std::sync::atomic::Ordering;
        self.metrics.scans.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .scanned_bytes
            .fetch_add(scanned_bytes, Ordering::Relaxed);
        let runtime = self.runtime.lock().await;
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            scanned_bytes,
            queue_depth = runtime.queued.len(),
            in_flight = runtime.in_flight.len(),
            "Pane summary reconciliation complete"
        );
        drop(runtime);
        self.kick_dispatch().await;
    }

    pub async fn list_for_pane(
        &self,
        session_id: Uuid,
        pane_id: u32,
    ) -> anyhow::Result<(Vec<PaneWorkSummary>, PaneWorkSummaryAvailability)> {
        self.list_cached(session_id, pane_id).await
    }

    /// Reconcile only the still-open window after the persisted cache has
    /// already been returned to the requesting client. Historical backfill is
    /// exclusively owned by the periodic completed-window scheduler.
    pub async fn reconcile_current_for_pane(
        self: &Arc<Self>,
        session_id: Uuid,
        pane_id: u32,
    ) -> anyhow::Result<()> {
        self.reconcile_session(session_id, Some(pane_id), ReconcileScope::Current, None)
            .await?;
        self.kick_dispatch().await;
        self.broadcast_pane_snapshot(session_id, pane_id).await
    }

    pub async fn refresh(
        self: &Arc<Self>,
        session_id: Uuid,
        pane_id: u32,
        window_start: Option<DateTime<Utc>>,
    ) -> anyhow::Result<(Vec<PaneWorkSummary>, PaneWorkSummaryAvailability)> {
        let now = Utc::now();
        let target_window = window_start.unwrap_or_else(|| window_bounds(now).0);
        let refresh_key = (session_id, pane_id, target_window);
        if let Some(last) = self.refreshes.get(&refresh_key) {
            if now.signed_duration_since(*last)
                < Duration::seconds(self.config.refresh_throttle_seconds as i64)
            {
                return self.list_cached(session_id, pane_id).await;
            }
        }
        self.refreshes.insert(refresh_key, now);
        self.storage
            .update_pane_work_summaries(&session_id, |document| {
                if let Some(summary) = document.summaries.iter_mut().find(|summary| {
                    summary.pane_id == pane_id && summary.window_start == target_window
                }) {
                    summary.status = PaneWorkSummaryStatus::Stale;
                    summary.error = None;
                    summary.updated_at = Some(Utc::now());
                }
                Ok(())
            })
            .await?;
        self.reconcile_session(
            session_id,
            Some(pane_id),
            ReconcileScope::Target(target_window),
            Some(target_window),
        )
        .await?;
        self.kick_dispatch().await;
        self.list_cached(session_id, pane_id).await
    }

    async fn list_cached(
        &self,
        session_id: Uuid,
        pane_id: u32,
    ) -> anyhow::Result<(Vec<PaneWorkSummary>, PaneWorkSummaryAvailability)> {
        let mut summaries = self
            .storage
            .load_pane_work_summaries(&session_id)
            .await?
            .summaries
            .into_iter()
            .filter(|summary| summary.pane_id == pane_id)
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| right.window_start.cmp(&left.window_start));
        Ok((summaries, self.availability_for(session_id)))
    }

    #[cfg(test)]
    async fn reconcile_completed_for_pane(
        self: &Arc<Self>,
        session_id: Uuid,
        pane_id: u32,
    ) -> anyhow::Result<()> {
        self.reconcile_session(session_id, Some(pane_id), ReconcileScope::Completed, None)
            .await?;
        self.kick_dispatch().await;
        Ok(())
    }

    /// How much of a session's log `reconcile_session` reads, newest-first.
    ///
    /// Scaled off `max_source_bytes` because that is the size at which a single
    /// window stops being summarisable: a generous multiple covers several
    /// panes with several live windows each, and anything past it belongs to
    /// windows that would be rejected as over-bounds regardless.
    fn source_tail_budget(&self) -> u64 {
        const WINDOWS_WORTH_OF_SOURCE: u64 = 64;
        const FLOOR_BYTES: u64 = 4 * 1024 * 1024;
        (self.config.max_source_bytes as u64)
            .saturating_mul(WINDOWS_WORTH_OF_SOURCE)
            .max(FLOOR_BYTES)
    }

    fn availability_for(&self, session_id: Uuid) -> PaneWorkSummaryAvailability {
        if !self.config.enabled {
            return PaneWorkSummaryAvailability::SummarizerDisabled;
        }
        if !self
            .sessions
            .session_supports_capability(&session_id, PANE_WORK_SUMMARY_CAPABILITY)
        {
            return PaneWorkSummaryAvailability::CliUpdateRequired;
        }
        self.availability
            .get(&session_id)
            .map(|entry| *entry)
            .unwrap_or(PaneWorkSummaryAvailability::Available)
    }

    async fn reconcile_session(
        self: &Arc<Self>,
        session_id: Uuid,
        pane_filter: Option<u32>,
        scope: ReconcileScope,
        force_window: Option<DateTime<Utc>>,
    ) -> anyhow::Result<usize> {
        // Bounded read. This runs on a timer over every session, and reading
        // the whole log was what turned one runaway session into an
        // out-of-memory server: 2.9 GB of JSON became roughly 15 GB of structs
        // every sweep. Reading only the tail costs nothing real — a window
        // whose source exceeds `max_source_bytes` is rejected below rather
        // than summarised, so the bytes beyond that budget could never have
        // produced a summary anyway.
        let (messages, source_truncated) = self
            .storage
            .get_messages_tail(&session_id, self.source_tail_budget())
            .await?;
        let scanned_bytes = messages.iter().map(|message| message.content.len()).sum();
        // When the read was truncated, the oldest window we can see is missing
        // its beginning. Summarising it would quietly describe a fragment as
        // if it were the whole window, so it is treated as over-bounds below.
        let partial_window_through = source_truncated
            .then(|| messages.first())
            .flatten()
            .and_then(|message| DateTime::parse_from_rfc3339(&message.created_at).ok())
            .map(|ts| window_bounds(ts.with_timezone(&Utc)).0);
        let panes = {
            let active = self.sessions.get_session_panes(&session_id);
            if active.is_empty() {
                self.storage.load_pane_list(&session_id).await?
            } else {
                active
            }
        };
        let fallback = (panes.len() == 1).then(|| panes[0].pane_id);
        let providers = panes
            .iter()
            .map(|pane| (pane.pane_id, pane.provider))
            .collect::<HashMap<_, _>>();
        let now = Utc::now();
        let pane_windows = build_source_windows(session_id, &messages, fallback)
            .into_iter()
            .filter(|window| pane_filter.is_none_or(|pane| window.pane_id == pane))
            .collect::<Vec<_>>();
        let retained_window_keys = pane_windows
            .iter()
            .map(|window| (window.pane_id, window.window_start))
            .collect::<HashSet<_>>();
        let mut windows = pane_windows
            .into_iter()
            .filter(|window| scope.includes(window, now))
            .collect::<Vec<_>>();
        windows.sort_by(|left, right| right.window_start.cmp(&left.window_start));

        let first_recovery = self.recovered_sessions.insert(session_id, ()).is_none();
        let document_result = if first_recovery {
            self.storage.recover_pane_work_summaries(&session_id).await
        } else {
            self.storage.load_pane_work_summaries(&session_id).await
        };
        let mut document = match document_result {
            Ok(document) => document,
            Err(error) => {
                tracing::warn!(%session_id, %error, "Pane summary cache is unreadable");
                return Err(error);
            }
        };
        for summary in &mut document.summaries {
            if pane_filter.is_none_or(|pane| summary.pane_id == pane)
                && !retained_window_keys.contains(&(summary.pane_id, summary.window_start))
                && summary.window_end < now - Duration::days(7)
                && matches!(
                    summary.status,
                    PaneWorkSummaryStatus::Queued
                        | PaneWorkSummaryStatus::Generating
                        | PaneWorkSummaryStatus::Stale
                        | PaneWorkSummaryStatus::Failed
                )
            {
                summary.status = PaneWorkSummaryStatus::SourceExpired;
                summary.error = Some(
                    "The retained conversation source expired before generation completed"
                        .to_string(),
                );
                summary.updated_at = Some(now);
            }
        }
        let mut tasks = Vec::new();
        for window in windows {
            let force = force_window == Some(window.window_start);
            let kind = if window.window_end <= now {
                PaneWorkSummaryWindowKind::Completed
            } else {
                PaneWorkSummaryWindowKind::Current
            };
            let existing_index = document.summaries.iter().position(|summary| {
                summary.pane_id == window.pane_id && summary.window_start == window.window_start
            });
            let should_queue;
            if let Some(index) = existing_index {
                let existing = &mut document.summaries[index];
                // The persisted queue survives a server restart, but the process-local
                // task queue does not. Rebuild queued work during the first reconciliation
                // for the session (recovery also converts interrupted Generating records
                // back to Queued).
                let rebuild_recovered_task = existing.status == PaneWorkSummaryStatus::Queued
                    && self
                        .recovered_windows
                        .insert((session_id, window.pane_id, window.window_start), ())
                        .is_none();
                if existing.source_digest != window.source_digest {
                    existing.status = PaneWorkSummaryStatus::Stale;
                    existing.source_digest = window.source_digest.clone();
                    existing.source_message_count = window.records.len() as u32;
                    existing.source_through = Some(window.source_through);
                    existing.source_through_id = Some(window.source_through_id.clone());
                    existing.updated_at = Some(now);
                    should_queue = true;
                } else if needs_completed_replacement(existing, kind) {
                    existing.window_kind = PaneWorkSummaryWindowKind::Completed;
                    existing.status = PaneWorkSummaryStatus::Queued;
                    existing.updated_at = Some(now);
                    should_queue = true;
                } else if matches!(
                    existing.status,
                    PaneWorkSummaryStatus::Complete
                        | PaneWorkSummaryStatus::Partial
                        | PaneWorkSummaryStatus::Queued
                        | PaneWorkSummaryStatus::Generating
                ) && !force
                    && !rebuild_recovered_task
                {
                    continue;
                } else if matches!(
                    existing.status,
                    PaneWorkSummaryStatus::Failed | PaneWorkSummaryStatus::SourceExpired
                ) && !force
                {
                    continue;
                } else {
                    should_queue = true;
                }
            } else {
                document.summaries.push(PaneWorkSummary {
                    protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
                    session_id,
                    pane_id: window.pane_id,
                    window_start: window.window_start,
                    window_end: window.window_end,
                    window_kind: kind,
                    status: PaneWorkSummaryStatus::Queued,
                    summary: None,
                    source_digest: window.source_digest.clone(),
                    source_message_count: window.records.len() as u32,
                    source_through: Some(window.source_through),
                    source_through_id: Some(window.source_through_id.clone()),
                    generated_at: None,
                    updated_at: Some(now),
                    provider: None,
                    model: None,
                    attempts: 0,
                    error: None,
                });
                should_queue = true;
            }
            if !should_queue {
                continue;
            }
            self.recovered_windows
                .insert((session_id, window.pane_id, window.window_start), ());

            let source_bytes = window.canonical_source().len();
            let manifest =
                chunk_window(&window, self.config.max_chunk_bytes, self.config.max_chunks);
            // A window at or before the truncation boundary is missing its
            // oldest records, so its source is both incomplete and — since it
            // filled the read budget — far larger than the cap allows.
            let partial = partial_window_through
                .is_some_and(|boundary| window.window_start <= boundary);
            if partial || source_bytes > self.config.max_source_bytes || manifest.overflowed {
                if let Some(summary) = document.summaries.iter_mut().find(|summary| {
                    summary.pane_id == window.pane_id && summary.window_start == window.window_start
                }) {
                    summary.status = PaneWorkSummaryStatus::Failed;
                    summary.error = Some(format!(
                        "Summary source exceeds configured bounds ({} bytes, {} chunks)",
                        source_bytes, manifest.total_chunks
                    ));
                    summary.updated_at = Some(now);
                }
                continue;
            }

            let chunk_count = manifest.chunks.len() as u32;
            let reduction_key = (
                session_id,
                window.pane_id,
                window.window_start,
                window.source_digest.clone(),
            );
            let mut chunk_tasks = manifest
                .chunks
                .into_iter()
                .enumerate()
                .map(|(index, content)| GenerationTask {
                    job: PaneWorkSummaryGenerationJob {
                        protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
                        job_id: Uuid::new_v4(),
                        session_id,
                        pane_id: window.pane_id,
                        pane_provider: providers
                            .get(&window.pane_id)
                            .copied()
                            .unwrap_or(Provider::Claude),
                        window_start: window.window_start,
                        window_end: window.window_end,
                        source_digest: window.source_digest.clone(),
                        stage: PaneWorkSummaryStage::Notes,
                        chunk_index: Some(index as u32),
                        chunk_count: Some(chunk_count),
                        content,
                        correction_attempt: false,
                    },
                    window_kind: kind,
                    source_message_count: window.records.len() as u32,
                    source_through: window.source_through,
                    source_through_id: window.source_through_id.clone(),
                    attempt: 1,
                })
                .collect::<Vec<_>>();
            if let Some(base) = chunk_tasks.first().cloned() {
                let mut runtime = self.runtime.lock().await;
                runtime.reductions.insert(
                    reduction_key,
                    ReductionState {
                        base,
                        notes: vec![None; chunk_count as usize],
                    },
                );
            }
            tasks.append(&mut chunk_tasks);
            if let Some(summary) = document.summaries.iter_mut().find(|summary| {
                summary.pane_id == window.pane_id && summary.window_start == window.window_start
            }) {
                summary.status = PaneWorkSummaryStatus::Queued;
                summary.error = None;
                summary.updated_at = Some(now);
            }
        }
        self.storage
            .save_pane_work_summaries(&session_id, &document)
            .await?;
        tracing::debug!(
            %session_id,
            summary_records = document.summaries.len(),
            queued_stages = tasks.len(),
            scanned_bytes,
            "Pane summary cache reconciled"
        );
        for task in tasks {
            self.enqueue(task).await;
        }
        Ok(scanned_bytes)
    }

    async fn enqueue(&self, task: GenerationTask) {
        let key = task.logical_key();
        let mut runtime = self.runtime.lock().await;
        if runtime.logical_jobs.insert(key) {
            runtime.queued.push_back(task);
        }
    }

    async fn enqueue_front(&self, task: GenerationTask) {
        let key = task.logical_key();
        let mut runtime = self.runtime.lock().await;
        if runtime.logical_jobs.insert(key) {
            runtime.queued.push_front(task);
        }
    }

    async fn kick_dispatch(self: &Arc<Self>) {
        loop {
            let reserved = {
                let mut runtime = self.runtime.lock().await;
                if runtime.in_flight.len() >= self.config.global_concurrency.max(1) {
                    None
                } else {
                    let queue_len = runtime.queued.len();
                    let mut selected = None;
                    for _ in 0..queue_len {
                        let Some(task) = runtime.queued.pop_front() else {
                            break;
                        };
                        let session = self.sessions.get_session(&task.job.session_id);
                        let cli_id = session.and_then(|session| session.cli_client_id);
                        let Some(cli_id) = cli_id else {
                            runtime.queued.push_back(task);
                            continue;
                        };
                        if runtime.busy_clis.contains(&cli_id)
                            || !self.sessions.session_supports_capability(
                                &task.job.session_id,
                                PANE_WORK_SUMMARY_CAPABILITY,
                            )
                        {
                            runtime.queued.push_back(task);
                            continue;
                        }
                        let in_flight = InFlightJob {
                            task: task.clone(),
                            cli_id,
                            started_at: Utc::now(),
                        };
                        runtime.busy_clis.insert(cli_id);
                        runtime.in_flight.insert(task.job.job_id, in_flight.clone());
                        selected = Some(in_flight);
                        break;
                    }
                    selected
                }
            };
            let Some(in_flight) = reserved else {
                return;
            };
            let sent = self
                .sessions
                .send_to_cli(
                    &in_flight.cli_id,
                    ServerToCli::GeneratePaneWorkSummary {
                        job: in_flight.task.job.clone(),
                    },
                )
                .await;
            if !sent {
                self.release_and_retry(in_flight.task.job.job_id, "CLI disconnected")
                    .await;
                continue;
            }
            use std::sync::atomic::Ordering;
            self.metrics.dispatched.fetch_add(1, Ordering::Relaxed);
            let _ = self
                .set_record_status(&in_flight.task, PaneWorkSummaryStatus::Generating, None)
                .await;
        }
    }

    pub async fn accept_result(
        self: &Arc<Self>,
        cli_id: Uuid,
        result: PaneWorkSummaryGenerationResult,
    ) -> bool {
        let in_flight = {
            let mut runtime = self.runtime.lock().await;
            let Some(in_flight) = runtime.in_flight.get(&result.job_id).cloned() else {
                tracing::warn!(job_id = %result.job_id, "Ignoring unknown pane summary result");
                return false;
            };
            let job = &in_flight.task.job;
            let matches = in_flight.cli_id == cli_id
                && result.protocol_version == PANE_WORK_SUMMARY_PROTOCOL_VERSION
                && result.session_id == job.session_id
                && result.pane_id == job.pane_id
                && result.window_start == job.window_start
                && result.source_digest == job.source_digest
                && result.stage == job.stage
                && result.chunk_index == job.chunk_index;
            if !matches {
                tracing::warn!(job_id = %result.job_id, %cli_id, "Rejecting mismatched pane summary result");
                return false;
            }
            runtime.in_flight.remove(&result.job_id);
            runtime.busy_clis.remove(&cli_id);
            runtime.logical_jobs.remove(&in_flight.task.logical_key());
            in_flight
        };
        tracing::info!(
            job_id = %result.job_id,
            session_id = %result.session_id,
            pane_id = result.pane_id,
            stage = ?result.stage,
            latency_ms = Utc::now().signed_duration_since(in_flight.started_at).num_milliseconds(),
            provider = result.provider.as_deref().unwrap_or("unknown"),
            model = result.model.as_deref().unwrap_or("unknown"),
            "Pane summary result received"
        );
        match result.kind {
            PaneWorkSummaryResultKind::Success => {
                if self
                    .accept_success(in_flight.task.clone(), result)
                    .await
                    .is_err()
                {
                    self.mark_terminal_failure(&in_flight.task, "Invalid summary provider output")
                        .await;
                }
            }
            PaneWorkSummaryResultKind::Unavailable => {
                self.availability.insert(
                    result.session_id,
                    PaneWorkSummaryAvailability::SummarizerUnavailable,
                );
                use std::sync::atomic::Ordering;
                self.metrics.unavailable.fetch_add(1, Ordering::Relaxed);
                self.mark_terminal_failure(
                    &in_flight.task,
                    result.error.as_deref().unwrap_or("Summarizer unavailable"),
                )
                .await;
            }
            PaneWorkSummaryResultKind::RetryableFailure => {
                self.retry_or_fail(
                    in_flight.task,
                    result
                        .error
                        .as_deref()
                        .unwrap_or("Transient provider failure"),
                )
                .await;
            }
            PaneWorkSummaryResultKind::PermanentFailure => {
                self.mark_terminal_failure(
                    &in_flight.task,
                    result
                        .error
                        .as_deref()
                        .unwrap_or("Provider rejected summary job"),
                )
                .await;
            }
        }
        self.kick_dispatch().await;
        true
    }

    async fn accept_success(
        self: &Arc<Self>,
        task: GenerationTask,
        result: PaneWorkSummaryGenerationResult,
    ) -> anyhow::Result<()> {
        let output = result
            .output
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing output"))?;
        match task.job.stage {
            PaneWorkSummaryStage::Notes => {
                anyhow::ensure!(output.len() <= 4 * 1024, "notes output too large");
                anyhow::ensure!(!output.chars().any(forbidden_control), "control character");
                let key = (
                    task.job.session_id,
                    task.job.pane_id,
                    task.job.window_start,
                    task.job.source_digest.clone(),
                );
                let maybe_final = {
                    let mut runtime = self.runtime.lock().await;
                    let reduction = runtime
                        .reductions
                        .get_mut(&key)
                        .ok_or_else(|| anyhow::anyhow!("missing reduction state"))?;
                    let index = task.job.chunk_index.unwrap_or_default() as usize;
                    anyhow::ensure!(index < reduction.notes.len(), "bad chunk index");
                    reduction.notes[index] = Some(normalize_whitespace(output));
                    if reduction.notes.iter().all(Option::is_some) {
                        let notes = reduction
                            .notes
                            .iter()
                            .enumerate()
                            .map(|(index, note)| {
                                format!("CHUNK {}: {}", index + 1, note.as_deref().unwrap())
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let mut final_task = reduction.base.clone();
                        final_task.job.job_id = Uuid::new_v4();
                        final_task.job.stage = PaneWorkSummaryStage::Final;
                        final_task.job.chunk_index = None;
                        final_task.job.content = notes;
                        final_task.attempt = 1;
                        runtime.reductions.remove(&key);
                        Some(final_task)
                    } else {
                        None
                    }
                };
                if let Some(final_task) = maybe_final {
                    // Finish the newest window before spending quota on older
                    // intermediate chunks. Otherwise every window can appear
                    // to be generating while all final reductions sit at the
                    // back of a long backfill queue.
                    self.enqueue_front(final_task).await;
                }
            }
            PaneWorkSummaryStage::Final => {
                match validate_final_output(output, task.source_message_count) {
                    Ok(summary_text) => {
                        self.complete_record(&task, summary_text, &result).await?;
                    }
                    Err(error) if !task.job.correction_attempt => {
                        let mut correction = task.clone();
                        correction.job.job_id = Uuid::new_v4();
                        correction.job.correction_attempt = true;
                        correction.job.content = format!(
                            "The prior response was invalid ({error}). Return only the required JSON object.\n\n{}",
                            correction.job.content
                        );
                        self.enqueue_front(correction).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }

    async fn complete_record(
        &self,
        task: &GenerationTask,
        summary_text: String,
        result: &PaneWorkSummaryGenerationResult,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        let status = if task.window_kind == PaneWorkSummaryWindowKind::Current {
            PaneWorkSummaryStatus::Partial
        } else {
            PaneWorkSummaryStatus::Complete
        };
        let document = self
            .storage
            .update_pane_work_summaries(&task.job.session_id, |document| {
                let record = matching_record_mut(document, task)?;
                record.status = status;
                record.window_kind = task.window_kind;
                record.summary = Some(summary_text);
                record.source_through = Some(task.source_through);
                record.source_through_id = Some(task.source_through_id.clone());
                record.generated_at = Some(now);
                record.updated_at = Some(now);
                record.provider = result.provider.clone();
                record.model = result.model.clone();
                record.attempts = task.attempt;
                record.error = None;
                Ok(())
            })
            .await?;
        self.broadcast_record(&document, task).await;
        Ok(())
    }

    async fn set_record_status(
        &self,
        task: &GenerationTask,
        status: PaneWorkSummaryStatus,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let document = self
            .storage
            .update_pane_work_summaries(&task.job.session_id, |document| {
                let record = matching_record_mut(document, task)?;
                record.status = status;
                record.attempts = task.attempt;
                record.error = error.map(|error| clip_utf8(error, 512));
                record.updated_at = Some(Utc::now());
                Ok(())
            })
            .await?;
        self.broadcast_record(&document, task).await;
        Ok(())
    }

    async fn broadcast_record(&self, document: &PaneWorkSummaryDocument, task: &GenerationTask) {
        let Some(summary) = document
            .summaries
            .iter()
            .find(|summary| {
                summary.pane_id == task.job.pane_id && summary.window_start == task.job.window_start
            })
            .cloned()
        else {
            return;
        };
        let web_ids = self
            .sessions
            .get_session(&task.job.session_id)
            .map(|session| session.web_connection_ids)
            .unwrap_or_default();
        for web_id in web_ids {
            let Some(user_id) = self.sessions.get_web_user(&web_id) else {
                continue;
            };
            if self
                .db
                .check_session_access(&task.job.session_id.to_string(), &user_id.to_string())
                .await
                .unwrap_or(false)
            {
                let _ = self
                    .sessions
                    .send_to_web(
                        &web_id,
                        ServerToWeb::PaneWorkSummaryUpdated {
                            session_id: task.job.session_id,
                            pane_id: task.job.pane_id,
                            summary: summary.clone(),
                            availability: self.availability_for(task.job.session_id),
                        },
                    )
                    .await;
            }
        }
    }

    async fn broadcast_pane_snapshot(&self, session_id: Uuid, pane_id: u32) -> anyhow::Result<()> {
        let (summaries, availability) = self.list_cached(session_id, pane_id).await?;
        let web_ids = self
            .sessions
            .get_session(&session_id)
            .map(|session| session.web_connection_ids)
            .unwrap_or_default();
        for web_id in web_ids {
            let Some(user_id) = self.sessions.get_web_user(&web_id) else {
                continue;
            };
            if self
                .db
                .check_session_access(&session_id.to_string(), &user_id.to_string())
                .await
                .unwrap_or(false)
            {
                let _ = self
                    .sessions
                    .send_to_web(
                        &web_id,
                        ServerToWeb::PaneWorkSummaries {
                            session_id,
                            pane_id,
                            summaries: summaries.clone(),
                            availability,
                        },
                    )
                    .await;
            }
        }
        Ok(())
    }

    pub async fn sweep_timeouts(self: &Arc<Self>) {
        let cutoff = Utc::now() - Duration::seconds(self.config.job_timeout_seconds.max(1) as i64);
        let expired = {
            let runtime = self.runtime.lock().await;
            runtime
                .in_flight
                .iter()
                .filter(|(_, job)| job.started_at <= cutoff)
                .map(|(job_id, _)| *job_id)
                .collect::<Vec<_>>()
        };
        for job_id in expired {
            self.release_and_retry(job_id, "Summary generation timed out")
                .await;
        }
        self.kick_dispatch().await;
    }

    async fn release_and_retry(&self, job_id: Uuid, error: &str) {
        let task = {
            let mut runtime = self.runtime.lock().await;
            let Some(in_flight) = runtime.in_flight.remove(&job_id) else {
                return;
            };
            runtime.busy_clis.remove(&in_flight.cli_id);
            runtime.logical_jobs.remove(&in_flight.task.logical_key());
            in_flight.task
        };
        self.retry_or_fail(task, error).await;
    }

    async fn retry_or_fail(&self, mut task: GenerationTask, error: &str) {
        if task.attempt < self.config.max_attempts.max(1) {
            task.attempt += 1;
            task.job.job_id = Uuid::new_v4();
            use std::sync::atomic::Ordering;
            self.metrics.retries.fetch_add(1, Ordering::Relaxed);
            let delay = 1_u64 << (task.attempt - 2).min(5);
            let _ = self
                .set_record_status(&task, PaneWorkSummaryStatus::Queued, Some(error))
                .await;
            let mut runtime = self.runtime.lock().await;
            let key = task.logical_key();
            if runtime.logical_jobs.insert(key) {
                runtime.queued.push_back(task);
            }
            drop(runtime);
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        } else {
            self.mark_terminal_failure(&task, error).await;
        }
    }

    async fn mark_terminal_failure(&self, task: &GenerationTask, error: &str) {
        use std::sync::atomic::Ordering;
        self.metrics.failures.fetch_add(1, Ordering::Relaxed);
        let _ = self
            .set_record_status(task, PaneWorkSummaryStatus::Failed, Some(error))
            .await;
    }
}

fn matching_record_mut<'a>(
    document: &'a mut PaneWorkSummaryDocument,
    task: &GenerationTask,
) -> anyhow::Result<&'a mut PaneWorkSummary> {
    let record = document
        .summaries
        .iter_mut()
        .find(|summary| {
            summary.pane_id == task.job.pane_id && summary.window_start == task.job.window_start
        })
        .ok_or_else(|| anyhow::anyhow!("summary cache record missing"))?;
    anyhow::ensure!(
        record.source_digest == task.job.source_digest,
        "summary source digest changed"
    );
    Ok(record)
}

fn forbidden_control(character: char) -> bool {
    character.is_control() && !character.is_whitespace()
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_final_output(output: &str, source_message_count: u32) -> anyhow::Result<String> {
    anyhow::ensure!(output.len() <= 2 * 1024, "final output too large");
    anyhow::ensure!(!output.chars().any(forbidden_control), "control character");
    let value: serde_json::Value = serde_json::from_str(output)?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("output is not an object"))?;
    anyhow::ensure!(object.len() == 1, "unexpected output fields");
    let summary = object
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing summary string"))?;
    let summary = normalize_whitespace(summary);
    anyhow::ensure!(!summary.is_empty(), "empty summary");
    anyhow::ensure!(
        !summary.contains('<') && !summary.contains('>'),
        "markup is not permitted"
    );
    let words = summary.split_whitespace().count();
    anyhow::ensure!(words <= 100, "summary exceeds 100 words");
    if source_message_count >= 4 {
        anyhow::ensure!(words >= 50, "summary shorter than 50 words");
    }
    Ok(summary)
}

fn needs_completed_replacement(
    existing: &PaneWorkSummary,
    next_kind: PaneWorkSummaryWindowKind,
) -> bool {
    next_kind == PaneWorkSummaryWindowKind::Completed
        && (existing.window_kind == PaneWorkSummaryWindowKind::Current
            || existing.status == PaneWorkSummaryStatus::Partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, pane: Option<&str>, timestamp: &str, content: &str) -> StoredMessage {
        StoredMessage {
            id: id.to_string(),
            role: "assistant".to_string(),
            content: content.to_string(),
            message_type: "text".to_string(),
            created_at: timestamp.to_string(),
            pane_type: pane.map(str::to_string),
        }
    }

    #[test]
    fn utc_windows_are_fixed_and_non_overlapping() {
        let timestamp = "2026-08-11T05:59:59Z".parse().unwrap();
        let (start, end) = window_bounds(timestamp);
        assert_eq!(start.to_rfc3339(), "2026-08-11T03:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-08-11T06:00:00+00:00");
        assert_eq!(window_bounds(end).0, end);
    }

    #[test]
    fn pane_normalization_and_windows_never_mix_siblings() {
        let session_id = Uuid::new_v4();
        let messages = vec![
            message("b", Some("pane-8"), "2026-08-11T04:10:00Z", "pane eight"),
            message("a", Some("7"), "2026-08-11T04:00:00Z", "pane seven"),
            message("legacy", None, "2026-08-11T04:20:00Z", "fallback"),
        ];
        let windows = build_source_windows(session_id, &messages, Some(7));
        assert_eq!(windows.len(), 2);
        let seven = windows.iter().find(|window| window.pane_id == 7).unwrap();
        assert_eq!(seven.records.len(), 2);
        assert!(!seven.canonical_source().contains("pane eight"));
        let eight = windows.iter().find(|window| window.pane_id == 8).unwrap();
        assert_eq!(eight.records.len(), 1);
    }

    #[test]
    fn canonical_source_redacts_secrets_and_excludes_pty() {
        let session_id = Uuid::new_v4();
        let mut secret = message(
            "secret",
            Some("2"),
            "2026-08-11T04:00:00Z",
            "Authorization: Bearer abcdefghijklmnop and token=supersecret",
        );
        let mut pty = message("pty", Some("2"), "2026-08-11T04:01:00Z", "raw terminal");
        pty.message_type = "terminal_output".to_string();
        let windows = build_source_windows(session_id, &[secret.clone(), pty], None);
        let source = windows[0].canonical_source();
        assert!(!source.contains("abcdefghijklmnop"));
        assert!(!source.contains("supersecret"));
        assert!(!source.contains("raw terminal"));
        secret.content = "safe".to_string();
        let changed = build_source_windows(session_id, &[secret], None);
        assert!(cached_digest_is_stale(
            &windows[0].source_digest,
            &changed[0]
        ));
    }

    #[test]
    fn chunking_respects_message_boundaries_and_reports_overflow() {
        let session_id = Uuid::new_v4();
        let messages = (0..8)
            .map(|index| {
                message(
                    &format!("m{index}"),
                    Some("2"),
                    &format!("2026-08-11T04:{index:02}:00Z"),
                    &"x".repeat(100),
                )
            })
            .collect::<Vec<_>>();
        let window = build_source_windows(session_id, &messages, None).remove(0);
        let manifest = chunk_window(&window, 180, 2);
        assert!(manifest.overflowed);
        assert_eq!(manifest.chunks.len(), 2);
        assert!(manifest.chunks.last().unwrap().contains("m7"));
    }

    #[test]
    fn canonical_digest_is_stable_across_input_order() {
        let session_id = Uuid::nil();
        let a = message("a", Some("2"), "2026-08-11T04:00:00Z", "first");
        let b = message("b", Some("2"), "2026-08-11T04:01:00Z", "second");
        let first = build_source_windows(session_id, &[a.clone(), b.clone()], None);
        let second = build_source_windows(session_id, &[b, a], None);
        assert_eq!(first[0].source_digest, second[0].source_digest);
    }

    #[test]
    fn a_partial_current_record_is_replaced_when_its_window_closes() {
        let session_id = Uuid::new_v4();
        let start = Utc::now() - Duration::hours(3);
        let record = PaneWorkSummary {
            protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            session_id,
            pane_id: 2,
            window_start: start,
            window_end: start + Duration::hours(3),
            window_kind: PaneWorkSummaryWindowKind::Current,
            status: PaneWorkSummaryStatus::Partial,
            summary: Some("partial".to_string()),
            source_digest: "digest".to_string(),
            source_message_count: 1,
            source_through: Some(start),
            source_through_id: Some("m1".to_string()),
            generated_at: Some(start),
            updated_at: Some(start),
            provider: Some("claude".to_string()),
            model: None,
            attempts: 1,
            error: None,
        };
        assert!(needs_completed_replacement(
            &record,
            PaneWorkSummaryWindowKind::Completed
        ));
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use crate::db::{Session, User};
    use tokio::sync::mpsc;

    fn service_message(
        id: &str,
        pane: Option<&str>,
        timestamp: &str,
        content: &str,
    ) -> StoredMessage {
        StoredMessage {
            id: id.to_string(),
            role: "assistant".to_string(),
            content: content.to_string(),
            message_type: "text".to_string(),
            created_at: timestamp.to_string(),
            pane_type: pane.map(str::to_string),
        }
    }

    async fn test_service() -> (
        Arc<PaneWorkSummaryService>,
        Database,
        FileStorage,
        Arc<SessionManager>,
    ) {
        let root = std::env::temp_dir().join(format!("apas-summary-service-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db_path = root.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        let storage = FileStorage::new(&root);
        let sessions = Arc::new(SessionManager::new());
        let service = Arc::new(PaneWorkSummaryService::new(
            db.clone(),
            sessions.clone(),
            storage.clone(),
            SummaryConfig::default(),
        ));
        (service, db, storage, sessions)
    }

    async fn add_user_and_session(db: &Database, user_id: Uuid, session_id: Uuid) {
        db.create_user(&User {
            id: user_id.to_string(),
            email: format!("{user_id}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.create_session(&Session {
            id: session_id.to_string(),
            user_id: user_id.to_string(),
            cli_client_id: None,
            working_dir: Some("/tmp/project".to_string()),
            hostname: Some("test-host".to_string()),
            status: "connected".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(session_id.to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
    }

    fn result_for(
        job: &PaneWorkSummaryGenerationJob,
        output: &str,
    ) -> PaneWorkSummaryGenerationResult {
        PaneWorkSummaryGenerationResult {
            protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            job_id: job.job_id,
            session_id: job.session_id,
            pane_id: job.pane_id,
            window_start: job.window_start,
            source_digest: job.source_digest.clone(),
            stage: job.stage,
            chunk_index: job.chunk_index,
            kind: PaneWorkSummaryResultKind::Success,
            output: Some(output.to_string()),
            error: None,
            provider: Some("claude".to_string()),
            model: Some("test-model".to_string()),
        }
    }

    fn cached_summary(
        window: &SourceWindow,
        window_kind: PaneWorkSummaryWindowKind,
        status: PaneWorkSummaryStatus,
        summary: Option<&str>,
    ) -> PaneWorkSummary {
        PaneWorkSummary {
            protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            session_id: window.session_id,
            pane_id: window.pane_id,
            window_start: window.window_start,
            window_end: window.window_end,
            window_kind,
            status,
            summary: summary.map(str::to_string),
            source_digest: window.source_digest.clone(),
            source_message_count: window.records.len() as u32,
            source_through: Some(window.source_through),
            source_through_id: Some(window.source_through_id.clone()),
            generated_at: summary.map(|_| Utc::now()),
            updated_at: Some(Utc::now()),
            provider: summary.map(|_| "codex".to_string()),
            model: None,
            attempts: u32::from(summary.is_some()),
            error: (status == PaneWorkSummaryStatus::Failed)
                .then(|| "Previous generation failed".to_string()),
        }
    }

    fn register_capable_cli(
        sessions: &SessionManager,
        owner: Uuid,
        session_id: Uuid,
    ) -> (Uuid, mpsc::Receiver<ServerToCli>) {
        let cli_id = Uuid::new_v4();
        let (cli_tx, cli_rx) = mpsc::channel(16);
        sessions.register_cli(cli_id, owner, cli_tx, Some("test".to_string()));
        sessions.set_cli_capabilities(cli_id, vec![PANE_WORK_SUMMARY_CAPABILITY.to_string()]);
        sessions.create_cli_session(
            session_id,
            cli_id,
            Some("/tmp/project".to_string()),
            Some("test-host".to_string()),
        );
        (cli_id, cli_rx)
    }

    #[tokio::test]
    async fn a_window_cut_short_by_the_bounded_read_is_never_summarised() {
        // The reconciler reads only the tail of a session's log, so the oldest
        // window it can see may be missing its beginning. The danger is not the
        // huge window — that fails the size check anyway — but the *small*
        // surviving fragment of one, which looks perfectly summarisable and
        // would be described as if it were the whole window.
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (_cli_id, _cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let budget = service.source_tail_budget();
        let filler = "x".repeat(4 * 1024);
        let old_start = window_bounds(Utc::now()).0 - Duration::hours(SUMMARY_WINDOW_HOURS * 2);
        let newer_start = window_bounds(Utc::now()).0 - Duration::hours(SUMMARY_WINDOW_HOURS);
        // Count exactly what lands on disk — the cut position is what this
        // fixture is really controlling, and an estimate drifts enough to move
        // it outside the older window.
        let append = |message: StoredMessage| {
            let bytes = serde_json::to_string(&message).unwrap().len() as u64 + 1;
            (message, bytes)
        };

        // The window that will be cut into. Sized so that whatever survives the
        // cut stays under max_source_bytes — otherwise the existing size check
        // would reject it and this test would prove nothing.
        let mut written = 0u64;
        for i in 0..10 {
            let (message, bytes) = append(service_message(
                &format!("old{i}"),
                Some("7"),
                &(old_start + Duration::seconds(i)).to_rfc3339(),
                &filler,
            ));
            storage.append_message(&session_id, &message).await.unwrap();
            written += bytes;
        }
        // Overshoot the budget by less than the older window's size, so the cut
        // lands inside that window instead of past it.
        let overshoot = 25 * 1024;
        let mut i = 0i64;
        while written < budget + overshoot {
            let (message, bytes) = append(service_message(
                &format!("new{i}"),
                Some("7"),
                &(newer_start + Duration::seconds(i % 3000)).to_rfc3339(),
                &filler,
            ));
            storage.append_message(&session_id, &message).await.unwrap();
            written += bytes;
            i += 1;
        }

        let (tail, truncated) = storage
            .get_messages_tail(&session_id, budget)
            .await
            .unwrap();
        assert!(truncated, "fixture did not exceed the read budget");
        assert!(
            tail.iter().any(|m| m.id.starts_with("old")),
            "fixture cut away the older window entirely; nothing to assert on"
        );

        service
            .reconcile_completed_for_pane(session_id, 7)
            .await
            .unwrap();

        let document = storage
            .load_pane_work_summaries(&session_id)
            .await
            .unwrap();
        let partial = document
            .summaries
            .iter()
            .find(|summary| summary.window_start == old_start)
            .expect("the partially-read window should be recorded");
        assert_eq!(
            partial.status,
            PaneWorkSummaryStatus::Failed,
            "a window missing its start was accepted for summarisation"
        );
        assert!(
            partial
                .error
                .as_deref()
                .is_some_and(|e| e.contains("exceeds configured bounds")),
            "unexpected rejection reason: {:?}",
            partial.error
        );
    }

    #[tokio::test]
    async fn first_reconcile_rebuilds_the_process_local_queue_after_restart() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (_cli_id, mut cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let window_start = window_bounds(Utc::now()).0 - Duration::hours(SUMMARY_WINDOW_HOURS);
        let message = service_message(
            "interrupted",
            Some("7"),
            &(window_start + Duration::minutes(1)).to_rfc3339(),
            "This summary was interrupted by a server restart.",
        );
        storage.append_message(&session_id, &message).await.unwrap();
        let windows = build_source_windows(session_id, &[message], None);
        storage
            .save_pane_work_summaries(
                &session_id,
                &PaneWorkSummaryDocument {
                    version: 1,
                    summaries: vec![cached_summary(
                        &windows[0],
                        PaneWorkSummaryWindowKind::Completed,
                        PaneWorkSummaryStatus::Generating,
                        None,
                    )],
                },
            )
            .await
            .unwrap();

        service
            .reconcile_completed_for_pane(session_id, 7)
            .await
            .unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job } = cli_rx.recv().await.unwrap() else {
            panic!("expected recovered notes job")
        };
        assert_eq!(job.window_start, window_start);
        assert_eq!(job.stage, PaneWorkSummaryStage::Notes);
    }

    #[tokio::test]
    async fn listing_returns_the_durable_cache_without_reconciling_changed_source() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (_cli_id, mut cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let window_start = window_bounds(Utc::now()).0 - Duration::hours(SUMMARY_WINDOW_HOURS);
        let first = service_message(
            "first",
            Some("7"),
            &(window_start + Duration::minutes(1)).to_rfc3339(),
            "Implemented the original behavior.",
        );
        storage.append_message(&session_id, &first).await.unwrap();
        let window = build_source_windows(session_id, &[first], None)
            .into_iter()
            .next()
            .unwrap();
        storage
            .save_pane_work_summaries(
                &session_id,
                &PaneWorkSummaryDocument {
                    version: 1,
                    summaries: vec![cached_summary(
                        &window,
                        PaneWorkSummaryWindowKind::Completed,
                        PaneWorkSummaryStatus::Complete,
                        Some("Saved completed summary."),
                    )],
                },
            )
            .await
            .unwrap();
        storage
            .append_message(
                &session_id,
                &service_message(
                    "late",
                    Some("7"),
                    &(window_start + Duration::minutes(2)).to_rfc3339(),
                    "Late activity changed the retained source.",
                ),
            )
            .await
            .unwrap();

        let (summaries, availability) = service.list_for_pane(session_id, 7).await.unwrap();
        assert_eq!(availability, PaneWorkSummaryAvailability::Available);
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].summary.as_deref(),
            Some("Saved completed summary.")
        );
        assert_eq!(summaries[0].status, PaneWorkSummaryStatus::Complete);
        assert!(cli_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn panel_open_reconciles_only_the_changed_current_window() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (_cli_id, mut cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let now = Utc::now();
        let current_start = window_bounds(now).0;
        let completed_start = current_start - Duration::hours(SUMMARY_WINDOW_HOURS);
        for source in [
            service_message(
                "completed",
                Some("7"),
                &(completed_start + Duration::minutes(1)).to_rfc3339(),
                "This historical window is background-owned.",
            ),
            service_message(
                "current",
                Some("7"),
                &now.to_rfc3339(),
                "This current window is generated on panel open.",
            ),
        ] {
            storage.append_message(&session_id, &source).await.unwrap();
        }

        service
            .reconcile_current_for_pane(session_id, 7)
            .await
            .unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job } = cli_rx.recv().await.unwrap() else {
            panic!("expected current-window summary job")
        };
        assert_eq!(job.window_start, current_start);
        assert!(cli_rx.try_recv().is_err());

        let document = storage.load_pane_work_summaries(&session_id).await.unwrap();
        assert_eq!(document.summaries.len(), 1);
        assert_eq!(document.summaries[0].window_start, current_start);

        service
            .reconcile_current_for_pane(session_id, 7)
            .await
            .unwrap();
        assert!(cli_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn toolbar_refresh_requeues_only_the_current_window() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (_cli_id, mut cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let current_time = Utc::now();
        let current_start = window_bounds(current_time).0;
        let completed_start = current_start - Duration::hours(SUMMARY_WINDOW_HOURS);
        let messages = vec![
            service_message(
                "completed",
                Some("7"),
                &(completed_start + Duration::minutes(1)).to_rfc3339(),
                "Completed and verified the previous window's work.",
            ),
            service_message(
                "current",
                Some("7"),
                &current_time.to_rfc3339(),
                "Continued implementation in the current window.",
            ),
        ];
        for source in &messages {
            storage.append_message(&session_id, source).await.unwrap();
        }
        let windows = build_source_windows(session_id, &messages, None);
        let completed = windows
            .iter()
            .find(|window| window.window_start == completed_start)
            .unwrap();
        let current = windows
            .iter()
            .find(|window| window.window_start == current_start)
            .unwrap();
        storage
            .save_pane_work_summaries(
                &session_id,
                &PaneWorkSummaryDocument {
                    version: 1,
                    summaries: vec![
                        cached_summary(
                            completed,
                            PaneWorkSummaryWindowKind::Completed,
                            PaneWorkSummaryStatus::Complete,
                            Some("Keep this completed summary."),
                        ),
                        cached_summary(
                            current,
                            PaneWorkSummaryWindowKind::Current,
                            PaneWorkSummaryStatus::Partial,
                            Some("Refresh only this partial summary."),
                        ),
                    ],
                },
            )
            .await
            .unwrap();

        service.refresh(session_id, 7, None).await.unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job } = cli_rx.recv().await.unwrap() else {
            panic!("expected current-window notes job")
        };
        assert_eq!(job.window_start, current_start);
        assert!(cli_rx.try_recv().is_err());

        let document = storage.load_pane_work_summaries(&session_id).await.unwrap();
        let preserved = document
            .summaries
            .iter()
            .find(|summary| summary.window_start == completed_start)
            .unwrap();
        assert_eq!(preserved.status, PaneWorkSummaryStatus::Complete);
        assert_eq!(
            preserved.summary.as_deref(),
            Some("Keep this completed summary.")
        );
    }

    #[tokio::test]
    async fn retry_requeues_only_the_selected_failed_window() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (_cli_id, mut cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let newest_start = window_bounds(Utc::now()).0 - Duration::hours(SUMMARY_WINDOW_HOURS);
        let failed_start = newest_start - Duration::hours(SUMMARY_WINDOW_HOURS);
        let messages = vec![
            service_message(
                "failed",
                Some("7"),
                &(failed_start + Duration::minutes(1)).to_rfc3339(),
                "The older summary generation failed.",
            ),
            service_message(
                "complete",
                Some("7"),
                &(newest_start + Duration::minutes(1)).to_rfc3339(),
                "The newer window already has a valid summary.",
            ),
        ];
        for source in &messages {
            storage.append_message(&session_id, source).await.unwrap();
        }
        let windows = build_source_windows(session_id, &messages, None);
        let failed = windows
            .iter()
            .find(|window| window.window_start == failed_start)
            .unwrap();
        let complete = windows
            .iter()
            .find(|window| window.window_start == newest_start)
            .unwrap();
        storage
            .save_pane_work_summaries(
                &session_id,
                &PaneWorkSummaryDocument {
                    version: 1,
                    summaries: vec![
                        cached_summary(
                            failed,
                            PaneWorkSummaryWindowKind::Completed,
                            PaneWorkSummaryStatus::Failed,
                            None,
                        ),
                        cached_summary(
                            complete,
                            PaneWorkSummaryWindowKind::Completed,
                            PaneWorkSummaryStatus::Complete,
                            Some("Preserve the newer completed summary."),
                        ),
                    ],
                },
            )
            .await
            .unwrap();

        service
            .refresh(session_id, 7, Some(failed_start))
            .await
            .unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job } = cli_rx.recv().await.unwrap() else {
            panic!("expected selected-window retry job")
        };
        assert_eq!(job.window_start, failed_start);
        assert!(cli_rx.try_recv().is_err());

        let document = storage.load_pane_work_summaries(&session_id).await.unwrap();
        let preserved = document
            .summaries
            .iter()
            .find(|summary| summary.window_start == newest_start)
            .unwrap();
        assert_eq!(preserved.status, PaneWorkSummaryStatus::Complete);
        assert_eq!(
            preserved.summary.as_deref(),
            Some("Preserve the newer completed summary.")
        );
    }

    #[tokio::test]
    async fn final_reduction_is_dispatched_before_older_window_notes() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let (cli_id, mut cli_rx) = register_capable_cli(&sessions, owner, session_id);

        let newest_start = window_bounds(Utc::now()).0 - Duration::hours(SUMMARY_WINDOW_HOURS);
        let older_start = newest_start - Duration::hours(SUMMARY_WINDOW_HOURS);
        for source in [
            service_message(
                "older",
                Some("7"),
                &(older_start + Duration::minutes(1)).to_rfc3339(),
                "Worked on an older task.",
            ),
            service_message(
                "newest",
                Some("7"),
                &(newest_start + Duration::minutes(1)).to_rfc3339(),
                "Worked on the newest task.",
            ),
        ] {
            storage.append_message(&session_id, &source).await.unwrap();
        }

        service
            .reconcile_completed_for_pane(session_id, 7)
            .await
            .unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job: notes_job } = cli_rx.recv().await.unwrap()
        else {
            panic!("expected newest notes job")
        };
        assert_eq!(notes_job.window_start, newest_start);
        assert_eq!(notes_job.stage, PaneWorkSummaryStage::Notes);
        assert!(
            service
                .accept_result(cli_id, result_for(&notes_job, "Newest grounded facts"))
                .await
        );

        let ServerToCli::GeneratePaneWorkSummary { job: next_job } = cli_rx.recv().await.unwrap()
        else {
            panic!("expected newest final job")
        };
        assert_eq!(next_job.window_start, newest_start);
        assert_eq!(next_job.stage, PaneWorkSummaryStage::Final);
    }

    #[tokio::test]
    async fn fake_capable_cli_runs_stages_persists_and_broadcasts_only_to_authorized_user() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let outsider = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        db.create_user(&User {
            id: outsider.to_string(),
            email: format!("{outsider}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();

        let cli_id = Uuid::new_v4();
        let (cli_tx, mut cli_rx) = mpsc::channel(8);
        sessions.register_cli(cli_id, owner, cli_tx, Some("test".to_string()));
        sessions.set_cli_capabilities(cli_id, vec![PANE_WORK_SUMMARY_CAPABILITY.to_string()]);
        sessions.create_cli_session(
            session_id,
            cli_id,
            Some("/tmp/project".to_string()),
            Some("test-host".to_string()),
        );

        let owner_web = Uuid::new_v4();
        let outsider_web = Uuid::new_v4();
        let (owner_tx, mut owner_rx) = mpsc::channel(16);
        let (outsider_tx, mut outsider_rx) = mpsc::channel(16);
        sessions.register_web(owner_web, owner_tx);
        sessions.set_web_user(owner_web, owner);
        sessions.register_web(outsider_web, outsider_tx);
        sessions.set_web_user(outsider_web, outsider);
        assert!(sessions.attach_web_to_session(&session_id, owner_web, Some(cli_id)));
        assert!(sessions.attach_web_to_session(&session_id, outsider_web, Some(cli_id)));

        let source_time = Utc::now() - Duration::hours(4);
        storage
            .append_message(
                &session_id,
                &StoredMessage {
                    id: "source-1".to_string(),
                    role: "assistant".to_string(),
                    content:
                        "Implemented the requested storage behavior and ran its focused tests."
                            .to_string(),
                    message_type: "text".to_string(),
                    created_at: source_time.to_rfc3339(),
                    pane_type: Some("7".to_string()),
                },
            )
            .await
            .unwrap();

        let (_, availability) = service.list_for_pane(session_id, 7).await.unwrap();
        assert_eq!(availability, PaneWorkSummaryAvailability::Available);
        service
            .reconcile_completed_for_pane(session_id, 7)
            .await
            .unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job: notes_job } = cli_rx.recv().await.unwrap()
        else {
            panic!("expected notes job")
        };
        assert_eq!(notes_job.stage, PaneWorkSummaryStage::Notes);

        let mut mismatched = result_for(&notes_job, "grounded facts");
        mismatched.source_digest = "wrong".to_string();
        assert!(!service.accept_result(cli_id, mismatched).await);
        assert!(
            service
                .accept_result(cli_id, result_for(&notes_job, "grounded facts"))
                .await
        );

        let ServerToCli::GeneratePaneWorkSummary { job: final_job } = cli_rx.recv().await.unwrap()
        else {
            panic!("expected final job")
        };
        assert_eq!(final_job.stage, PaneWorkSummaryStage::Final);
        assert!(service
            .accept_result(
                cli_id,
                result_for(
                    &final_job,
                    r#"{"summary":"Implemented the requested behavior and verified it with focused tests."}"#,
                ),
            )
            .await);

        let document = storage.load_pane_work_summaries(&session_id).await.unwrap();
        assert_eq!(document.summaries.len(), 1);
        assert_eq!(
            document.summaries[0].status,
            PaneWorkSummaryStatus::Complete
        );
        assert_eq!(document.summaries[0].provider.as_deref(), Some("claude"));

        let mut saw_complete = false;
        while let Ok(message) = owner_rx.try_recv() {
            if matches!(
                message,
                ServerToWeb::PaneWorkSummaryUpdated {
                    summary: PaneWorkSummary {
                        status: PaneWorkSummaryStatus::Complete,
                        ..
                    },
                    ..
                }
            ) {
                saw_complete = true;
            }
        }
        assert!(saw_complete);
        assert!(outsider_rx.try_recv().is_err());

        storage
            .append_message(
                &session_id,
                &StoredMessage {
                    id: "source-late".to_string(),
                    role: "assistant".to_string(),
                    content: "A late retained result changed the completed window.".to_string(),
                    message_type: "text".to_string(),
                    created_at: (source_time + Duration::minutes(1)).to_rfc3339(),
                    pane_type: Some("7".to_string()),
                },
            )
            .await
            .unwrap();
        service
            .reconcile_completed_for_pane(session_id, 7)
            .await
            .unwrap();
        let ServerToCli::GeneratePaneWorkSummary { job: refreshed_job } =
            cli_rx.recv().await.unwrap()
        else {
            panic!("expected regeneration job")
        };
        assert_ne!(refreshed_job.source_digest, notes_job.source_digest);
        let refreshed = storage.load_pane_work_summaries(&session_id).await.unwrap();
        assert_eq!(
            refreshed.summaries[0].status,
            PaneWorkSummaryStatus::Generating
        );
    }

    #[tokio::test]
    async fn old_cli_keeps_cached_summaries_readable_without_receiving_jobs() {
        let (service, db, storage, sessions) = test_service().await;
        let owner = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        add_user_and_session(&db, owner, session_id).await;
        let cli_id = Uuid::new_v4();
        let (cli_tx, mut cli_rx) = mpsc::channel(2);
        sessions.register_cli(cli_id, owner, cli_tx, Some("old".to_string()));
        sessions.set_cli_capabilities(cli_id, Vec::new());
        sessions.create_cli_session(session_id, cli_id, None, None);
        let start = Utc::now() - Duration::hours(6);
        storage
            .save_pane_work_summaries(
                &session_id,
                &PaneWorkSummaryDocument {
                    version: 1,
                    summaries: vec![PaneWorkSummary {
                        protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
                        session_id,
                        pane_id: 2,
                        window_start: start,
                        window_end: start + Duration::hours(3),
                        window_kind: PaneWorkSummaryWindowKind::Completed,
                        status: PaneWorkSummaryStatus::Complete,
                        summary: Some("Previously cached summary".to_string()),
                        source_digest: "cached".to_string(),
                        source_message_count: 1,
                        source_through: Some(start),
                        source_through_id: Some("old".to_string()),
                        generated_at: Some(start),
                        updated_at: Some(start),
                        provider: Some("claude".to_string()),
                        model: None,
                        attempts: 1,
                        error: None,
                    }],
                },
            )
            .await
            .unwrap();

        let (summaries, availability) = service.list_for_pane(session_id, 2).await.unwrap();
        assert_eq!(
            summaries[0].summary.as_deref(),
            Some("Previously cached summary")
        );
        assert_eq!(availability, PaneWorkSummaryAvailability::CliUpdateRequired);
        assert!(cli_rx.try_recv().is_err());
    }
}
