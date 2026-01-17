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
        let file_path = self.messages_file(session_id);

        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut messages = Vec::new();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<StoredMessage>(&line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!("Failed to parse message line: {}", e);
                }
            }
        }

        // If limit specified, return only the most recent messages
        if let Some(limit) = limit {
            if messages.len() > limit {
                messages = messages.split_off(messages.len() - limit);
            }
        }

        Ok(messages)
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
