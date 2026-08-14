use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

static SESSION_GC_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ceiling on one session's `messages.jsonl`.
///
/// Sized to be unreachable by human conversation and firmly reachable by a
/// runaway writer. The incident that motivated it wrote 2.9 GB into a single
/// session; the server then died trying to read that back into memory. 256 MiB
/// is roughly a third of a million typical messages — orders of magnitude more
/// than any real session — while still leaving the file readable in full.
pub const SESSION_MESSAGES_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Target size after a trim. Deliberately below the cap so an over-cap session
/// does not rewrite its whole log on every subsequent append.
const SESSION_MESSAGES_TRIM_TO_BYTES: u64 = 192 * 1024 * 1024;
const PANE_WORK_SUMMARY_SIDECAR_VERSION: u32 = 1;

/// Truncate `content` to roughly `max_bytes`, preserving JSON validity for
/// `tool_result` envelopes. The web client parses these envelopes
/// (`JSON.parse(content)`) to pull out the human-visible `content` and
/// `is_error` fields; raw-byte truncation would leave invalid JSON and the
/// UI would render the broken string verbatim.
///
/// For tool_result: parse the envelope, truncate the inner `content`
/// string, drop the bulky `tool_use_result` (full before/after for Edit),
/// re-serialize. For everything else: hard-truncate with a marker.
///
/// Shared between storage-layer reads (memory cap) and ws_web transit
/// (wire-frame cap); each caller passes its own `max_bytes` and `reason`.
pub fn truncate_message_content(
    content: String,
    message_type: &str,
    max_bytes: usize,
    reason: &str,
) -> String {
    if content.len() <= max_bytes {
        return content;
    }
    if message_type == "tool_result" {
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(obj) = v.as_object_mut() {
                let original_len = content.len();
                if let Some(inner) = obj.get_mut("content") {
                    if let Some(s) = inner.as_str() {
                        let head: String = s.chars().take(8_192).collect();
                        *inner = serde_json::Value::String(format!(
                            "{head}\n…[truncated for {reason}; full size {original_len} bytes]"
                        ));
                    }
                }
                obj.remove("tool_use_result");
                if let Ok(serialized) = serde_json::to_string(&v) {
                    return serialized;
                }
            }
        }
    }
    if message_type == "tool_use" {
        // Envelope: {"id": "...", "name": "...", "input": <value>}. The web
        // does JSON.parse(content) on initial load; a raw-byte truncation
        // leaves invalid JSON, the parse throws, and the message falls back
        // to plain text — which means tools like AskUserQuestion silently
        // lose their UI card. Preserve envelope validity by trimming only
        // the `input` field.
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(obj) = v.as_object_mut() {
                let original_len = content.len();
                if let Some(input) = obj.get_mut("input") {
                    if let Some(s) = input.as_str() {
                        // String input (e.g. Bash command) — trim to a head.
                        let head: String = s.chars().take(8_192).collect();
                        *input = serde_json::Value::String(format!(
                            "{head}\n…[truncated for {reason}; full size {original_len} bytes]"
                        ));
                    } else {
                        // Structured input (e.g. AskUserQuestion's
                        // {questions: [...]}). Replace wholesale with a
                        // marker; we can't selectively trim object trees
                        // without breaking the tool-specific schema, and
                        // structured inputs above the cap are pathological.
                        *input = serde_json::json!({
                            "_truncated": true,
                            "_original_bytes": original_len,
                            "_reason": reason,
                        });
                    }
                }
                if let Ok(serialized) = serde_json::to_string(&v) {
                    return serialized;
                }
            }
        }
    }
    let original_len = content.len();
    let head: String = content.chars().take(8_192).collect();
    format!("{head}\n…[truncated for {reason}; full size {original_len} bytes]")
}

fn session_gc_temp_path(file_path: &Path) -> PathBuf {
    let sequence = SESSION_GC_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = file_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "messages.jsonl".into());

    file_path.with_file_name(format!(
        "{file_name}.gc.{}.{}.{}.tmp",
        std::process::id(),
        nanos,
        sequence
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub message_type: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaneWorkSummaryDocument {
    #[serde(default = "pane_work_summary_sidecar_version")]
    pub version: u32,
    #[serde(default)]
    pub summaries: Vec<shared::PaneWorkSummary>,
}

impl Default for PaneWorkSummaryDocument {
    fn default() -> Self {
        Self {
            version: PANE_WORK_SUMMARY_SIDECAR_VERSION,
            summaries: Vec::new(),
        }
    }
}

const fn pane_work_summary_sidecar_version() -> u32 {
    PANE_WORK_SUMMARY_SIDECAR_VERSION
}

#[derive(Clone)]
pub struct FileStorage {
    base_path: PathBuf,
    /// Per-session locks shared between message appends and the periodic GC
    /// task. Without this an `append_message` mid-write would race with the
    /// GC's atomic rename — the append handle would land on the orphaned
    /// pre-rename inode and the message would silently vanish.
    session_locks: Arc<StdMutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>>,
    /// Size ceiling for one session's log, and the size a trim leaves behind.
    /// Fields rather than constants so tests can exercise the trim without
    /// writing a quarter of a gigabyte.
    max_session_bytes: u64,
    trim_session_to_bytes: u64,
}

#[derive(Debug, Default, Clone)]
pub struct GcStats {
    pub sessions_scanned: u64,
    pub sessions_modified: u64,
    pub messages_kept: u64,
    pub messages_dropped: u64,
    pub bytes_freed: u64,
}

impl FileStorage {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            session_locks: Arc::new(StdMutex::new(HashMap::new())),
            max_session_bytes: SESSION_MESSAGES_MAX_BYTES,
            trim_session_to_bytes: SESSION_MESSAGES_TRIM_TO_BYTES,
        }
    }

    /// Shrink the per-session size cap. Test-only: the production ceiling is
    /// too large to reach in a unit test.
    #[cfg(test)]
    fn with_session_size_cap(mut self, max_bytes: u64, trim_to_bytes: u64) -> Self {
        self.max_session_bytes = max_bytes;
        self.trim_session_to_bytes = trim_to_bytes;
        self
    }

    fn session_lock(&self, session_id: &Uuid) -> Arc<AsyncMutex<()>> {
        let mut locks = self.session_locks.lock().expect("session_locks poisoned");
        locks
            .entry(*session_id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// Get the directory path for a session
    fn session_dir(&self, session_id: &Uuid) -> PathBuf {
        self.base_path.join("sessions").join(session_id.to_string())
    }

    /// Get the messages file path for a session
    fn messages_file(&self, session_id: &Uuid) -> PathBuf {
        self.session_dir(session_id).join("messages.jsonl")
    }

    /// Get the pane list metadata file path for a session
    fn panes_file(&self, session_id: &Uuid) -> PathBuf {
        self.session_dir(session_id).join("panes.json")
    }

    fn pane_work_summaries_file(&self, session_id: &Uuid) -> PathBuf {
        self.session_dir(session_id)
            .join("pane-work-summaries.json")
    }

    /// Ensure the session directory exists
    async fn ensure_session_dir(&self, session_id: &Uuid) -> Result<()> {
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir).await?;
        Ok(())
    }

    /// Append a message to the session's message file
    pub async fn append_message(&self, session_id: &Uuid, message: &StoredMessage) -> Result<()> {
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().await;

        self.ensure_session_dir(session_id).await?;

        let file_path = self.messages_file(session_id);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;

        let mut json = serde_json::to_string(message)?;
        json.push('\n');
        file.write_all(json.as_bytes()).await?;
        // Flush the tokio buffer so a subsequent open() sees the bytes —
        // we don't sync to disk (that's the OS's call), but we do close
        // out the in-process write before returning to the caller.
        file.flush().await?;
        drop(file);

        // Enforce the size ceiling while we still hold the session lock, so a
        // concurrent append cannot interleave with the rewrite.
        self.enforce_session_size_cap(session_id, &file_path).await?;

        Ok(())
    }

    /// Drop the oldest messages once a session's log passes
    /// [`SESSION_MESSAGES_MAX_BYTES`].
    ///
    /// The age-based GC cannot do this job. It deletes what has aged past its
    /// retention window, which says nothing about volume: a pane republishing
    /// its transcript in a loop wrote 2.9 GB inside a single 7-day window, and
    /// the sweep correctly kept all of it while the server ran out of memory
    /// reading it back. This is the bound on *how much*, and it has to be
    /// enforced at append because that is the only point that sees every write.
    ///
    /// Caller must hold the session lock.
    async fn enforce_session_size_cap(&self, session_id: &Uuid, file_path: &Path) -> Result<()> {
        // std rather than tokio::fs: tokio's metadata can lag a write that
        // this very call just made (see get_messages_after for the same note).
        let size = match std::fs::metadata(file_path) {
            Ok(meta) => meta.len(),
            Err(_) => return Ok(()),
        };
        if size <= self.max_session_bytes {
            return Ok(());
        }

        // Trim well below the ceiling rather than to it, so the next append
        // does not immediately rewrite the file again.
        let keep_from = size.saturating_sub(self.trim_session_to_bytes);
        let tmp_path = session_gc_temp_path(file_path);

        let copy_result: Result<u64> = async {
            let file = fs::File::open(file_path).await?;
            let mut reader = BufReader::new(file);
            reader.seek(SeekFrom::Start(keep_from)).await?;
            // The seek almost certainly landed mid-line; discard that
            // fragment so the file never starts with half a record.
            let mut partial = String::new();
            reader.read_line(&mut partial).await?;

            let mut tmp = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)
                .await?;
            let copied = tokio::io::copy(&mut reader, &mut tmp).await?;
            tmp.flush().await?;
            Ok(copied)
        }
        .await;

        let copied = match copy_result {
            Ok(copied) => copied,
            Err(err) => {
                let _ = fs::remove_file(&tmp_path).await;
                return Err(err);
            }
        };

        if let Err(err) = fs::rename(&tmp_path, file_path).await {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(err.into());
        }

        // Loud on purpose: this discards a user's conversation history, and
        // reaching it at all means something upstream is writing far too much.
        tracing::warn!(
            %session_id,
            previous_bytes = size,
            retained_bytes = copied,
            cap_bytes = self.max_session_bytes,
            "Session message log exceeded its size cap — dropped the oldest messages"
        );
        Ok(())
    }

    /// Persist pane configurations for a session so inactive sessions can restore tabs after restart
    pub async fn save_pane_list(
        &self,
        session_id: &Uuid,
        panes: &[shared::PaneConfig],
    ) -> Result<()> {
        self.ensure_session_dir(session_id).await?;
        let file_path = self.panes_file(session_id);
        let json = serde_json::to_vec(panes)?;
        fs::write(file_path, json).await?;
        Ok(())
    }

    /// Load persisted pane configurations for a session
    pub async fn load_pane_list(&self, session_id: &Uuid) -> Result<Vec<shared::PaneConfig>> {
        let file_path = self.panes_file(session_id);
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read(file_path).await?;
        let panes = serde_json::from_slice::<Vec<shared::PaneConfig>>(&data)?;
        Ok(panes)
    }

    /// Read the durable summary cache. A malformed document fails closed so
    /// the caller cannot overwrite potentially recoverable cache data while
    /// rebuilding derived state.
    pub async fn load_pane_work_summaries(
        &self,
        session_id: &Uuid,
    ) -> Result<PaneWorkSummaryDocument> {
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().await;
        self.load_pane_work_summaries_unlocked(session_id).await
    }

    async fn load_pane_work_summaries_unlocked(
        &self,
        session_id: &Uuid,
    ) -> Result<PaneWorkSummaryDocument> {
        let path = self.pane_work_summaries_file(session_id);
        if !path.exists() {
            return Ok(PaneWorkSummaryDocument::default());
        }
        let bytes = fs::read(&path).await?;
        let document: PaneWorkSummaryDocument = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            document.version == PANE_WORK_SUMMARY_SIDECAR_VERSION,
            "unsupported pane summary sidecar version {}",
            document.version
        );
        Ok(document)
    }

    /// Atomically replace the versioned summary cache under the same
    /// per-session lock used by message GC and project deletion.
    pub async fn save_pane_work_summaries(
        &self,
        session_id: &Uuid,
        document: &PaneWorkSummaryDocument,
    ) -> Result<()> {
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().await;
        self.save_pane_work_summaries_unlocked(session_id, document)
            .await
    }

    async fn save_pane_work_summaries_unlocked(
        &self,
        session_id: &Uuid,
        document: &PaneWorkSummaryDocument,
    ) -> Result<()> {
        anyhow::ensure!(
            document.version == PANE_WORK_SUMMARY_SIDECAR_VERSION,
            "refusing to write unsupported pane summary sidecar version {}",
            document.version
        );
        self.ensure_session_dir(session_id).await?;
        let path = self.pane_work_summaries_file(session_id);
        let tmp_path = session_gc_temp_path(&path);
        let bytes = serde_json::to_vec_pretty(document)?;
        let write_result: Result<()> = async {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)
                .await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            fs::rename(&tmp_path, &path).await?;
            Ok(())
        }
        .await;
        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path).await;
        }
        write_result
    }

    /// Serialize read-modify-write updates so simultaneous stage results
    /// cannot lose one another.
    pub async fn update_pane_work_summaries<F>(
        &self,
        session_id: &Uuid,
        update: F,
    ) -> Result<PaneWorkSummaryDocument>
    where
        F: FnOnce(&mut PaneWorkSummaryDocument) -> Result<()>,
    {
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().await;
        let mut document = self.load_pane_work_summaries_unlocked(session_id).await?;
        update(&mut document)?;
        self.save_pane_work_summaries_unlocked(session_id, &document)
            .await?;
        Ok(document)
    }

    /// Queued/generating jobs are process-local. Requeue them on restart so
    /// the scheduler can reconstruct source chunks from retained messages.
    pub async fn recover_pane_work_summaries(
        &self,
        session_id: &Uuid,
    ) -> Result<PaneWorkSummaryDocument> {
        self.update_pane_work_summaries(session_id, |document| {
            for summary in &mut document.summaries {
                if matches!(
                    summary.status,
                    shared::PaneWorkSummaryStatus::Queued
                        | shared::PaneWorkSummaryStatus::Generating
                ) {
                    summary.status = shared::PaneWorkSummaryStatus::Queued;
                    summary.error = None;
                }
            }
            Ok(())
        })
        .await
    }

    /// Permanently remove all file-backed artifacts for the supplied sessions.
    /// Callers must first exclude new project writes; the per-session locks
    /// drain any append or GC operation that was already in flight.
    pub async fn delete_session_dirs(&self, session_ids: &[Uuid]) -> Result<()> {
        let mut ordered = session_ids.to_vec();
        ordered.sort_unstable();
        ordered.dedup();

        for session_id in ordered {
            let lock = self.session_lock(&session_id);
            let guard = lock.lock().await;
            match fs::remove_dir_all(self.session_dir(&session_id)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            drop(guard);

            // Remove only the same lock and only when no append/GC caller has
            // already cloned it. A waiter retains its Arc and keeps the lock
            // alive even if a future session reuses this UUID.
            let mut locks = self.session_locks.lock().expect("session_locks poisoned");
            if locks
                .get(&session_id)
                .is_some_and(|stored| Arc::ptr_eq(stored, &lock) && Arc::strong_count(stored) == 2)
            {
                locks.remove(&session_id);
            }
        }
        Ok(())
    }

    /// Copy historical artifacts from `source_session_id` into `target_session_id`
    /// when the target has no message history yet.
    /// Returns true when a copy happened.
    pub async fn seed_history_if_missing(
        &self,
        source_session_id: &Uuid,
        target_session_id: &Uuid,
    ) -> Result<bool> {
        if source_session_id == target_session_id {
            return Ok(false);
        }

        let source_messages = self.messages_file(source_session_id);
        if !source_messages.exists() {
            return Ok(false);
        }
        let source_meta = fs::metadata(&source_messages).await?;
        if source_meta.len() == 0 {
            return Ok(false);
        }

        let target_messages = self.messages_file(target_session_id);
        if target_messages.exists() {
            let target_meta = fs::metadata(&target_messages).await?;
            if target_meta.len() > 0 {
                return Ok(false);
            }
        }

        self.ensure_session_dir(target_session_id).await?;
        fs::copy(&source_messages, &target_messages).await?;

        let source_panes = self.panes_file(source_session_id);
        let target_panes = self.panes_file(target_session_id);
        if source_panes.exists() && !target_panes.exists() {
            fs::copy(&source_panes, &target_panes).await?;
        }

        Ok(true)
    }

    /// Read ALL messages for a session (no limit)
    /// Read at most the newest `max_bytes` of a session's log.
    ///
    /// Returns the messages oldest-first, plus whether older messages were
    /// skipped — a caller that cares about completeness has to know it is
    /// looking at a tail rather than the whole conversation.
    ///
    /// This exists because [`Self::get_messages`] is unbounded, and the one
    /// production caller that ran it on a timer parsed a 2.9 GB log into
    /// roughly 15 GB of structs every 15 minutes until the server died. Peak
    /// memory here is a function of `max_bytes`, never of the file.
    pub async fn get_messages_tail(
        &self,
        session_id: &Uuid,
        max_bytes: u64,
    ) -> Result<(Vec<StoredMessage>, bool)> {
        let file_path = self.messages_file(session_id);
        if !file_path.exists() {
            return Ok((Vec::new(), false));
        }
        // std rather than tokio: tokio's metadata can lag a very recent write
        // by another task, and a short size would silently drop the newest
        // messages — the ones this read most wants.
        let size = std::fs::metadata(&file_path)?.len();
        if size == 0 {
            return Ok((Vec::new(), false));
        }

        let keep_from = size.saturating_sub(max_bytes);
        let truncated = keep_from > 0;

        let file = fs::File::open(&file_path).await?;
        let mut reader = BufReader::new(file);
        if truncated {
            reader.seek(SeekFrom::Start(keep_from)).await?;
            // The seek lands mid-line; drop that fragment so parsing starts on
            // a record boundary.
            let mut partial = String::new();
            reader.read_line(&mut partial).await?;
        }

        let mut messages = Vec::new();
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<StoredMessage>(&line) {
                messages.push(msg);
            }
        }
        Ok((messages, truncated))
    }

    pub async fn get_messages(&self, session_id: &Uuid) -> Result<Vec<StoredMessage>> {
        let file_path = self.messages_file(session_id);

        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut all_messages = Vec::new();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => all_messages.push(msg),
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }

        Ok(all_messages)
    }

    /// Read messages for a session, optionally limited to the most recent N
    pub async fn get_messages_with_limit(
        &self,
        session_id: &Uuid,
        limit: Option<usize>,
    ) -> Result<Vec<StoredMessage>> {
        let (messages, _) = self.get_messages_paginated(session_id, limit, None).await?;
        Ok(messages)
    }

    /// Return every stored message with `created_at > after_created_at`,
    /// sorted ASC, capped at `CATCHUP_LIMIT`. Used by the web client to fill
    /// the gap after a WebSocket reconnect — the client passes the max
    /// `created_at` it has live-streamed and asks for everything newer the
    /// server has on disk. Returns an empty vec when the timestamp is at or
    /// past the tail (nothing missed).
    ///
    /// I/O: scans the file backward from EOF in `CHUNK_BYTES`-sized chunks
    /// and stops as soon as it sees a run of `REORDER_SLACK_LINES` lines at
    /// or older than the cutoff. For the common case (recent watermark,
    /// nothing missed) this touches roughly one chunk, not the whole file —
    /// previous forward-scan versions read the entire jsonl every call,
    /// which is what made fan-out catchup OOM the server on big sessions.
    ///
    /// Memory: O(CATCHUP_LIMIT × MAX_CONTENT_BYTES) plus one chunk buffer.
    pub async fn get_messages_after(
        &self,
        session_id: &Uuid,
        after_created_at: &str,
    ) -> Result<Vec<StoredMessage>> {
        // Catchup is meant to fill a "disconnect window" gap — typically
        // dozens of messages, not thousands. If a user has been gone long
        // enough that 500 messages aren't enough, the next page reload
        // will hit the initial-load snapshot path instead.
        const CATCHUP_LIMIT: usize = 500;
        // Per-message content cap applied at storage-read time so the
        // in-memory window can't blow up on a single huge tool_result. The
        // web-side `truncate_for_transit` re-applies envelope-aware
        // truncation on the way out; this is just a defensive byte cap.
        const MAX_CONTENT_BYTES: usize = 64 * 1024;
        // Read size for each backward seek. Big enough that recent-cutoff
        // catchups finish in one read; small enough that we can stop early
        // on the next chunk if needed.
        const CHUNK_BYTES: usize = 64 * 1024;
        // Tolerance for slight microsecond reordering near the cutoff. The
        // jsonl is append-ordered, but two messages dispatched in the same
        // tick may land out of timestamp order; keep scanning a short way
        // past the first "too old" line before giving up.
        const REORDER_SLACK_LINES: usize = 50;

        let file_path = self.messages_file(session_id);
        if !file_path.exists() {
            return Ok(Vec::new());
        }

        // Use std stat for the size: tokio's File::metadata can lag behind
        // very recent writes by another tokio task on the same file (we hit
        // this in the test that appends a 1 MiB message and immediately
        // reads it back — tokio reports a short size, std reports the
        // committed one). A backward seek using a short size truncates the
        // result, so we read short and lose the head of the line.
        let file_size = std::fs::metadata(&file_path)?.len();
        if file_size == 0 {
            return Ok(Vec::new());
        }
        let mut file = fs::File::open(&file_path).await?;

        // We collect newest-first as the reverse scan finds them; sorted
        // ASC at the end before returning.
        let mut found: Vec<StoredMessage> = Vec::new();
        let mut slack: usize = 0;
        let mut hit_cap = false;
        let mut pos: u64 = file_size;
        // The leftmost bytes of each chunk usually start mid-line; that
        // fragment is held until the previous chunk's read prepends the
        // rest of the line to it.
        let mut carry: Vec<u8> = Vec::new();
        let mut stop = false;

        'outer: while pos > 0 && !stop {
            let chunk_size = pos.min(CHUNK_BYTES as u64) as usize;
            let new_pos = pos - chunk_size as u64;
            file.seek(SeekFrom::Start(new_pos)).await?;
            let mut buf = vec![0u8; chunk_size];
            file.read_exact(&mut buf).await?;
            pos = new_pos;

            // The carry from the previous (later-in-file) iteration is the
            // tail of a line that began in this earlier chunk; glue it on.
            if !carry.is_empty() {
                buf.extend_from_slice(&carry);
                carry.clear();
            }

            // Trim the trailing newline (the file's final '\n') so we don't
            // process an empty "line" at the end.
            let mut end = buf.len();
            while end > 0 && buf[end - 1] == b'\n' {
                end -= 1;
            }

            while end > 0 {
                let nl = buf[..end].iter().rposition(|&b| b == b'\n');
                match nl {
                    Some(nl_idx) => {
                        let line = &buf[nl_idx + 1..end];
                        end = nl_idx;
                        if line.is_empty() {
                            continue;
                        }
                        if !consume_line(
                            line,
                            after_created_at,
                            MAX_CONTENT_BYTES,
                            CATCHUP_LIMIT,
                            REORDER_SLACK_LINES,
                            &mut found,
                            &mut slack,
                            &mut hit_cap,
                        ) {
                            stop = true;
                            break 'outer;
                        }
                    }
                    None => {
                        // Buffer head is a partial line that started in an
                        // earlier chunk — save and let the next iteration
                        // glue it on. Unless this IS the first chunk
                        // (pos == 0), in which case it's a complete line.
                        if pos > 0 {
                            carry = buf[..end].to_vec();
                        } else if end > 0 {
                            let line = &buf[..end];
                            let _ = consume_line(
                                line,
                                after_created_at,
                                MAX_CONTENT_BYTES,
                                CATCHUP_LIMIT,
                                REORDER_SLACK_LINES,
                                &mut found,
                                &mut slack,
                                &mut hit_cap,
                            );
                        }
                        break;
                    }
                }
            }
        }

        // If we never finished the loop because we hit pos == 0, any leftover
        // carry is the file's first line; consume it.
        if !carry.is_empty() && !stop {
            let _ = consume_line(
                &carry,
                after_created_at,
                MAX_CONTENT_BYTES,
                CATCHUP_LIMIT,
                REORDER_SLACK_LINES,
                &mut found,
                &mut slack,
                &mut hit_cap,
            );
        }

        if hit_cap {
            tracing::warn!(
                "Catchup for session {} hit the {} message cap; older gap messages skipped",
                session_id,
                CATCHUP_LIMIT
            );
        }

        // Reverse: we pushed newest-first; the client expects ASC. Also sort
        // to absorb microsecond-scale reordering within the kept window.
        found.reverse();
        found.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(found)
    }

    /// Per-pane variant of `get_messages_after`. Each pane has its own
    /// `created_at` watermark — a message is returned only when its
    /// pane's watermark is satisfied. Solves the over-fetch in the
    /// single-cutoff form: a fast-streaming pane no longer drags the
    /// catchup window past slower panes' tails, and slower panes don't
    /// re-receive everything they already had just because some other
    /// pane is busy.
    ///
    /// Behavior:
    ///   * Lines whose `pane_id` IS in `pane_watermarks` are kept iff
    ///     `created_at > pane_watermarks[pane_id]`.
    ///   * Lines whose `pane_id` is NOT in `pane_watermarks` are always
    ///     kept — the client has never seen this pane, so every record
    ///     is new to it.
    ///   * Scan stops on `CATCHUP_LIMIT` matches OR on a slack window
    ///     of lines all at-or-older than the MIN of watermarks (no
    ///     further matches possible). MIN keeps the scan correct for
    ///     the worst-case pane; per-line filtering keeps the result
    ///     minimal for the others.
    pub async fn get_messages_per_pane_after(
        &self,
        session_id: &Uuid,
        pane_watermarks: &std::collections::HashMap<u32, String>,
    ) -> Result<Vec<StoredMessage>> {
        const CATCHUP_LIMIT: usize = 500;
        const MAX_CONTENT_BYTES: usize = 64 * 1024;
        const CHUNK_BYTES: usize = 64 * 1024;
        const REORDER_SLACK_LINES: usize = 50;

        let file_path = self.messages_file(session_id);
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file_size = std::fs::metadata(&file_path)?.len();
        if file_size == 0 {
            return Ok(Vec::new());
        }

        // Stop-condition cutoff is the MIN of provided watermarks —
        // once we're scanning lines older than every pane's watermark,
        // nothing newer can possibly turn up. An empty watermark map
        // (client knows nothing) treats every line as kept; we still
        // cap at CATCHUP_LIMIT via the per-line consumer.
        let min_cutoff: String = pane_watermarks.values().min().cloned().unwrap_or_default();

        let mut file = fs::File::open(&file_path).await?;
        let mut found: Vec<StoredMessage> = Vec::new();
        let mut slack: usize = 0;
        let mut hit_cap = false;
        let mut pos: u64 = file_size;
        let mut carry: Vec<u8> = Vec::new();
        let mut stop = false;

        'outer: while pos > 0 && !stop {
            let chunk_size = pos.min(CHUNK_BYTES as u64) as usize;
            let new_pos = pos - chunk_size as u64;
            file.seek(SeekFrom::Start(new_pos)).await?;
            let mut buf = vec![0u8; chunk_size];
            file.read_exact(&mut buf).await?;
            pos = new_pos;

            if !carry.is_empty() {
                buf.extend_from_slice(&carry);
                carry.clear();
            }

            let mut end = buf.len();
            while end > 0 && buf[end - 1] == b'\n' {
                end -= 1;
            }

            while end > 0 {
                let nl = buf[..end].iter().rposition(|&b| b == b'\n');
                match nl {
                    Some(nl_idx) => {
                        let line = &buf[nl_idx + 1..end];
                        end = nl_idx;
                        if line.is_empty() {
                            continue;
                        }
                        if !consume_per_pane_after_line(
                            line,
                            pane_watermarks,
                            &min_cutoff,
                            MAX_CONTENT_BYTES,
                            CATCHUP_LIMIT,
                            REORDER_SLACK_LINES,
                            &mut found,
                            &mut slack,
                            &mut hit_cap,
                        ) {
                            stop = true;
                            break 'outer;
                        }
                    }
                    None => {
                        if pos > 0 {
                            carry = buf[..end].to_vec();
                        } else if end > 0 {
                            let _ = consume_per_pane_after_line(
                                &buf[..end],
                                pane_watermarks,
                                &min_cutoff,
                                MAX_CONTENT_BYTES,
                                CATCHUP_LIMIT,
                                REORDER_SLACK_LINES,
                                &mut found,
                                &mut slack,
                                &mut hit_cap,
                            );
                        }
                        break;
                    }
                }
            }
        }

        if !carry.is_empty() && !stop {
            let _ = consume_per_pane_after_line(
                &carry,
                pane_watermarks,
                &min_cutoff,
                MAX_CONTENT_BYTES,
                CATCHUP_LIMIT,
                REORDER_SLACK_LINES,
                &mut found,
                &mut slack,
                &mut hit_cap,
            );
        }

        if hit_cap {
            tracing::warn!(
                "Per-pane catchup for session {} hit the {} message cap; older gap messages skipped",
                session_id,
                CATCHUP_LIMIT
            );
        }

        found.reverse();
        found.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(found)
    }

    /// Read messages for a session with pagination support
    /// Returns (messages, has_more)
    pub async fn get_messages_paginated(
        &self,
        session_id: &Uuid,
        limit: Option<usize>,
        before_id: Option<&str>,
    ) -> Result<(Vec<StoredMessage>, bool)> {
        let file_path = self.messages_file(session_id);

        if !file_path.exists() {
            return Ok((Vec::new(), false));
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut all_messages = Vec::new();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => all_messages.push(msg),
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }

        // If before_id is specified, find messages before that ID
        let messages = if let Some(before_id) = before_id {
            // Find the index of the message with before_id
            if let Some(idx) = all_messages.iter().position(|m| m.id == before_id) {
                // Take messages before this index
                all_messages[..idx].to_vec()
            } else {
                // ID not found, return empty
                Vec::new()
            }
        } else {
            all_messages
        };

        // Apply limit (take from the end to get most recent)
        let limit = limit.unwrap_or(100);
        let has_more = messages.len() > limit;
        let result = if messages.len() > limit {
            messages[messages.len() - limit..].to_vec()
        } else {
            messages
        };

        Ok((result, has_more))
    }

    /// Read messages for a session with pagination support, optionally filtered by pane type (legacy)
    /// Returns (messages, has_more)
    pub async fn get_messages_paginated_by_pane(
        &self,
        session_id: &Uuid,
        limit: Option<usize>,
        before_id: Option<&str>,
        pane_type: Option<shared::PaneType>,
    ) -> Result<(Vec<StoredMessage>, bool)> {
        let pane_filter = pane_type.map(|p| shared::PaneConfig::pane_id_from_legacy(&p));
        self.get_messages_paginated_by_pane_id(session_id, limit, before_id, pane_filter)
            .await
    }

    /// Read messages for a session with pagination support, optionally filtered by pane_id
    /// Returns (messages, has_more)
    pub async fn get_messages_paginated_by_pane_id(
        &self,
        session_id: &Uuid,
        limit: Option<usize>,
        before_id: Option<&str>,
        pane_id: Option<u32>,
    ) -> Result<(Vec<StoredMessage>, bool)> {
        let file_path = self.messages_file(session_id);

        if !file_path.exists() {
            return Ok((Vec::new(), false));
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut all_messages = Vec::new();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => {
                    // Filter by pane_id if specified
                    if let Some(filter) = pane_id {
                        if parse_stored_pane_id(msg.pane_type.as_deref()) == Some(filter) {
                            all_messages.push(msg);
                        }
                    } else {
                        all_messages.push(msg);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }

        // If before_id is specified, find messages before that ID
        let messages = if let Some(before_id) = before_id {
            if let Some(idx) = all_messages.iter().position(|m| m.id == before_id) {
                all_messages[..idx].to_vec()
            } else {
                Vec::new()
            }
        } else {
            all_messages
        };

        // Apply limit (take from the end to get most recent)
        let limit = limit.unwrap_or(100);
        let has_more = messages.len() > limit;
        let result = if messages.len() > limit {
            messages[messages.len() - limit..].to_vec()
        } else {
            messages
        };

        Ok((result, has_more))
    }

    /// Read messages for a session, loading recent messages per pane
    /// This ensures all panes have messages included
    /// Returns (messages, has_more) where messages are sorted by created_at
    /// Initial-load fetch: return the latest `limit_per_pane` messages for
    /// each pane in the session, combined and sorted ASC.
    ///
    /// Reads the file backward from EOF in `CHUNK_BYTES` chunks and stops
    /// once every discovered pane bucket is full (plus a small slack window
    /// to catch any in-flight pane the recent tail hasn't surfaced yet).
    /// On a 4 GB jsonl with a half-dozen active panes this touches roughly
    /// a few hundred KB of the file's tail instead of all 4 GB.
    ///
    /// `has_more` is true when at least one pane's bucket hit the cap (so
    /// older messages exist for pagination) or when the scan bailed on the
    /// slack window (older messages exist somewhere in the head).
    pub async fn get_messages_per_pane(
        &self,
        session_id: &Uuid,
        limit_per_pane: usize,
    ) -> Result<(Vec<StoredMessage>, bool)> {
        const CHUNK_BYTES: usize = 64 * 1024;
        // Tighter content cap for initial load than the per-call catchup
        // budget. The Hashbrown-port style workers in big projects emit
        // tool_results that include full file dumps — a single message
        // can be hundreds of KB. At 16 KB the user still sees the head of
        // the result (enough to know what happened); the full content is
        // available in the agent's session JSONL if needed.
        const MAX_CONTENT_BYTES: usize = 16 * 1024;
        // After every known pane bucket is full, keep reading this many
        // more lines so a newly-active pane lurking just past the scan
        // window doesn't get missed entirely.
        const SLACK_LINES_AFTER_ALL_FULL: usize = 500;

        let file_path = self.messages_file(session_id);
        if !file_path.exists() {
            return Ok((Vec::new(), false));
        }
        let file_size = std::fs::metadata(&file_path)?.len();
        if file_size == 0 {
            return Ok((Vec::new(), false));
        }

        let mut file = fs::File::open(&file_path).await?;
        // Per-pane bucket, newest-first as discovered by the reverse scan.
        let mut buckets: std::collections::HashMap<Option<String>, Vec<StoredMessage>> =
            std::collections::HashMap::new();
        let mut has_more = false;
        let mut slack: usize = 0;
        let mut pos: u64 = file_size;
        let mut carry: Vec<u8> = Vec::new();
        let mut stop = false;

        'outer: while pos > 0 && !stop {
            let chunk_size = pos.min(CHUNK_BYTES as u64) as usize;
            let new_pos = pos - chunk_size as u64;
            file.seek(SeekFrom::Start(new_pos)).await?;
            let mut buf = vec![0u8; chunk_size];
            file.read_exact(&mut buf).await?;
            pos = new_pos;

            if !carry.is_empty() {
                buf.extend_from_slice(&carry);
                carry.clear();
            }

            let mut end = buf.len();
            while end > 0 && buf[end - 1] == b'\n' {
                end -= 1;
            }

            while end > 0 {
                let nl = buf[..end].iter().rposition(|&b| b == b'\n');
                match nl {
                    Some(nl_idx) => {
                        let line = &buf[nl_idx + 1..end];
                        end = nl_idx;
                        if line.is_empty() {
                            continue;
                        }
                        if !consume_per_pane_line(
                            line,
                            limit_per_pane,
                            MAX_CONTENT_BYTES,
                            SLACK_LINES_AFTER_ALL_FULL,
                            &mut buckets,
                            &mut has_more,
                            &mut slack,
                        ) {
                            stop = true;
                            break 'outer;
                        }
                    }
                    None => {
                        if pos > 0 {
                            carry = buf[..end].to_vec();
                        } else if end > 0 {
                            let _ = consume_per_pane_line(
                                &buf[..end],
                                limit_per_pane,
                                MAX_CONTENT_BYTES,
                                SLACK_LINES_AFTER_ALL_FULL,
                                &mut buckets,
                                &mut has_more,
                                &mut slack,
                            );
                        }
                        break;
                    }
                }
            }
        }

        if !carry.is_empty() && !stop {
            let _ = consume_per_pane_line(
                &carry,
                limit_per_pane,
                MAX_CONTENT_BYTES,
                SLACK_LINES_AFTER_ALL_FULL,
                &mut buckets,
                &mut has_more,
                &mut slack,
            );
        }

        // Buckets are newest-first; flatten and sort ASC for the client.
        let mut combined = Vec::new();
        for (_pane, bucket) in buckets {
            combined.extend(bucket);
        }
        combined.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok((combined, has_more))
    }

    /// Drop every message with `created_at < cutoff` from a session's
    /// messages.jsonl. Reads the file, filters lines whose timestamp falls
    /// before the cutoff, writes the survivors to a unique same-directory
    /// staging file, then atomically renames over the original. Holds the
    /// per-session lock for the whole operation so concurrent appends queue
    /// cleanly.
    ///
    /// Returns (kept, dropped, bytes_freed). Lines that fail to parse are
    /// kept defensively so a parser regression can't silently delete data.
    pub async fn gc_session_before(
        &self,
        session_id: &Uuid,
        cutoff: DateTime<Utc>,
    ) -> Result<(u64, u64, u64)> {
        let lock = self.session_lock(session_id);
        let _guard = lock.lock().await;

        let file_path = self.messages_file(session_id);
        if !file_path.exists() {
            return Ok((0, 0, 0));
        }

        let original_size = fs::metadata(&file_path).await?.len();

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let tmp_path = session_gc_temp_path(&file_path);

        let rewrite_result: Result<(u64, u64)> = async {
            let mut tmp = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)
                .await?;

            let mut kept: u64 = 0;
            let mut dropped: u64 = 0;

            while let Some(line) = lines.next_line().await? {
                if line.trim().is_empty() {
                    continue;
                }
                // Parse just enough to read created_at. Unparseable lines are
                // kept — better to retain mystery data than to silently nuke it.
                let keep = match serde_json::from_str::<StoredMessage>(&line) {
                    Ok(msg) => match DateTime::parse_from_rfc3339(&msg.created_at) {
                        Ok(ts) => ts.with_timezone(&Utc) >= cutoff,
                        Err(_) => true,
                    },
                    Err(_) => true,
                };
                if keep {
                    tmp.write_all(line.as_bytes()).await?;
                    tmp.write_all(b"\n").await?;
                    kept += 1;
                } else {
                    dropped += 1;
                }
            }

            tmp.flush().await?;
            drop(tmp);
            Ok((kept, dropped))
        }
        .await;

        let (kept, dropped) = match rewrite_result {
            Ok(counts) => counts,
            Err(err) => {
                let _ = fs::remove_file(&tmp_path).await;
                return Err(err);
            }
        };

        if dropped == 0 {
            // Nothing to rewrite — leave the original alone.
            let _ = fs::remove_file(&tmp_path).await;
            return Ok((kept, 0, 0));
        }

        if let Err(err) = fs::rename(&tmp_path, &file_path).await {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(err.into());
        }
        let new_size = fs::metadata(&file_path).await?.len();
        let bytes_freed = original_size.saturating_sub(new_size);
        Ok((kept, dropped, bytes_freed))
    }

    /// Walk every session directory and GC each. Returns aggregated stats so
    /// the periodic task can log a single summary line per run.
    pub async fn gc_all_sessions_before(&self, cutoff: DateTime<Utc>) -> Result<GcStats> {
        let mut stats = GcStats::default();
        let sessions = self.list_sessions_with_messages().await?;
        for sid in sessions {
            stats.sessions_scanned += 1;
            match self.gc_session_before(&sid, cutoff).await {
                Ok((kept, dropped, freed)) => {
                    stats.messages_kept += kept;
                    stats.messages_dropped += dropped;
                    stats.bytes_freed += freed;
                    if dropped > 0 {
                        stats.sessions_modified += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!("GC failed for session {}: {} — leaving file intact", sid, e);
                }
            }
        }
        Ok(stats)
    }

    /// List all session IDs that have message files
    pub async fn list_sessions_with_messages(&self) -> Result<Vec<Uuid>> {
        let sessions_dir = self.base_path.join("sessions");

        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        let mut entries = fs::read_dir(&sessions_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                let name = entry.file_name();
                if let Some(name_str) = name.to_str() {
                    if let Ok(uuid) = Uuid::parse_str(name_str) {
                        // Check if messages.jsonl exists
                        let messages_file = entry.path().join("messages.jsonl");
                        if messages_file.exists() {
                            sessions.push(uuid);
                        }
                    }
                }
            }
        }

        Ok(sessions)
    }
}

/// Process one line emitted by the tail-first reverse scan.
///
/// Returns `false` to signal "stop scanning" — either we've collected
/// `CATCHUP_LIMIT` matches or we've hit the slack threshold for lines
/// older than the cutoff. `true` means "keep going."
fn consume_line(
    line: &[u8],
    after_created_at: &str,
    max_content: usize,
    limit: usize,
    slack_limit: usize,
    found: &mut Vec<StoredMessage>,
    slack: &mut usize,
    hit_cap: &mut bool,
) -> bool {
    let mut msg = match serde_json::from_slice::<StoredMessage>(line) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse message line ({} bytes): {}", line.len(), e);
            return true;
        }
    };
    if msg.created_at.as_str() <= after_created_at {
        *slack += 1;
        return *slack < slack_limit;
    }
    msg.content = truncate_message_content(msg.content, &msg.message_type, max_content, "catchup");
    if found.len() >= limit {
        *hit_cap = true;
        return false;
    }
    found.push(msg);
    true
}

/// Per-line consumer for `get_messages_per_pane_after`. Filters
/// each line by the watermark of its own pane (rather than a single
/// session-level cutoff), so a fast pane's recent traffic doesn't
/// drag the catchup result past a slow pane's tail.
fn consume_per_pane_after_line(
    line: &[u8],
    pane_watermarks: &std::collections::HashMap<u32, String>,
    min_cutoff: &str,
    max_content: usize,
    limit: usize,
    slack_limit: usize,
    found: &mut Vec<StoredMessage>,
    slack: &mut usize,
    hit_cap: &mut bool,
) -> bool {
    let mut msg = match serde_json::from_slice::<StoredMessage>(line) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse message line ({} bytes): {}", line.len(), e);
            return true;
        }
    };
    // Walk-back termination: when the line predates EVERY pane's
    // watermark, we've left the catchup window. Bump slack to absorb
    // microsecond reordering near the cutoff; bail when slack saturates.
    if msg.created_at.as_str() <= min_cutoff {
        *slack += 1;
        return *slack < slack_limit;
    }
    // Per-pane inclusion. A pane the client hasn't seen at all
    // (no watermark) is treated as a brand-new pane — every record
    // for it is kept.
    let pane_id = parse_stored_pane_id(msg.pane_type.as_deref());
    let keep = match pane_id {
        Some(pid) => match pane_watermarks.get(&pid) {
            Some(wm) => msg.created_at.as_str() > wm.as_str(),
            None => true,
        },
        None => {
            // No pane_id on the record (legacy single-pane bucket).
            // Keep iff watermarks map has a None-bucket sentinel, OR
            // when the map is empty (treat as "client wants everything").
            // We don't expose a sentinel on the wire yet, so default
            // to keep — the client-side dedupe-by-id covers duplicates.
            true
        }
    };
    if !keep {
        // Pane known to client and message already in client cache.
        // Reset slack so we don't bail prematurely on a sparse pane.
        *slack = 0;
        return true;
    }
    msg.content = truncate_message_content(msg.content, &msg.message_type, max_content, "catchup");
    if found.len() >= limit {
        *hit_cap = true;
        return false;
    }
    found.push(msg);
    *slack = 0;
    true
}

/// Process one line emitted by the per-pane reverse scan in
/// `get_messages_per_pane`. Returns `false` to signal "stop scanning"
/// once every known pane bucket is full AND we've consumed the slack
/// window without discovering a new pane or filling an existing one.
fn consume_per_pane_line(
    line: &[u8],
    limit_per_pane: usize,
    max_content: usize,
    slack_limit: usize,
    buckets: &mut HashMap<Option<String>, Vec<StoredMessage>>,
    has_more: &mut bool,
    slack: &mut usize,
) -> bool {
    let mut msg = match serde_json::from_slice::<StoredMessage>(line) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse message line ({} bytes): {}", line.len(), e);
            return true;
        }
    };
    let bucket_key = normalized_bucket_key(msg.pane_type.as_deref());
    let is_new_bucket = !buckets.contains_key(&bucket_key);
    let bucket = buckets.entry(bucket_key).or_default();

    if bucket.len() >= limit_per_pane {
        // Already have the newest `limit_per_pane` for this pane. Older
        // messages exist, so flag has_more for the paginator.
        *has_more = true;
        *slack += 1;
        let all_full = !buckets.is_empty() && buckets.values().all(|b| b.len() >= limit_per_pane);
        return !(all_full && *slack >= slack_limit);
    }

    msg.content =
        truncate_message_content(msg.content, &msg.message_type, max_content, "initial load");
    bucket.push(msg);
    // A new bucket or a fresh push extends the discovery horizon — reset
    // the slack counter so we don't bail too early on a sparse pane.
    if is_new_bucket {
        *slack = 0;
    } else {
        *slack = 0;
    }
    true
}

fn normalized_bucket_key(raw_pane_type: Option<&str>) -> Option<String> {
    if let Some(id) = parse_stored_pane_id(raw_pane_type) {
        return Some(id.to_string());
    }
    raw_pane_type.map(|s| s.to_string())
}

fn parse_stored_pane_id(raw_pane_type: Option<&str>) -> Option<u32> {
    let raw = raw_pane_type?.trim();
    if raw.is_empty() {
        return None;
    }

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

    let trailing_digits_rev: String = lower
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if trailing_digits_rev.is_empty() {
        return None;
    }
    let trailing_digits: String = trailing_digits_rev.chars().rev().collect();
    trailing_digits.parse::<u32>().ok()
}

#[cfg(test)]
mod gc_tests {
    use super::{session_gc_temp_path, FileStorage, StoredMessage};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn fresh_storage() -> FileStorage {
        let base = std::env::temp_dir().join(format!("apas-gc-test-{}", Uuid::new_v4()));
        FileStorage::new(base)
    }

    fn make_msg(id: &str, ts_iso: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: "assistant".into(),
            content: "hello".into(),
            message_type: "text".into(),
            created_at: ts_iso.into(),
            pane_type: Some("2".into()),
        }
    }

    async fn session_tmp_files(storage: &FileStorage, sid: &Uuid) -> Vec<String> {
        let mut entries = tokio::fs::read_dir(storage.session_dir(sid)).await.unwrap();
        let mut tmp_files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp") {
                tmp_files.push(name);
            }
        }
        tmp_files.sort();
        tmp_files
    }

    #[test]
    fn gc_temp_paths_are_unique_and_same_directory() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let messages = storage.messages_file(&sid);
        let first = session_gc_temp_path(&messages);
        let second = session_gc_temp_path(&messages);

        assert_ne!(first, second);
        assert_eq!(first.parent(), messages.parent());
        assert_eq!(second.parent(), messages.parent());
        assert_ne!(first, messages.with_extension("jsonl.tmp"));

        let file_name = first.file_name().unwrap().to_string_lossy();
        assert!(file_name.starts_with("messages.jsonl.gc."));
        assert!(file_name.ends_with(".tmp"));
    }

    #[tokio::test]
    async fn append_trims_a_session_that_outgrows_its_size_cap() {
        // The failure this guards: a pane republishing its transcript wrote
        // 2.9 GB into one session inside the GC's 7-day window, so the
        // age-based sweep correctly kept every byte and the server exhausted
        // its memory reading the file back.
        let storage = fresh_storage().with_session_size_cap(8 * 1024, 4 * 1024);
        let sid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        for i in 0..400 {
            storage
                .append_message(&sid, &make_msg(&format!("m{i}"), &now))
                .await
                .unwrap();
        }

        let size = std::fs::metadata(storage.messages_file(&sid)).unwrap().len();
        assert!(
            size <= 8 * 1024,
            "log stayed over its cap after appends: {size} bytes"
        );

        // The newest messages are the ones worth keeping, and every retained
        // line must still parse — a trim that cut mid-record would corrupt the
        // store just as surely as letting it grow.
        let kept = storage.get_messages(&sid).await.unwrap();
        assert!(!kept.is_empty(), "trim emptied the session");
        assert_eq!(
            kept.last().unwrap().id,
            "m399",
            "trim discarded the newest message"
        );
        assert!(
            kept.iter().all(|m| m.role == "assistant"),
            "a partial line survived the trim"
        );
        assert!(
            session_tmp_files(&storage, &sid).await.is_empty(),
            "trim left a temp file behind"
        );
    }

    #[tokio::test]
    async fn tail_read_is_bounded_and_reports_that_it_skipped_older_messages() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        for i in 0..500 {
            storage
                .append_message(&sid, &make_msg(&format!("m{i}"), &now))
                .await
                .unwrap();
        }
        let full = storage.get_messages(&sid).await.unwrap();
        assert_eq!(full.len(), 500);

        let (tail, truncated) = storage.get_messages_tail(&sid, 2 * 1024).await.unwrap();
        assert!(truncated, "a partial read must say so");
        assert!(tail.len() < full.len(), "tail read was not bounded");
        assert!(!tail.is_empty());
        // Newest-biased, and never starting mid-record.
        assert_eq!(tail.last().unwrap().id, "m499");
        assert!(tail.iter().all(|m| m.role == "assistant"));
    }

    #[tokio::test]
    async fn tail_read_returns_everything_when_the_log_fits() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        for i in 0..5 {
            storage
                .append_message(&sid, &make_msg(&format!("m{i}"), &now))
                .await
                .unwrap();
        }

        let (tail, truncated) = storage.get_messages_tail(&sid, 1024 * 1024).await.unwrap();
        assert!(!truncated, "a complete read must not claim truncation");
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0].id, "m0");
    }

    #[tokio::test]
    async fn tail_read_of_a_missing_session_is_empty_not_an_error() {
        let storage = fresh_storage();
        let (tail, truncated) = storage
            .get_messages_tail(&Uuid::new_v4(), 1024)
            .await
            .unwrap();
        assert!(tail.is_empty());
        assert!(!truncated);
    }

    #[tokio::test]
    async fn append_leaves_a_session_under_the_cap_untouched() {
        let storage = fresh_storage().with_session_size_cap(8 * 1024, 4 * 1024);
        let sid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        for i in 0..5 {
            storage
                .append_message(&sid, &make_msg(&format!("m{i}"), &now))
                .await
                .unwrap();
        }

        let kept = storage.get_messages(&sid).await.unwrap();
        assert_eq!(kept.len(), 5, "an under-cap session must not be rewritten");
        assert_eq!(kept[0].id, "m0");
    }

    #[tokio::test]
    async fn gc_drops_messages_older_than_cutoff() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now();
        let old = (now - Duration::days(45)).to_rfc3339();
        let recent = (now - Duration::days(2)).to_rfc3339();

        storage
            .append_message(&sid, &make_msg("old1", &old))
            .await
            .unwrap();
        storage
            .append_message(&sid, &make_msg("old2", &old))
            .await
            .unwrap();
        storage
            .append_message(&sid, &make_msg("recent1", &recent))
            .await
            .unwrap();

        let cutoff = now - Duration::days(30);
        let (kept, dropped, freed) = storage.gc_session_before(&sid, cutoff).await.unwrap();
        assert_eq!(kept, 1);
        assert_eq!(dropped, 2);
        assert!(freed > 0);

        let survivors = storage.get_messages(&sid).await.unwrap();
        assert_eq!(survivors.len(), 1);
        assert_eq!(survivors[0].id, "recent1");
        assert!(!storage
            .messages_file(&sid)
            .with_extension("jsonl.tmp")
            .exists());
        assert_eq!(
            session_tmp_files(&storage, &sid).await,
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn get_messages_after_returns_only_newer_sorted_asc() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now();
        // Write three messages in reverse-time order so we exercise the sort.
        let t1 = (now - Duration::seconds(30)).to_rfc3339();
        let t2 = (now - Duration::seconds(20)).to_rfc3339();
        let t3 = (now - Duration::seconds(10)).to_rfc3339();

        storage
            .append_message(&sid, &make_msg("c", &t3))
            .await
            .unwrap();
        storage
            .append_message(&sid, &make_msg("a", &t1))
            .await
            .unwrap();
        storage
            .append_message(&sid, &make_msg("b", &t2))
            .await
            .unwrap();

        // Cut between t1 and t2 — expect b and c, in created_at ASC order.
        let after = (now - Duration::seconds(25)).to_rfc3339();
        let got = storage.get_messages_after(&sid, &after).await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "b");
        assert_eq!(got[1].id, "c");

        // Cut after t3 — nothing missed.
        let after = (now + Duration::seconds(5)).to_rfc3339();
        let got = storage.get_messages_after(&sid, &after).await.unwrap();
        assert!(got.is_empty());

        // Empty string as cutoff — return everything.
        let got = storage.get_messages_after(&sid, "").await.unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].id, "a");
        assert_eq!(got[2].id, "c");
    }

    #[tokio::test]
    async fn get_messages_after_caps_window_keeping_newest() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let base = Utc::now() - Duration::hours(1);
        // Write more than the cap; expect the newest to be kept.
        let total = 550u32;
        for i in 0..total {
            let ts = (base + Duration::milliseconds(i as i64)).to_rfc3339();
            storage
                .append_message(&sid, &make_msg(&format!("m{i}"), &ts))
                .await
                .unwrap();
        }
        let got = storage.get_messages_after(&sid, "").await.unwrap();
        // Sliding window keeps the newest `CATCHUP_LIMIT` (500). Oldest are dropped.
        assert_eq!(got.len(), 500);
        let dropped = (total as usize) - got.len();
        assert_eq!(got.first().unwrap().id, format!("m{dropped}"));
        assert_eq!(got.last().unwrap().id, format!("m{}", total - 1));
    }

    #[tokio::test]
    async fn get_messages_per_pane_returns_latest_per_bucket_and_flags_has_more() {
        // 3 panes × 250 messages each, interleaved by round-robin. With
        // limit_per_pane=100 the function should return 300 messages and
        // flag has_more=true (older messages exist per pane).
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let base = Utc::now() - Duration::hours(1);
        let panes = ["1", "2", "3"];
        for i in 0..750u32 {
            let pane = panes[(i as usize) % panes.len()].to_string();
            let ts = (base + Duration::milliseconds(i as i64)).to_rfc3339();
            let mut msg = make_msg(&format!("m{i}"), &ts);
            msg.pane_type = Some(pane);
            storage.append_message(&sid, &msg).await.unwrap();
        }
        let (got, has_more) = storage.get_messages_per_pane(&sid, 100).await.unwrap();
        assert_eq!(got.len(), 300);
        assert!(has_more);
        // ASC by created_at; first one should be one of m{750-300}..m{750-298}
        // (the three panes' 100th-from-newest entries).
        let first_idx: u32 = got[0].id[1..].parse().unwrap();
        let last_idx: u32 = got[got.len() - 1].id[1..].parse().unwrap();
        assert!(first_idx >= 450);
        assert_eq!(last_idx, 749);
        // Each pane shows up with exactly 100 messages.
        let mut per_pane: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for m in &got {
            *per_pane.entry(m.pane_type.as_deref().unwrap()).or_insert(0) += 1;
        }
        assert_eq!(
            per_pane.values().copied().collect::<Vec<_>>(),
            vec![100, 100, 100]
        );
    }

    #[tokio::test]
    async fn get_messages_per_pane_returns_all_when_under_cap() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let base = Utc::now() - Duration::hours(1);
        for i in 0..50u32 {
            let ts = (base + Duration::milliseconds(i as i64)).to_rfc3339();
            let mut msg = make_msg(&format!("m{i}"), &ts);
            msg.pane_type = Some("2".into());
            storage.append_message(&sid, &msg).await.unwrap();
        }
        let (got, has_more) = storage.get_messages_per_pane(&sid, 100).await.unwrap();
        assert_eq!(got.len(), 50);
        assert!(!has_more);
    }

    #[tokio::test]
    async fn get_messages_after_keeps_tool_result_json_envelope_valid_after_truncation() {
        // Regression: a previous implementation truncated by raw byte cut
        // ("…[truncated for catchup; full size N bytes]" appended directly
        // to the JSON), which left invalid JSON and the web client's
        // JSON.parse fell back to rendering the raw string. The truncation
        // must produce a parseable JSON object.
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now();
        let ts = now.to_rfc3339();
        // Build a tool_result envelope with a big inner content + bulky
        // tool_use_result (mirrors what an Edit tool_result looks like on
        // a sizable source file).
        let big_inner = "x".repeat(200 * 1024);
        let big_diff = "y".repeat(200 * 1024);
        let envelope = serde_json::json!({
            "content": big_inner,
            "is_error": false,
            "tool_use_id": "toolu_abc",
            "tool_use_result": {
                "oldString": "old",
                "newString": "new",
                "originalFile": big_diff,
            },
        })
        .to_string();
        let mut msg = make_msg("tr", &ts);
        msg.message_type = "tool_result".to_string();
        msg.content = envelope;
        storage.append_message(&sid, &msg).await.unwrap();

        let got = storage.get_messages_after(&sid, "").await.unwrap();
        assert_eq!(got.len(), 1);
        let returned_content = &got[0].content;
        // The truncated content MUST still be valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(returned_content)
            .expect("truncated tool_result must remain valid JSON");
        // Envelope fields preserved.
        assert_eq!(parsed["is_error"], false);
        assert_eq!(parsed["tool_use_id"], "toolu_abc");
        // Inner content is a truncated string, with marker.
        let inner = parsed["content"]
            .as_str()
            .expect("inner content stays a string");
        assert!(inner.contains("truncated for catchup"));
        // The bulky tool_use_result was dropped on truncation.
        assert!(parsed.get("tool_use_result").is_none());
    }

    #[tokio::test]
    async fn get_messages_after_keeps_tool_use_string_input_envelope_valid_after_truncation() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let ts = Utc::now().to_rfc3339();
        let big_input = "run ".repeat(60 * 1024);
        let envelope = serde_json::json!({
            "id": "toolu_bash",
            "name": "Bash",
            "input": big_input,
        })
        .to_string();
        let mut msg = make_msg("tu-string", &ts);
        msg.message_type = "tool_use".to_string();
        msg.content = envelope;
        storage.append_message(&sid, &msg).await.unwrap();

        let got = storage.get_messages_after(&sid, "").await.unwrap();
        assert_eq!(got.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&got[0].content)
            .expect("truncated tool_use must remain valid JSON");
        assert_eq!(parsed["id"], "toolu_bash");
        assert_eq!(parsed["name"], "Bash");
        let input = parsed["input"]
            .as_str()
            .expect("string input stays a string");
        assert!(input.starts_with("run run"));
        assert!(input.contains("truncated for catchup"));
    }

    #[tokio::test]
    async fn get_messages_after_replaces_structured_tool_use_input_with_marker_after_truncation() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let ts = Utc::now().to_rfc3339();
        let envelope = serde_json::json!({
            "id": "toolu_question",
            "name": "AskUserQuestion",
            "input": {
                "questions": [
                    {
                        "id": "deployment_choice",
                        "header": "Deploy",
                        "question": "x".repeat(200 * 1024),
                        "options": [
                            {"label": "Ship", "description": "Deploy now"},
                            {"label": "Wait", "description": "Hold deployment"}
                        ]
                    }
                ]
            },
        })
        .to_string();
        let original_len = envelope.len();
        let mut msg = make_msg("tu-object", &ts);
        msg.message_type = "tool_use".to_string();
        msg.content = envelope;
        storage.append_message(&sid, &msg).await.unwrap();

        let got = storage.get_messages_after(&sid, "").await.unwrap();
        assert_eq!(got.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(&got[0].content)
            .expect("truncated structured tool_use must remain valid JSON");
        assert_eq!(parsed["id"], "toolu_question");
        assert_eq!(parsed["name"], "AskUserQuestion");
        let input = parsed["input"]
            .as_object()
            .expect("structured input is replaced by a marker object");
        assert_eq!(
            input.get("_truncated").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            input.get("_reason").and_then(|v| v.as_str()),
            Some("catchup")
        );
        assert_eq!(
            input.get("_original_bytes").and_then(|v| v.as_u64()),
            Some(original_len as u64),
        );
        assert!(!input.contains_key("questions"));
    }

    #[tokio::test]
    async fn get_messages_after_handles_lines_split_across_chunk_boundaries() {
        // Each message body is padded so the resulting JSONL line is much
        // bigger than the 64 KiB chunk read; this exercises the carry/glue
        // path between successive backward chunks. Without the carry
        // handling, we'd silently drop messages whose JSON spans a chunk
        // boundary (since `from_slice` on a fragment fails to parse).
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let base = Utc::now() - Duration::hours(1);
        let big_payload = "x".repeat(200 * 1024); // > 3 chunks
        for i in 0..6u32 {
            let ts = (base + Duration::seconds(i as i64)).to_rfc3339();
            let mut msg = make_msg(&format!("big{i}"), &ts);
            msg.content = big_payload.clone();
            storage.append_message(&sid, &msg).await.unwrap();
        }
        // Empty cutoff → expect all six, sorted ASC by created_at, with
        // content truncated by the per-message cap (the body is bigger
        // than MAX_CONTENT_BYTES so the marker is added).
        let got = storage.get_messages_after(&sid, "").await.unwrap();
        assert_eq!(got.len(), 6);
        for (i, msg) in got.iter().enumerate() {
            assert_eq!(msg.id, format!("big{i}"));
            assert!(msg.content.contains("truncated for catchup"));
        }
    }

    #[tokio::test]
    async fn get_messages_after_bails_early_on_recent_cutoff() {
        // Functional sanity check on the early-bail path: with a recent
        // cutoff most of the file should be irrelevant. We can't measure
        // bytes-read from a unit test directly, but we can at least verify
        // the result is correct (only the post-cutoff messages, none of
        // the older ones) without any panics from incomplete carry handling.
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let base = Utc::now() - Duration::hours(1);
        // 200 medium-sized messages, ascending timestamps.
        let payload = "y".repeat(2048);
        for i in 0..200u32 {
            let ts = (base + Duration::seconds(i as i64)).to_rfc3339();
            let mut msg = make_msg(&format!("m{i}"), &ts);
            msg.content = payload.clone();
            storage.append_message(&sid, &msg).await.unwrap();
        }
        // Cut at message 195 — expect 4 results (m196..m199).
        let cutoff = (base + Duration::seconds(195)).to_rfc3339();
        let got = storage.get_messages_after(&sid, &cutoff).await.unwrap();
        assert_eq!(got.len(), 4);
        assert_eq!(got[0].id, "m196");
        assert_eq!(got[3].id, "m199");
    }

    #[tokio::test]
    async fn get_messages_after_truncates_oversized_content_per_message() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now();
        let ts = now.to_rfc3339();
        let mut huge = make_msg("huge", &ts);
        // 1 MiB content — well past the per-message cap.
        huge.content = "x".repeat(1024 * 1024);
        storage.append_message(&sid, &huge).await.unwrap();

        let got = storage.get_messages_after(&sid, "").await.unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].content.len() < 64 * 1024);
        assert!(got[0].content.contains("truncated for catchup"));
        assert!(got[0].content.contains(&format!("{}", 1024 * 1024)));
    }

    #[tokio::test]
    async fn gc_keeps_everything_when_nothing_is_old() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now();
        let recent = (now - Duration::days(2)).to_rfc3339();

        storage
            .append_message(&sid, &make_msg("r1", &recent))
            .await
            .unwrap();
        storage
            .append_message(&sid, &make_msg("r2", &recent))
            .await
            .unwrap();

        let (kept, dropped, freed) = storage
            .gc_session_before(&sid, now - Duration::days(30))
            .await
            .unwrap();
        assert_eq!(kept, 2);
        assert_eq!(dropped, 0);
        assert_eq!(freed, 0);

        let survivors = storage.get_messages(&sid).await.unwrap();
        assert_eq!(survivors.len(), 2);
        assert!(!storage
            .messages_file(&sid)
            .with_extension("jsonl.tmp")
            .exists());
        assert_eq!(
            session_tmp_files(&storage, &sid).await,
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn gc_keeps_unparseable_lines_defensively() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let now = Utc::now();
        let recent = (now - Duration::days(2)).to_rfc3339();

        storage
            .append_message(&sid, &make_msg("r1", &recent))
            .await
            .unwrap();
        // Inject a malformed line — a parser regression must not silently
        // delete it. Append directly to bypass JSON serialization.
        let path = storage.messages_file(&sid);
        let mut f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut f, b"this is not json\n")
            .await
            .unwrap();
        drop(f);

        let (kept, dropped, _) = storage
            .gc_session_before(&sid, now - Duration::days(30))
            .await
            .unwrap();
        assert_eq!(kept, 2);
        assert_eq!(dropped, 0);
        let raw = tokio::fs::read_to_string(storage.messages_file(&sid))
            .await
            .unwrap();
        assert!(raw.contains("this is not json"));
    }

    #[tokio::test]
    async fn gc_no_op_on_missing_file_is_zero() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let (kept, dropped, freed) = storage.gc_session_before(&sid, Utc::now()).await.unwrap();
        assert_eq!((kept, dropped, freed), (0, 0, 0));
    }

    #[tokio::test]
    async fn gc_all_aggregates_stats_across_sessions() {
        let storage = fresh_storage();
        let now = Utc::now();
        let old = (now - Duration::days(45)).to_rfc3339();
        let recent = (now - Duration::days(2)).to_rfc3339();

        let sid_a = Uuid::new_v4();
        storage
            .append_message(&sid_a, &make_msg("a1", &old))
            .await
            .unwrap();
        storage
            .append_message(&sid_a, &make_msg("a2", &recent))
            .await
            .unwrap();

        let sid_b = Uuid::new_v4();
        storage
            .append_message(&sid_b, &make_msg("b1", &recent))
            .await
            .unwrap();

        let stats = storage
            .gc_all_sessions_before(now - Duration::days(30))
            .await
            .unwrap();
        assert_eq!(stats.sessions_scanned, 2);
        assert_eq!(stats.sessions_modified, 1);
        assert_eq!(stats.messages_kept, 2);
        assert_eq!(stats.messages_dropped, 1);
    }

    #[tokio::test]
    async fn session_directory_deletion_drains_an_earlier_append_and_is_idempotent() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let lock = storage.session_lock(&sid);
        let held = lock.lock().await;

        // Tokio mutex waiters are FIFO: queue the append first and deletion
        // second, then prove cleanup removes what the in-flight append wrote.
        let append_storage = storage.clone();
        let append = tokio::spawn(async move {
            append_storage
                .append_message(&sid, &make_msg("in-flight", &Utc::now().to_rfc3339()))
                .await
        });
        tokio::task::yield_now().await;
        let delete_storage = storage.clone();
        let delete = tokio::spawn(async move { delete_storage.delete_session_dirs(&[sid]).await });
        tokio::task::yield_now().await;
        drop(held);
        drop(lock);

        append.await.unwrap().unwrap();
        delete.await.unwrap().unwrap();
        assert!(!storage.session_dir(&sid).exists());
        storage.delete_session_dirs(&[sid]).await.unwrap();
        assert!(!storage
            .session_locks
            .lock()
            .expect("session_locks poisoned")
            .contains_key(&sid));
    }
}

#[cfg(test)]
mod pane_work_summary_storage_tests {
    use super::{FileStorage, PaneWorkSummaryDocument, StoredMessage};
    use chrono::{Duration, Utc};
    use shared::{
        PaneWorkSummary, PaneWorkSummaryStatus, PaneWorkSummaryWindowKind,
        PANE_WORK_SUMMARY_PROTOCOL_VERSION,
    };
    use uuid::Uuid;

    fn fresh_storage() -> FileStorage {
        FileStorage::new(
            std::env::temp_dir().join(format!("apas-summary-storage-{}", Uuid::new_v4())),
        )
    }

    fn summary(session_id: Uuid, pane_id: u32) -> PaneWorkSummary {
        let start = Utc::now() - Duration::hours(3);
        PaneWorkSummary {
            protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            session_id,
            pane_id,
            window_start: start,
            window_end: start + Duration::hours(3),
            window_kind: PaneWorkSummaryWindowKind::Completed,
            status: PaneWorkSummaryStatus::Complete,
            summary: Some(format!("summary for pane {pane_id}")),
            source_digest: format!("digest-{pane_id}"),
            source_message_count: 2,
            source_through: Some(start + Duration::minutes(2)),
            source_through_id: Some("m2".to_string()),
            generated_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            provider: Some("claude".to_string()),
            model: None,
            attempts: 1,
            error: None,
        }
    }

    #[tokio::test]
    async fn sidecar_is_atomic_and_survives_message_gc_and_pane_closure() {
        let storage = fresh_storage();
        let session_id = Uuid::new_v4();
        let document = PaneWorkSummaryDocument {
            version: 1,
            summaries: vec![summary(session_id, 4)],
        };
        storage
            .save_pane_work_summaries(&session_id, &document)
            .await
            .unwrap();
        storage
            .append_message(
                &session_id,
                &StoredMessage {
                    id: "old".to_string(),
                    role: "assistant".to_string(),
                    content: "source".to_string(),
                    message_type: "text".to_string(),
                    created_at: (Utc::now() - Duration::days(9)).to_rfc3339(),
                    pane_type: Some("4".to_string()),
                },
            )
            .await
            .unwrap();
        storage
            .gc_session_before(&session_id, Utc::now() - Duration::days(7))
            .await
            .unwrap();
        storage.save_pane_list(&session_id, &[]).await.unwrap();

        assert_eq!(
            storage.load_pane_work_summaries(&session_id).await.unwrap(),
            document
        );
        let raw = tokio::fs::read_to_string(storage.pane_work_summaries_file(&session_id))
            .await
            .unwrap();
        assert!(raw.contains("summary for pane 4"));
    }

    #[tokio::test]
    async fn concurrent_updates_do_not_lose_records() {
        let storage = fresh_storage();
        let session_id = Uuid::new_v4();
        let mut tasks = Vec::new();
        for pane_id in 1..=8 {
            let storage = storage.clone();
            tasks.push(tokio::spawn(async move {
                storage
                    .update_pane_work_summaries(&session_id, |document| {
                        document.summaries.push(summary(session_id, pane_id));
                        Ok(())
                    })
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let document = storage.load_pane_work_summaries(&session_id).await.unwrap();
        assert_eq!(document.summaries.len(), 8);
    }

    #[tokio::test]
    async fn corruption_is_reported_without_touching_messages_and_deletion_removes_sidecar() {
        let storage = fresh_storage();
        let session_id = Uuid::new_v4();
        storage.ensure_session_dir(&session_id).await.unwrap();
        tokio::fs::write(storage.pane_work_summaries_file(&session_id), b"not json")
            .await
            .unwrap();
        storage
            .append_message(
                &session_id,
                &StoredMessage {
                    id: "keep".to_string(),
                    role: "user".to_string(),
                    content: "keep me".to_string(),
                    message_type: "text".to_string(),
                    created_at: Utc::now().to_rfc3339(),
                    pane_type: Some("2".to_string()),
                },
            )
            .await
            .unwrap();

        assert!(storage.load_pane_work_summaries(&session_id).await.is_err());
        assert_eq!(storage.get_messages(&session_id).await.unwrap().len(), 1);
        storage.delete_session_dirs(&[session_id]).await.unwrap();
        assert!(!storage.session_dir(&session_id).exists());
    }

    #[tokio::test]
    async fn restart_requeues_in_progress_records() {
        let storage = fresh_storage();
        let session_id = Uuid::new_v4();
        let mut record = summary(session_id, 2);
        record.status = PaneWorkSummaryStatus::Generating;
        storage
            .save_pane_work_summaries(
                &session_id,
                &PaneWorkSummaryDocument {
                    version: 1,
                    summaries: vec![record],
                },
            )
            .await
            .unwrap();
        let recovered = storage
            .recover_pane_work_summaries(&session_id)
            .await
            .unwrap();
        assert_eq!(recovered.summaries[0].status, PaneWorkSummaryStatus::Queued);
    }
}

#[cfg(test)]
mod tests {
    use super::{normalized_bucket_key, parse_stored_pane_id};

    #[test]
    fn parse_stored_pane_id_handles_legacy_numeric_and_composite_formats() {
        assert_eq!(
            parse_stored_pane_id(Some("deadloop")),
            Some(shared::PANE_ID_DEADLOOP)
        );
        assert_eq!(
            parse_stored_pane_id(Some("interactive")),
            Some(shared::PANE_ID_INTERACTIVE)
        );
        assert_eq!(parse_stored_pane_id(Some("939")), Some(939));
        assert_eq!(
            parse_stored_pane_id(Some("claude-interactive-1")),
            Some(shared::PANE_ID_INTERACTIVE)
        );
        assert_eq!(
            parse_stored_pane_id(Some("codex-deadloop-7")),
            Some(shared::PANE_ID_DEADLOOP)
        );
        assert_eq!(parse_stored_pane_id(Some("pane-42")), Some(42));
        assert_eq!(parse_stored_pane_id(Some("unknown")), None);
        assert_eq!(parse_stored_pane_id(None), None);
    }

    #[test]
    fn normalized_bucket_key_collapses_equivalent_pane_identifiers() {
        assert_eq!(
            normalized_bucket_key(Some("interactive")),
            Some("2".to_string())
        );
        assert_eq!(normalized_bucket_key(Some("2")), Some("2".to_string()));
        assert_eq!(
            normalized_bucket_key(Some("claude-interactive-1")),
            Some("2".to_string())
        );
    }
}
