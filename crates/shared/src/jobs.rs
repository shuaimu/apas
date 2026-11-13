//! Durable autonomous jobs are independent of interactive sessions and panes.
//! Public contracts contain profile identifiers, never credential values or host paths.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const AUTONOMOUS_JOBS_CAPABILITY: &str = "autonomous_jobs_v1";
pub const OMP_JOB_WORKER_CAPABILITY: &str = "omp_job_worker_v1";
pub const JOB_PROTOCOL_VERSION: u32 = 1;
pub const JOB_LEASE_SECONDS: u64 = 120;
pub const JOB_LEASE_RENEW_SECONDS: u64 = 20;
pub const JOB_READINESS_SECONDS: u64 = 300;
pub const JOB_MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Preparing,
    Running,
    NeedsInput,
    Verifying,
    Publishing,
    ReviewReady,
    /// Successful read-only worker inspection; never evidence of code delivery.
    Completed,
    Blocked,
    RecoveryRequired,
    Cancelling,
    Cancelled,
    Failed,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::ReviewReady | Self::Completed | Self::Cancelled | Self::Failed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobLimits {
    pub elapsed_seconds: u64,
    /// Reported-spend threshold, not a promise about the provider's final invoice.
    pub spend_micro_usd: u64,
    pub attempts: u32,
    pub cpu_millis: u32,
    pub memory_bytes: u64,
    pub workspace_bytes: u64,
    pub output_bytes: u64,
    pub pids: u32,
    pub model_concurrency: u32,
    pub model_output_tokens: u32,
}

impl Default for JobLimits {
    fn default() -> Self {
        Self {
            elapsed_seconds: 3600,
            spend_micro_usd: 10_000_000,
            attempts: 1,
            cpu_millis: 2000,
            memory_bytes: 4 * 1024 * 1024 * 1024,
            workspace_bytes: 8 * 1024 * 1024 * 1024,
            output_bytes: 100 * 1024 * 1024,
            pids: 256,
            model_concurrency: 2,
            model_output_tokens: 16_384,
        }
    }
}

impl JobLimits {
    pub fn is_finite_positive(&self) -> bool {
        self.elapsed_seconds > 0
            && self.spend_micro_usd > 0
            && self.attempts > 0
            && self.cpu_millis > 0
            && self.memory_bytes > 0
            && self.workspace_bytes > 0
            && self.output_bytes > 0
            && self.pids > 0
            && self.model_concurrency > 0
            && self.model_output_tokens > 0
    }

    pub fn fits_within(&self, bound: &Self) -> bool {
        self.is_finite_positive()
            && self.elapsed_seconds <= bound.elapsed_seconds
            && self.spend_micro_usd <= bound.spend_micro_usd
            && self.attempts <= bound.attempts
            && self.cpu_millis <= bound.cpu_millis
            && self.memory_bytes <= bound.memory_bytes
            && self.workspace_bytes <= bound.workspace_bytes
            && self.output_bytes <= bound.output_bytes
            && self.pids <= bound.pids
            && self.model_concurrency <= bound.model_concurrency
            && self.model_output_tokens <= bound.model_output_tokens
    }

    pub fn narrowed_by(&self, other: &Self) -> Self {
        Self {
            elapsed_seconds: self.elapsed_seconds.min(other.elapsed_seconds),
            spend_micro_usd: self.spend_micro_usd.min(other.spend_micro_usd),
            attempts: self.attempts.min(other.attempts),
            cpu_millis: self.cpu_millis.min(other.cpu_millis),
            memory_bytes: self.memory_bytes.min(other.memory_bytes),
            workspace_bytes: self.workspace_bytes.min(other.workspace_bytes),
            output_bytes: self.output_bytes.min(other.output_bytes),
            pids: self.pids.min(other.pids),
            model_concurrency: self.model_concurrency.min(other.model_concurrency),
            model_output_tokens: self.model_output_tokens.min(other.model_output_tokens),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobUsage {
    pub elapsed_seconds: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// None is unknown, not zero.
    pub spend_micro_usd: Option<u64>,
    pub estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobPolicy {
    pub enabled: bool,
    pub worker_profiles: Vec<String>,
    pub model_profiles: Vec<String>,
    pub verification_profiles: Vec<String>,
    pub publish_enabled: bool,
    pub limits: JobLimits,
    pub max_active_project: u32,
    pub max_active_machine: u32,
    pub max_active_cluster: u32,
    pub max_queued_project: u32,
    #[serde(default)]
    pub recurring: crate::workers::RecurringWorkerPolicy,
}

impl Default for JobPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            worker_profiles: Vec::new(),
            model_profiles: Vec::new(),
            verification_profiles: Vec::new(),
            publish_enabled: false,
            limits: JobLimits::default(),
            max_active_project: 1,
            max_active_machine: 1,
            max_active_cluster: 1,
            max_queued_project: 32,
            recurring: crate::workers::RecurringWorkerPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobPolicyOverride {
    pub enabled: Option<bool>,
    pub worker_profiles: Option<Vec<String>>,
    pub model_profiles: Option<Vec<String>>,
    pub verification_profiles: Option<Vec<String>>,
    pub publish_enabled: Option<bool>,
    pub limits: Option<JobLimits>,
    pub max_active_project: Option<u32>,
    pub max_active_machine: Option<u32>,
    pub max_active_cluster: Option<u32>,
    pub max_queued_project: Option<u32>,
    pub recurring: Option<crate::workers::RecurringWorkerPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobPolicySource {
    pub scope: String,
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EffectiveJobPolicy {
    pub policy: JobPolicy,
    pub digest: String,
    pub sources: Vec<JobPolicySource>,
    pub requested: Option<JobPolicyOverride>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct JobAllowedActions {
    pub admit: bool,
    pub answer: bool,
    pub cancel: bool,
    pub retry: bool,
    pub retry_publication: bool,
    pub manage_policy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobModelApi {
    OpenaiResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobModelProfile {
    pub key: String,
    pub label: String,
    pub model: String,
    pub api: JobModelApi,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobVerificationProfile {
    pub key: String,
    pub label: String,
    pub digest: String,
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobWorkerProfile {
    pub key: String,
    pub label: String,
    pub digest: String,
    pub image: String,
    pub omp_version: String,
    pub model_profiles: Vec<JobModelProfile>,
    pub verification_profiles: Vec<JobVerificationProfile>,
    pub limits: JobLimits,
    pub ready: bool,
    pub unavailable_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobWorkerReadiness {
    pub machine_id: Uuid,
    pub cluster_owner_user_id: Uuid,
    pub protocol_version: u32,
    pub checked_at: String,
    pub expires_at: String,
    pub profiles: Vec<JobWorkerProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobTarget {
    pub project_id: String,
    pub machine_id: Uuid,
    pub cluster_owner_user_id: Uuid,
    pub hostname: String,
    pub profiles: Vec<JobWorkerProfile>,
    pub available: bool,
    pub unavailable_reasons: Vec<String>,
    pub policy: EffectiveJobPolicy,
    pub allowed_actions: JobAllowedActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobTargetsResponse {
    pub project_id: String,
    pub targets: Vec<JobTarget>,
    pub unavailable_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobPreviewRequest {
    pub instruction: String,
    pub acceptance_criteria: Vec<String>,
    pub source_url: Option<String>,
    pub machine_id: Uuid,
    pub worker_profile: String,
    pub verification_profile: String,
    pub model_profile: String,
    pub base_ref: String,
    pub limits: JobLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobTaskSpec {
    pub instruction: String,
    pub acceptance_criteria: Vec<String>,
    pub source_url: Option<String>,
    pub repository: String,
    pub base_ref: String,
    pub base_sha: String,
    pub worker_profile: String,
    pub worker_profile_digest: String,
    pub verification_profile: String,
    pub verification_profile_digest: String,
    pub model_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<crate::workers::WorkerExecutionContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobPreview {
    pub id: Uuid,
    pub project_id: String,
    pub fingerprint: String,
    pub expires_at: String,
    pub task: JobTaskSpec,
    pub machine_id: Uuid,
    pub cluster_owner_user_id: Uuid,
    pub hostname: String,
    pub limits: JobLimits,
    pub policy_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobAdmissionRequest {
    pub request_id: Uuid,
    pub preview_id: Uuid,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobMutationKind {
    Admit,
    Answer,
    Cancel,
    Retry,
    RetryPublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobReceiptStatus {
    Pending,
    Accepted,
    Completed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobMutationReceipt {
    pub request_id: Uuid,
    pub operation: JobMutationKind,
    pub job_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub status: JobReceiptStatus,
    pub error_code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobMutationRequest {
    pub request_id: Uuid,
    pub expected_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobSummary {
    pub id: Uuid,
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    pub state: JobState,
    pub state_version: u64,
    pub current_run_id: Uuid,
    pub cluster_owner_user_id: Uuid,
    pub machine_id: Uuid,
    pub hostname: String,
    pub created_at: String,
    pub updated_at: String,
    pub usage: JobUsage,
    pub limits: JobLimits,
    pub attention_count: u32,
    pub reason: Option<String>,
    pub allowed_actions: JobAllowedActions,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobListResponse {
    pub jobs: Vec<JobSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectJobSummary {
    #[serde(flatten)]
    pub summary: JobSummary,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectJobListResponse {
    pub jobs: Vec<ProjectJobSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobStopEvidence {
    pub boot_id: String,
    pub container_id: String,
    pub stopped_at: String,
    pub reason: String,
    pub descendants_stopped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobRun {
    pub id: Uuid,
    pub job_id: Uuid,
    pub attempt: u32,
    pub epoch: u64,
    pub state: JobState,
    pub usage: JobUsage,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub worker_version: Option<String>,
    pub candidate_sha: Option<String>,
    pub stop_evidence: Option<JobStopEvidence>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobQuestionState {
    Pending,
    AnswerAccepted,
    AnswerDelivered,
    Confirmed,
    Uncertain,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobQuestionOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobQuestionItem {
    pub id: String,
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<JobQuestionOption>,
    pub multi: bool,
    pub recommended: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobQuestionAnswer {
    pub id: String,
    pub selected_option_ids: Vec<String>,
    pub custom_input: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobQuestion {
    pub id: Uuid,
    pub run_id: Uuid,
    pub epoch: u64,
    pub version: u64,
    pub dialog_id: String,
    pub tool_call_id: String,
    pub state: JobQuestionState,
    pub questions: Vec<JobQuestionItem>,
    pub answers: Option<Vec<JobQuestionAnswer>>,
    pub created_at: String,
    pub confirmed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobAnswerRequest {
    pub request_id: Uuid,
    pub run_id: Uuid,
    pub epoch: u64,
    pub expected_version: u64,
    pub answers: Vec<JobQuestionAnswer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobEventKind {
    Admitted,
    PhaseChanged,
    Progress,
    Subagent,
    Question,
    AnswerConfirmed,
    Usage,
    Verification,
    Publication,
    CancellationRequested,
    Stopped,
    Failed,
    RecoveryRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobEvent {
    pub id: Uuid,
    pub job_id: Uuid,
    pub run_id: Option<Uuid>,
    pub seq: u64,
    pub kind: JobEventKind,
    pub summary: String,
    pub created_at: String,
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobEventsResponse {
    pub events: Vec<JobEvent>,
    pub next_seq: u64,
    pub high_watermark: u64,
    pub has_more: bool,
    pub resync_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobDetail {
    pub job: JobSummary,
    pub task: JobTaskSpec,
    pub runs: Vec<JobRun>,
    pub questions: Vec<JobQuestion>,
    pub high_watermark: u64,
    pub policy_digest: String,
    pub publication: Option<JobPublication>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobArtifactKind {
    Candidate,
    Diff,
    CheckLog,
    WorkerLog,
    Verification,
    Publication,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobArtifact {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: JobArtifactKind,
    pub sha256: String,
    pub size_bytes: u64,
    pub content_type: String,
    pub expires_at: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobArtifactContent {
    pub artifact: JobArtifact,
    pub text: String,
    pub next_cursor: Option<String>,
    pub expired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobDiffFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    pub binary: bool,
    pub artifact_id: Option<Uuid>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobCheckStatus {
    Passed,
    Failed,
    TimedOut,
    Missing,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobCheckResult {
    pub name: String,
    pub argv: Vec<String>,
    pub status: JobCheckStatus,
    pub exit_code: Option<i32>,
    pub elapsed_seconds: u64,
    pub log_artifact_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobVerification {
    pub candidate_sha: String,
    pub base_sha: String,
    pub profile_digest: String,
    pub image_digest: String,
    pub checks: Vec<JobCheckResult>,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobPublication {
    pub repository: String,
    pub number: u64,
    pub url: String,
    pub branch: String,
    pub base_sha: String,
    pub head_sha: String,
    pub draft: bool,
    pub state: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobReview {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub base_sha: String,
    pub candidate_sha: Option<String>,
    pub files: Vec<JobDiffFile>,
    pub verification: Option<JobVerification>,
    pub publication: Option<JobPublication>,
    pub reason: Option<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobApiError {
    pub error: String,
    pub code: String,
    pub request_id: Option<Uuid>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MobileJobsBootstrap {
    pub available: bool,
    pub jobs: Vec<JobSummary>,
    pub next_cursor: Option<String>,
    pub unavailable_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobRunIdentity {
    pub project_id: String,
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub cluster_owner_user_id: Uuid,
    pub machine_id: Uuid,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum JobWorkerCommand {
    Prepare {
        command_id: Uuid,
        project_id: String,
        machine_id: Uuid,
        cluster_owner_user_id: Uuid,
        base_ref: String,
        worker_profile: String,
    },
    Start {
        command_id: Uuid,
        identity: JobRunIdentity,
        task: JobTaskSpec,
        limits: JobLimits,
        policy_digest: String,
        lease_expires_at: String,
    },
    RenewLease {
        command_id: Uuid,
        identity: JobRunIdentity,
        lease_expires_at: String,
        limits: JobLimits,
    },
    Answer {
        command_id: Uuid,
        identity: JobRunIdentity,
        question_id: Uuid,
        dialog_id: String,
        tool_call_id: String,
        answers: Vec<JobQuestionAnswer>,
    },
    Cancel {
        command_id: Uuid,
        identity: JobRunIdentity,
        reason: String,
    },
    Inventory {
        command_id: Uuid,
    },
    Verify {
        command_id: Uuid,
        identity: JobRunIdentity,
    },
    Publish {
        command_id: Uuid,
        identity: JobRunIdentity,
        candidate_sha: String,
    },
    /// Read external PR disposition without restarting execution or publishing.
    RefreshPublication {
        command_id: Uuid,
        identity: JobRunIdentity,
    },
    /// Delete a positively stopped run's local content for project deletion.
    /// Minimal identity, operation receipts and stop evidence remain as tombstones.
    PurgeRun {
        command_id: Uuid,
        identity: JobRunIdentity,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobRunInventory {
    pub identity: JobRunIdentity,
    pub state: JobState,
    pub boot_id: String,
    pub container_id: Option<String>,
    pub last_source_seq: u64,
    pub prompt_confirmed: bool,
    pub usage: JobUsage,
    pub stop_evidence: Option<JobStopEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobWorkerReport {
    Readiness {
        readiness: JobWorkerReadiness,
    },
    Prepared {
        command_id: Uuid,
        project_id: String,
        machine_id: Uuid,
        cluster_owner_user_id: Uuid,
        repository: String,
        base_ref: String,
        base_sha: String,
        profile: JobWorkerProfile,
    },
    Accepted {
        command_id: Uuid,
        identity: JobRunIdentity,
    },
    Rejected {
        command_id: Uuid,
        identity: Option<JobRunIdentity>,
        code: String,
        message: String,
    },
    Event {
        identity: JobRunIdentity,
        source_seq: u64,
        event_kind: JobEventKind,
        summary: String,
        detail: Option<Value>,
        usage: Option<JobUsage>,
    },
    Question {
        identity: JobRunIdentity,
        source_seq: u64,
        question: JobQuestion,
    },
    AnswerConfirmed {
        identity: JobRunIdentity,
        source_seq: u64,
        question: JobQuestion,
    },
    Settled {
        identity: JobRunIdentity,
        source_seq: u64,
        usage: JobUsage,
    },
    /// A stopped execution's proposed durable checkpoint, not verified delivery.
    WorkerResult {
        identity: JobRunIdentity,
        source_seq: u64,
        result: crate::workers::WorkerResult,
        evidence: JobStopEvidence,
        usage: JobUsage,
    },
    Stopped {
        identity: JobRunIdentity,
        source_seq: u64,
        evidence: JobStopEvidence,
        usage: JobUsage,
    },
    Inventory {
        command_id: Uuid,
        machine_id: Uuid,
        cluster_owner_user_id: Uuid,
        runs: Vec<JobRunInventory>,
    },
    ArtifactChunk {
        identity: JobRunIdentity,
        artifact: JobArtifact,
        offset: u64,
        data_base64: String,
        final_chunk: bool,
    },
    Verified {
        command_id: Uuid,
        identity: JobRunIdentity,
        review: JobReview,
    },
    Published {
        command_id: Uuid,
        identity: JobRunIdentity,
        publication: JobPublication,
    },
    PublicationObserved {
        command_id: Uuid,
        identity: JobRunIdentity,
        publication: Option<JobPublication>,
    },
    Purged {
        command_id: Uuid,
        identity: JobRunIdentity,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum JobPolicyDocument {
    Deployment(JobPolicy),
    Override(JobPolicyOverride),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobPolicyManagement {
    pub requested: Option<JobPolicyDocument>,
    pub effective: Option<EffectiveJobPolicy>,
    pub inherited: Option<EffectiveJobPolicy>,
    pub unavailable_reasons: Vec<String>,
    pub can_manage: bool,
}
