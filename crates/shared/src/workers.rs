//! Persistent responsibilities above the bounded autonomous-job execution engine.
//! Model-authored results are proposals; they never authorize execution or confirm delivery.

use crate::jobs::{
    EffectiveJobPolicy, JobDetail, JobLimits, JobPolicy, JobReceiptStatus, JobRun, JobTarget,
    JobUsage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const AUTONOMOUS_WORKERS_CAPABILITY: &str = "autonomous_workers_v1";
pub const OMP_PERSISTENT_WORKER_CAPABILITY: &str = "omp_persistent_worker_v1";
pub const WORKER_RESULT_VERSION: u32 = 1;
pub const WORKER_EXECUTION_PROTOCOL_VERSION: u32 = 2;
pub const WORKER_BUDGET_WINDOW_SECONDS: u64 = 86_400;
pub const WORKER_MAX_RESULT_BYTES: usize = 128 * 1024;
pub const WORKER_MAX_CONTEXT_BYTES: usize = 256 * 1024;
pub const WORKER_MAX_BACKLOG_ITEMS: usize = 100;
pub const WORKER_RESULT_FILENAME: &str = "apas-worker-result.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecurringWorkerPolicy {
    pub enabled: bool,
    pub max_elapsed_seconds_24h: u64,
    pub max_spend_micro_usd_24h: u64,
    pub max_runs_24h: u32,
    pub min_interval_seconds: u64,
}

impl Default for RecurringWorkerPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_elapsed_seconds_24h: 7200,
            max_spend_micro_usd_24h: 20_000_000,
            max_runs_24h: 24,
            min_interval_seconds: 900,
        }
    }
}

impl RecurringWorkerPolicy {
    pub fn is_finite_positive(&self) -> bool {
        self.max_elapsed_seconds_24h > 0
            && self.max_spend_micro_usd_24h > 0
            && self.max_runs_24h > 0
            && self.min_interval_seconds > 0
    }

    pub fn fits_within(&self, bound: &Self) -> bool {
        self.is_finite_positive()
            && (!self.enabled || bound.enabled)
            && self.max_elapsed_seconds_24h <= bound.max_elapsed_seconds_24h
            && self.max_spend_micro_usd_24h <= bound.max_spend_micro_usd_24h
            && self.max_runs_24h <= bound.max_runs_24h
            && self.min_interval_seconds >= bound.min_interval_seconds
    }

    pub fn narrowed_by(&self, other: &Self) -> Self {
        Self {
            enabled: self.enabled && other.enabled,
            max_elapsed_seconds_24h: self
                .max_elapsed_seconds_24h
                .min(other.max_elapsed_seconds_24h),
            max_spend_micro_usd_24h: self
                .max_spend_micro_usd_24h
                .min(other.max_spend_micro_usd_24h),
            max_runs_24h: self.max_runs_24h.min(other.max_runs_24h),
            min_interval_seconds: self.min_interval_seconds.max(other.min_interval_seconds),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerLimits {
    pub per_run: JobLimits,
    pub elapsed_seconds_24h: u64,
    pub spend_micro_usd_24h: u64,
    pub runs_24h: u32,
    pub attempts_per_item: u32,
}

impl WorkerLimits {
    pub fn fits_within(&self, policy: &JobPolicy) -> bool {
        self.per_run.fits_within(&policy.limits)
            && self.per_run.attempts == 1
            && self.elapsed_seconds_24h >= self.per_run.elapsed_seconds
            && self.elapsed_seconds_24h <= policy.recurring.max_elapsed_seconds_24h
            && self.spend_micro_usd_24h >= self.per_run.spend_micro_usd
            && self.spend_micro_usd_24h <= policy.recurring.max_spend_micro_usd_24h
            && self.runs_24h > 0
            && self.runs_24h <= policy.recurring.max_runs_24h
            && self.attempts_per_item > 0
            && self.attempts_per_item <= policy.limits.attempts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerDesiredState {
    Active,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Queued,
    Preparing,
    Inspecting,
    Working,
    NeedsInput,
    Verifying,
    Publishing,
    Idle,
    WaitingReview,
    WaitingBudget,
    Pausing,
    Paused,
    Stopping,
    Blocked,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerMandateSpec {
    pub title: String,
    pub description: String,
    pub checklist: Vec<String>,
    pub machine_id: Uuid,
    pub worker_profile: String,
    pub verification_profile: String,
    pub model_profile: String,
    pub base_ref: String,
    pub interval_seconds: u64,
    pub limits: WorkerLimits,
}

impl WorkerMandateSpec {
    pub fn valid_text(&self) -> bool {
        bounded_text(&self.title, 200)
            && bounded_text(&self.description, 16_384)
            && !self.checklist.is_empty()
            && self.checklist.len() <= 32
            && self.checklist.iter().all(|s| bounded_text(s, 2048))
            && bounded_text(&self.base_ref, 256)
            && !self.base_ref.starts_with('-')
            && [
                &self.worker_profile,
                &self.verification_profile,
                &self.model_profile,
            ]
            .iter()
            .all(|s| bounded_text(s, 128))
            && self.interval_seconds > 0
            && !self.machine_id.is_nil()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerMandate {
    pub revision: u64,
    pub spec: WorkerMandateSpec,
    pub repository: String,
    pub worker_profile_digest: String,
    pub verification_profile_digest: String,
    pub policy_digest: String,
    pub approved_by: Uuid,
    pub approved_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerConversionSource {
    Job { job_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerPreviewRequest {
    #[serde(default)]
    pub worker_id: Option<Uuid>,
    #[serde(default)]
    pub expected_version: Option<u64>,
    pub spec: WorkerMandateSpec,
    #[serde(default)]
    pub source: Option<WorkerConversionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerPreview {
    pub id: Uuid,
    pub project_id: String,
    pub worker_id: Option<Uuid>,
    pub expected_version: Option<u64>,
    pub fingerprint: String,
    pub expires_at: String,
    pub mandate: WorkerMandate,
    pub cluster_owner_user_id: Uuid,
    pub hostname: String,
    pub base_sha: String,
    pub policy: EffectiveJobPolicy,
    pub source: Option<WorkerConversionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerApprovalRequest {
    pub request_id: Uuid,
    pub preview_id: Uuid,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkerAllowedActions {
    pub edit_mandate: bool,
    pub edit_backlog: bool,
    pub pause: bool,
    pub stop: bool,
    pub resume: bool,
    pub wake: bool,
    pub retry: bool,
    pub answer: bool,
    pub retry_publication: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerSummary {
    pub id: Uuid,
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub state: WorkerState,
    pub reason: Option<String>,
    pub desired_state: WorkerDesiredState,
    pub mandate_revision: u64,
    pub backlog_revision: u64,
    pub state_version: u64,
    pub high_watermark: u64,
    pub next_check_at: Option<String>,
    pub active_job_id: Option<Uuid>,
    pub active_run_id: Option<Uuid>,
    pub attention_run_id: Option<Uuid>,
    pub attention_question_id: Option<Uuid>,
    pub current_item_id: Option<Uuid>,
    pub machine_id: Uuid,
    pub cluster_owner_user_id: Uuid,
    pub hostname: String,
    pub attention: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkerBudget {
    pub usage: JobUsage,
    pub runs: u32,
    pub reserved_elapsed_seconds: u64,
    pub reserved_spend_micro_usd: u64,
    pub reserved_runs: u32,
    pub accounting_uncertain: bool,
    pub eligible_at: Option<String>,
    pub window_started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerDetail {
    pub worker: WorkerSummary,
    pub mandate: WorkerMandate,
    pub checkpoint: Option<WorkerCheckpoint>,
    pub budget: WorkerBudget,
    pub allowed_actions: WorkerAllowedActions,
    pub current_job: Option<JobDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerListResponse {
    pub workers: Vec<WorkerSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerTargetsResponse {
    pub project_id: String,
    pub targets: Vec<JobTarget>,
    pub unavailable_reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerMutationKind {
    Approve,
    EditMandate,
    Pause,
    Stop,
    Resume,
    Wake,
    UpsertItem,
    DismissItem,
    RetryItem,
    Answer,
    RetryPublication,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerMutationReceipt {
    pub request_id: Uuid,
    pub operation: WorkerMutationKind,
    pub status: JobReceiptStatus,
    pub worker_id: Option<Uuid>,
    pub project_id: String,
    pub state_version: Option<u64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerMutationRequest {
    pub request_id: Uuid,
    pub expected_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerItemState {
    Ready,
    Selected,
    Running,
    AwaitingReview,
    Done,
    Blocked,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerItemOrigin {
    Human,
    Inspection,
    Implementation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerItemDraft {
    pub title: String,
    pub instruction: String,
    pub acceptance_criteria: Vec<String>,
    pub scope_rationale: String,
    pub evidence: Vec<String>,
    pub priority: i32,
    pub dependencies: Vec<Uuid>,
}

impl WorkerItemDraft {
    pub fn is_valid(&self) -> bool {
        bounded_text(&self.title, 200)
            && bounded_text(&self.instruction, 8192)
            && bounded_text(&self.scope_rationale, 2048)
            && !self.acceptance_criteria.is_empty()
            && self.acceptance_criteria.len() <= 16
            && self
                .acceptance_criteria
                .iter()
                .all(|s| bounded_text(s, 1024))
            && self.evidence.len() <= 16
            && self.evidence.iter().all(|s| bounded_text(s, 1024))
            && self.dependencies.len() <= WORKER_MAX_BACKLOG_ITEMS
            && self.dependencies.iter().all(|id| !id.is_nil())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerItem {
    pub id: Uuid,
    pub worker_id: Uuid,
    pub revision: u64,
    pub item: WorkerItemDraft,
    pub origin: WorkerItemOrigin,
    pub state: WorkerItemState,
    pub attempts: u32,
    pub job_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub outcome: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerItemsResponse {
    pub items: Vec<WorkerItem>,
    pub backlog_revision: u64,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerItemRequest {
    pub request_id: Uuid,
    pub expected_version: u64,
    pub item_id: Uuid,
    pub expected_item_revision: Option<u64>,
    pub item: WorkerItemDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRunPurpose {
    Inspection,
    Implementation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerResultDisposition {
    SelectWork,
    NoWork,
    CandidateReady,
    Blocked,
    NoChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerChecklistObservation {
    pub index: u32,
    pub observation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerCheckpoint {
    pub id: Uuid,
    pub worker_id: Uuid,
    pub run_id: Uuid,
    pub mandate_revision: u64,
    pub backlog_revision: u64,
    pub base_sha: String,
    pub summary: String,
    pub checklist: Vec<WorkerChecklistObservation>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerItemProposal {
    pub id: Uuid,
    pub expected_revision: Option<u64>,
    pub item: WorkerItemDraft,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerResult {
    pub version: u32,
    pub worker_id: Uuid,
    pub mandate_revision: u64,
    pub backlog_revision: u64,
    pub schedule_generation: u64,
    pub reservation_id: Uuid,
    pub purpose: WorkerRunPurpose,
    pub disposition: WorkerResultDisposition,
    pub summary: String,
    pub checklist: Vec<WorkerChecklistObservation>,
    pub upserts: Vec<WorkerItemProposal>,
    pub selected_item_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerExecutionContext {
    pub worker_id: Uuid,
    pub mandate_revision: u64,
    pub backlog_revision: u64,
    pub schedule_generation: u64,
    pub reservation_id: Uuid,
    pub purpose: WorkerRunPurpose,
    pub mandate: WorkerMandateSpec,
    pub items: Vec<WorkerItem>,
    pub selected_item_id: Option<Uuid>,
    pub checkpoint: Option<WorkerCheckpoint>,
}

impl WorkerResult {
    pub fn validate(&self, context: &WorkerExecutionContext) -> Result<(), &'static str> {
        if self.version != WORKER_RESULT_VERSION
            || self.worker_id != context.worker_id
            || self.mandate_revision != context.mandate_revision
            || self.backlog_revision != context.backlog_revision
            || self.schedule_generation != context.schedule_generation
            || self.reservation_id != context.reservation_id
            || self.purpose != context.purpose
        {
            return Err("worker_result_identity_mismatch");
        }
        if !bounded_text(&self.summary, 8192)
            || self.checklist.len() > context.mandate.checklist.len()
            || self.checklist.iter().any(|c| {
                c.index as usize >= context.mandate.checklist.len()
                    || !bounded_text(&c.observation, 2048)
            })
            || self.upserts.len() > WORKER_MAX_BACKLOG_ITEMS
            || self
                .upserts
                .iter()
                .any(|p| p.id.is_nil() || !p.item.is_valid())
        {
            return Err("worker_result_invalid");
        }
        for (index, proposal) in self.upserts.iter().enumerate() {
            if self.upserts[..index]
                .iter()
                .any(|prior| prior.id == proposal.id)
            {
                return Err("worker_result_duplicate_item");
            }
        }
        for (index, observation) in self.checklist.iter().enumerate() {
            if self.checklist[..index]
                .iter()
                .any(|prior| prior.index == observation.index)
            {
                return Err("worker_result_duplicate_checklist_item");
            }
        }
        match (self.purpose, self.disposition) {
            (WorkerRunPurpose::Inspection, WorkerResultDisposition::SelectWork)
                if self.selected_item_id.is_some() => {}
            (
                WorkerRunPurpose::Inspection,
                WorkerResultDisposition::NoWork | WorkerResultDisposition::Blocked,
            ) if self.selected_item_id.is_none() => {}
            (
                WorkerRunPurpose::Implementation,
                WorkerResultDisposition::CandidateReady
                | WorkerResultDisposition::Blocked
                | WorkerResultDisposition::NoChange,
            ) if context.selected_item_id.is_some()
                && self.selected_item_id == context.selected_item_id => {}
            _ => return Err("worker_result_disposition_invalid"),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerRunSummary {
    pub worker_id: Uuid,
    pub item_id: Option<Uuid>,
    pub job_id: Uuid,
    pub purpose: WorkerRunPurpose,
    pub mandate_revision: u64,
    pub run: JobRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerRunsResponse {
    pub runs: Vec<WorkerRunSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkerEventKind {
    Approved,
    MandateChanged,
    StateChanged,
    BacklogChanged,
    Checkpoint,
    RunChanged,
    Question,
    Review,
    Budget,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerEvent {
    pub worker_id: Uuid,
    pub project_id: String,
    pub seq: u64,
    pub state_version: u64,
    pub kind: WorkerEventKind,
    pub summary: String,
    pub job_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub question_id: Option<Uuid>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerEventsResponse {
    pub events: Vec<WorkerEvent>,
    pub high_watermark: u64,
    pub has_more: bool,
    pub resync_required: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MobileWorkersBootstrap {
    pub enabled: bool,
    pub workers: Vec<WorkerSummary>,
    pub next_cursor: Option<String>,
}

fn bounded_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.contains('\0')
}
