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

    /// Read all messages for a session (with optional limit for recent messages)
    pub async fn get_messages(&self, session_id: &Uuid) -> Result<Vec<StoredMessage>> {
        self.get_messages_with_limit(session_id, None).await
    }

    /// Read messages for a session, optionally limited to the most recent N
    pub async fn get_messages_with_limit(&self, session_id: &Uuid, limit: Option<usize>) -> Result<Vec<StoredMessage>> {
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

    /// Read messages for a session, loading recent messages per pane type
    /// This ensures both deadloop and interactive messages are included
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

        let mut deadloop_messages = Vec::new();
        let mut interactive_messages = Vec::new();
        let mut other_messages = Vec::new();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => {
                    match msg.pane_type.as_deref() {
                        Some("deadloop") => deadloop_messages.push(msg),
                        Some("interactive") => interactive_messages.push(msg),
                        _ => other_messages.push(msg),
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }

        // Check if there are more messages than we're returning
        let has_more = deadloop_messages.len() > limit_per_pane
            || interactive_messages.len() > limit_per_pane
            || other_messages.len() > limit_per_pane;

        // Take the most recent N messages from each category
        let deadloop_recent: Vec<_> = if deadloop_messages.len() > limit_per_pane {
            deadloop_messages[deadloop_messages.len() - limit_per_pane..].to_vec()
        } else {
            deadloop_messages
        };

        let interactive_recent: Vec<_> = if interactive_messages.len() > limit_per_pane {
            interactive_messages[interactive_messages.len() - limit_per_pane..].to_vec()
        } else {
            interactive_messages
        };

        let other_recent: Vec<_> = if other_messages.len() > limit_per_pane {
            other_messages[other_messages.len() - limit_per_pane..].to_vec()
        } else {
            other_messages
        };

        // Combine and sort by created_at
        let mut combined = Vec::new();
        combined.extend(deadloop_recent);
        combined.extend(interactive_recent);
        combined.extend(other_recent);

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
