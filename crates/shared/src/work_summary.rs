use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Provider;

/// Protocol capability shared by the desktop web client, server, and CLI.
pub const PANE_WORK_SUMMARY_CAPABILITY: &str = "pane_work_summary_v1";
pub const PANE_WORK_SUMMARY_PROTOCOL_VERSION: u32 = 1;

/// Whether the cached record covers a closed window or the still-open window.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneWorkSummaryWindowKind {
    #[default]
    Completed,
    Current,
}

/// Durable generation state for one pane/window/source digest.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneWorkSummaryStatus {
    #[default]
    Queued,
    Generating,
    Complete,
    Partial,
    Stale,
    Failed,
    SourceExpired,
}

/// Generation support reported alongside cached records.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneWorkSummaryAvailability {
    Available,
    /// The selected session is owned by a CLI predating this capability.
    CliUpdateRequired,
    /// The CLI supports the protocol but its isolated adapter is disabled.
    SummarizerDisabled,
    /// The configured adapter is temporarily unable to accept generation.
    SummarizerUnavailable,
    /// A legacy or partially upgraded peer did not report availability.
    #[default]
    Unknown,
}

/// One cached summary record. Source text and intermediate notes are never
/// persisted in this record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct PaneWorkSummary {
    #[serde(default = "pane_work_summary_protocol_version")]
    pub protocol_version: u32,
    pub session_id: Uuid,
    pub pane_id: u32,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    #[serde(default)]
    pub window_kind: PaneWorkSummaryWindowKind,
    #[serde(default)]
    pub status: PaneWorkSummaryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub source_digest: String,
    #[serde(default)]
    pub source_message_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_through: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_through_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneWorkSummaryStage {
    #[default]
    Notes,
    Final,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneWorkSummaryResultKind {
    #[default]
    Success,
    RetryableFailure,
    PermanentFailure,
    Unavailable,
}

/// Bounded work item dispatched by the server to the session-owning CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct PaneWorkSummaryGenerationJob {
    #[serde(default = "pane_work_summary_protocol_version")]
    pub protocol_version: u32,
    pub job_id: Uuid,
    pub session_id: Uuid,
    pub pane_id: u32,
    #[serde(default)]
    pub pane_provider: Provider,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub source_digest: String,
    #[serde(default)]
    pub stage: PaneWorkSummaryStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u32>,
    pub content: String,
    #[serde(default)]
    pub correction_attempt: bool,
}

/// Correlated result returned by the isolated CLI summarizer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct PaneWorkSummaryGenerationResult {
    #[serde(default = "pane_work_summary_protocol_version")]
    pub protocol_version: u32,
    pub job_id: Uuid,
    pub session_id: Uuid,
    pub pane_id: u32,
    pub window_start: DateTime<Utc>,
    pub source_digest: String,
    #[serde(default)]
    pub stage: PaneWorkSummaryStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_index: Option<u32>,
    #[serde(default)]
    pub kind: PaneWorkSummaryResultKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

const fn pane_work_summary_protocol_version() -> u32 {
    PANE_WORK_SUMMARY_PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_record_fields_receive_rollout_safe_defaults() {
        let session_id = Uuid::new_v4();
        let json = format!(
            r#"{{"session_id":"{session_id}","pane_id":2,"window_start":"2026-08-11T03:00:00Z","window_end":"2026-08-11T06:00:00Z"}}"#
        );

        let record: PaneWorkSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(record.protocol_version, PANE_WORK_SUMMARY_PROTOCOL_VERSION);
        assert_eq!(record.status, PaneWorkSummaryStatus::Queued);
        assert_eq!(record.window_kind, PaneWorkSummaryWindowKind::Completed);
        assert_eq!(record.source_message_count, 0);
    }

    #[test]
    fn staged_job_and_result_round_trip_with_correlation() {
        let job = PaneWorkSummaryGenerationJob {
            protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            job_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            pane_id: 7,
            pane_provider: Provider::Codex,
            window_start: "2026-08-11T03:00:00Z".parse().unwrap(),
            window_end: "2026-08-11T06:00:00Z".parse().unwrap(),
            source_digest: "abc123".to_string(),
            stage: PaneWorkSummaryStage::Notes,
            chunk_index: Some(1),
            chunk_count: Some(3),
            content: "bounded source".to_string(),
            correction_attempt: false,
        };
        let decoded: PaneWorkSummaryGenerationJob =
            serde_json::from_str(&serde_json::to_string(&job).unwrap()).unwrap();
        assert_eq!(decoded, job);

        let result = PaneWorkSummaryGenerationResult {
            protocol_version: PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            job_id: job.job_id,
            session_id: job.session_id,
            pane_id: job.pane_id,
            window_start: job.window_start,
            source_digest: job.source_digest,
            stage: job.stage,
            chunk_index: job.chunk_index,
            kind: PaneWorkSummaryResultKind::Success,
            output: Some("grounded notes".to_string()),
            error: None,
            provider: Some("claude".to_string()),
            model: None,
        };
        let decoded: PaneWorkSummaryGenerationResult =
            serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();
        assert_eq!(decoded, result);
    }
}
