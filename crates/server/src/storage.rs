use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
}

impl FileStorage {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
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
mod tests {
    use super::{normalized_bucket_key, parse_stored_pane_id};

    #[test]
    fn parse_stored_pane_id_handles_legacy_numeric_and_composite_formats() {
        assert_eq!(parse_stored_pane_id(Some("deadloop")), Some(shared::PANE_ID_DEADLOOP));
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
        assert_eq!(normalized_bucket_key(Some("interactive")), Some("2".to_string()));
        assert_eq!(normalized_bucket_key(Some("2")), Some("2".to_string()));
        assert_eq!(
            normalized_bucket_key(Some("claude-interactive-1")),
            Some("2".to_string())
        );
    }
}
