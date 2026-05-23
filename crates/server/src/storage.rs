use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

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

#[derive(Clone)]
pub struct FileStorage {
    base_path: PathBuf,
    /// Per-session locks shared between message appends and the periodic GC
    /// task. Without this an `append_message` mid-write would race with the
    /// GC's atomic rename — the append handle would land on the orphaned
    /// pre-rename inode and the message would silently vanish.
    session_locks: Arc<StdMutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>>,
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
        }
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
    pub async fn get_messages_after(
        &self,
        session_id: &Uuid,
        after_created_at: &str,
    ) -> Result<Vec<StoredMessage>> {
        const CATCHUP_LIMIT: usize = 5000;
        let file_path = self.messages_file(session_id);
        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut newer = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => {
                    if msg.created_at.as_str() > after_created_at {
                        newer.push(msg);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }
        // RFC3339 timestamps are lexicographically ordered, but the file may
        // contain late-write reorderings on the microsecond scale — sort to
        // hand the client a clean ASC tail.
        newer.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        if newer.len() > CATCHUP_LIMIT {
            let drop = newer.len() - CATCHUP_LIMIT;
            tracing::warn!(
                "Catchup for session {} truncated: {} messages exceeded cap, dropping oldest {}",
                session_id,
                newer.len(),
                drop
            );
            newer = newer.split_off(drop);
        }
        Ok(newer)
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
    pub async fn get_messages_per_pane(
        &self,
        session_id: &Uuid,
        limit_per_pane: usize,
    ) -> Result<(Vec<StoredMessage>, bool)> {
        let file_path = self.messages_file(session_id);

        if !file_path.exists() {
            return Ok((Vec::new(), false));
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        // Dynamic pane bucketing using HashMap instead of hardcoded categories
        let mut pane_buckets: std::collections::HashMap<Option<String>, Vec<StoredMessage>> =
            std::collections::HashMap::new();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => {
                    // Normalize pane identifiers so legacy and numeric representations
                    // of the same pane collapse into a single bucket.
                    let bucket_key = normalized_bucket_key(msg.pane_type.as_deref());
                    pane_buckets.entry(bucket_key).or_default().push(msg);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }

        // Check if there are more messages than we're returning
        let has_more = pane_buckets
            .values()
            .any(|msgs| msgs.len() > limit_per_pane);

        // Take the most recent N messages from each bucket
        let mut combined = Vec::new();
        for msgs in pane_buckets.values() {
            let recent: Vec<_> = if msgs.len() > limit_per_pane {
                msgs[msgs.len() - limit_per_pane..].to_vec()
            } else {
                msgs.clone()
            };
            combined.extend(recent);
        }

        // Sort by created_at timestamp
        combined.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        Ok((combined, has_more))
    }

    /// Drop every message with `created_at < cutoff` from a session's
    /// messages.jsonl. Reads the file, filters lines whose timestamp falls
    /// before the cutoff, writes the survivors to `messages.jsonl.tmp`, then
    /// atomically renames over the original. Holds the per-session lock for
    /// the whole operation so concurrent appends queue cleanly.
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

        let tmp_path = file_path.with_extension("jsonl.tmp");
        let mut tmp = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
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

        if dropped == 0 {
            // Nothing to rewrite — leave the original alone.
            let _ = fs::remove_file(&tmp_path).await;
            return Ok((kept, 0, 0));
        }

        fs::rename(&tmp_path, &file_path).await?;
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
                    tracing::warn!(
                        "GC failed for session {}: {} — leaving file intact",
                        sid,
                        e
                    );
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
    use super::{FileStorage, StoredMessage};
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
    }

    #[tokio::test]
    async fn gc_no_op_on_missing_file_is_zero() {
        let storage = fresh_storage();
        let sid = Uuid::new_v4();
        let (kept, dropped, freed) = storage
            .gc_session_before(&sid, Utc::now())
            .await
            .unwrap();
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
