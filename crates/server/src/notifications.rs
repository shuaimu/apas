use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::{
    config::MobilePushConfig, db::MobileNotificationDeliveryRecord, mobile_metrics::MobileMetric,
    state::AppState,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GenericPushData {
    pub category: String,
    pub routing_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushMessage {
    pub to: String,
    pub title: &'static str,
    pub body: &'static str,
    pub data: GenericPushData,
    pub sound: Option<String>,
    pub ttl: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Ticket(String),
    TransientFailure(String),
    PermanentInvalidToken(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptOutcome {
    Delivered,
    Pending,
    TransientFailure(String),
    PermanentInvalidToken(String),
}

pub trait PushTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        messages: &'a [PushMessage],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PushOutcome>>> + Send + 'a>>;

    fn receipts<'a>(
        &'a self,
        ticket_ids: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<HashMap<String, ReceiptOutcome>>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct ExpoPushTransport {
    client: reqwest::Client,
    config: MobilePushConfig,
}

impl ExpoPushTransport {
    pub fn new(config: MobilePushConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            config,
        })
    }

    fn request(&self, url: &str) -> reqwest::RequestBuilder {
        let request = self.client.post(url);
        if let Some(token) = self.config.access_token.as_deref() {
            request.bearer_auth(token)
        } else {
            request
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExpoTicketResponse {
    data: Vec<ExpoTicket>,
}

#[derive(Debug, Deserialize)]
struct ExpoTicket {
    status: String,
    id: Option<String>,
    message: Option<String>,
    details: Option<ExpoErrorDetails>,
}

#[derive(Debug, Deserialize)]
struct ExpoErrorDetails {
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExpoReceiptResponse {
    data: HashMap<String, ExpoReceipt>,
}

#[derive(Debug, Deserialize)]
struct ExpoReceipt {
    status: String,
    message: Option<String>,
    details: Option<ExpoErrorDetails>,
}

fn expo_error(message: Option<String>, details: Option<ExpoErrorDetails>) -> PushOutcome {
    let code = details
        .and_then(|value| value.error)
        .unwrap_or_else(|| "unknown".to_string());
    let description = message.unwrap_or_else(|| "Expo push request failed".to_string());
    if code == "DeviceNotRegistered" {
        PushOutcome::PermanentInvalidToken(code)
    } else {
        PushOutcome::TransientFailure(format!("{code}: {description}"))
    }
}

impl PushTransport for ExpoPushTransport {
    fn send<'a>(
        &'a self,
        messages: &'a [PushMessage],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PushOutcome>>> + Send + 'a>> {
        Box::pin(async move {
            let response = self
                .request(&self.config.expo_push_url)
                .json(messages)
                .send()
                .await?
                .error_for_status()?
                .json::<ExpoTicketResponse>()
                .await?;
            if response.data.len() != messages.len() {
                return Err(anyhow!("Expo push ticket count mismatch"));
            }
            Ok(response
                .data
                .into_iter()
                .map(|ticket| {
                    if ticket.status == "ok" {
                        ticket.id.map(PushOutcome::Ticket).unwrap_or_else(|| {
                            PushOutcome::TransientFailure("missing Expo ticket ID".to_string())
                        })
                    } else {
                        expo_error(ticket.message, ticket.details)
                    }
                })
                .collect())
        })
    }

    fn receipts<'a>(
        &'a self,
        ticket_ids: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<HashMap<String, ReceiptOutcome>>> + Send + 'a>> {
        Box::pin(async move {
            let response = self
                .request(&self.config.expo_receipts_url)
                .json(&serde_json::json!({ "ids": ticket_ids }))
                .send()
                .await?
                .error_for_status()?
                .json::<ExpoReceiptResponse>()
                .await?;
            Ok(response
                .data
                .into_iter()
                .map(|(id, receipt)| {
                    let outcome = if receipt.status == "ok" {
                        ReceiptOutcome::Delivered
                    } else {
                        match expo_error(receipt.message, receipt.details) {
                            PushOutcome::PermanentInvalidToken(error) => {
                                ReceiptOutcome::PermanentInvalidToken(error)
                            }
                            PushOutcome::TransientFailure(error) => {
                                ReceiptOutcome::TransientFailure(error)
                            }
                            PushOutcome::Ticket(_) => ReceiptOutcome::Pending,
                        }
                    };
                    (id, outcome)
                })
                .collect())
        })
    }
}

fn message_for(delivery: &MobileNotificationDeliveryRecord) -> PushMessage {
    let body = match delivery.category.as_str() {
        "decision" => "A coding decision needs your attention.",
        "failure" => "A coding task needs your attention.",
        "pull_request" => "A pull request update is available.",
        "completion" => "A coding task finished.",
        _ => "A coding update is available.",
    };
    PushMessage {
        to: delivery.token.clone(),
        title: "APAS Code",
        body,
        data: GenericPushData {
            category: delivery.category.clone(),
            routing_id: delivery.routing_id.clone(),
            url: execution_notification_url(&delivery.routing_id).or_else(|| {
                delivery
                    .session_id
                    .as_deref()
                    .map(|session_id| format!("apas://code/session/{session_id}"))
            }),
        },
        sound: None,
        ttl: 3600,
    }
}

fn retry_delay(attempt_count: i64) -> u64 {
    5_u64.saturating_mul(2_u64.saturating_pow(attempt_count.clamp(0, 9) as u32))
}

async fn process_send_batch(state: &AppState, transport: &dyn PushTransport) -> Result<usize> {
    let deliveries = state
        .db
        .claim_mobile_notification_deliveries(state.config.mobile.push.batch_size)
        .await?;
    if deliveries.is_empty() {
        return Ok(0);
    }
    let messages = deliveries.iter().map(message_for).collect::<Vec<_>>();
    let outcomes = match transport.send(&messages).await {
        Ok(outcomes) => outcomes,
        Err(error) => {
            for delivery in &deliveries {
                state.mobile_metrics.increment(MobileMetric::PushRetry);
                state
                    .db
                    .retry_mobile_delivery(
                        delivery.id,
                        "push_transport_unavailable",
                        retry_delay(delivery.attempt_count + 1),
                    )
                    .await?;
            }
            return Err(error);
        }
    };
    for (delivery, outcome) in deliveries.iter().zip(outcomes) {
        match outcome {
            PushOutcome::Ticket(ticket_id) => {
                state.mobile_metrics.increment(MobileMetric::PushTicketed);
                state
                    .db
                    .mark_mobile_delivery_ticketed(delivery.id, &ticket_id)
                    .await?;
            }
            PushOutcome::TransientFailure(error) => {
                state.mobile_metrics.increment(MobileMetric::PushRetry);
                state
                    .db
                    .retry_mobile_delivery(
                        delivery.id,
                        &error,
                        retry_delay(delivery.attempt_count + 1),
                    )
                    .await?;
            }
            PushOutcome::PermanentInvalidToken(error) => {
                state
                    .mobile_metrics
                    .increment(MobileMetric::PushInvalidToken);
                state
                    .db
                    .retire_mobile_push_token(&delivery.push_token_id, delivery.id, &error)
                    .await?;
            }
        }
    }
    tracing::info!(
        delivery_count = deliveries.len(),
        "mobile push batch processed"
    );
    Ok(deliveries.len())
}

async fn process_receipts(state: &AppState, transport: &dyn PushTransport) -> Result<usize> {
    let deliveries = state
        .db
        .pending_mobile_notification_receipts(state.config.mobile.push.batch_size)
        .await?;
    if deliveries.is_empty() {
        return Ok(0);
    }
    let ticket_ids = deliveries
        .iter()
        .filter_map(|delivery| delivery.provider_ticket_id.clone())
        .collect::<Vec<_>>();
    let receipts = transport.receipts(&ticket_ids).await?;
    for delivery in &deliveries {
        let Some(ticket_id) = delivery.provider_ticket_id.as_deref() else {
            continue;
        };
        match receipts
            .get(ticket_id)
            .cloned()
            .unwrap_or(ReceiptOutcome::Pending)
        {
            ReceiptOutcome::Delivered => {
                state.mobile_metrics.increment(MobileMetric::PushDelivered);
                state.db.mark_mobile_delivery_delivered(delivery.id).await?;
            }
            ReceiptOutcome::Pending => {}
            ReceiptOutcome::TransientFailure(error) => {
                state.mobile_metrics.increment(MobileMetric::PushRetry);
                state
                    .db
                    .retry_mobile_delivery(
                        delivery.id,
                        &error,
                        retry_delay(delivery.attempt_count + 1),
                    )
                    .await?;
            }
            ReceiptOutcome::PermanentInvalidToken(error) => {
                state
                    .mobile_metrics
                    .increment(MobileMetric::PushInvalidToken);
                state
                    .db
                    .retire_mobile_push_token(&delivery.push_token_id, delivery.id, &error)
                    .await?;
            }
        }
    }
    tracing::info!(
        receipt_count = deliveries.len(),
        "mobile push receipts processed"
    );
    Ok(deliveries.len())
}

pub async fn run_worker(state: AppState, transport: Arc<dyn PushTransport>) {
    match state.db.recover_mobile_notification_outbox().await {
        Ok(recovered) if recovered > 0 => {
            tracing::info!(recovered, "recovered mobile notification outbox attempts");
        }
        Err(error) => tracing::warn!(%error, "failed to recover mobile notification outbox"),
        _ => {}
    }
    loop {
        if let Err(error) = process_send_batch(&state, transport.as_ref()).await {
            tracing::warn!(%error, "mobile push batch failed");
        }
        if let Err(error) = process_receipts(&state, transport.as_ref()).await {
            tracing::warn!(%error, "mobile push receipt poll failed");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub async fn enqueue_project_event(
    state: &AppState,
    project_id: &str,
    session_id: Option<&str>,
    pane_id: Option<u32>,
    category: &str,
    routing_id: &str,
    dedupe_key: &str,
) -> Result<bool> {
    if !state.config.mobile.features.notifications {
        return Ok(false);
    }
    let mut inserted = false;
    for user_id in state.db.active_project_user_ids(project_id).await? {
        inserted |= state
            .db
            .enqueue_mobile_notification_event(
                &uuid::Uuid::new_v4().to_string(),
                &user_id,
                project_id,
                session_id,
                pane_id,
                category,
                routing_id,
                &format!("{dedupe_key}:{user_id}"),
            )
            .await?;
    }
    Ok(inserted)
}

// Routing data contains only validated opaque identities; never copy worker text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionNotificationResource {
    Job(uuid::Uuid),
    Worker(uuid::Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionNotificationDestination {
    Detail,
    Review,
    Question(uuid::Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecutionNotificationRoute {
    pub resource: ExecutionNotificationResource,
    pub run_id: Option<uuid::Uuid>,
    pub destination: ExecutionNotificationDestination,
}

impl ExecutionNotificationRoute {
    pub(crate) fn parse(routing_id: &str) -> Option<Self> {
        let mut parts = routing_id.split(':');
        let kind = parts.next()?;
        let id = uuid::Uuid::parse_str(parts.next()?).ok()?;
        let resource = match kind {
            "job" => ExecutionNotificationResource::Job(id),
            "worker" => ExecutionNotificationResource::Worker(id),
            _ => return None,
        };
        let run_id = match parts.next()? {
            "-" => None,
            run => Some(uuid::Uuid::parse_str(run).ok()?),
        };
        let destination = match parts.next()? {
            "detail" => ExecutionNotificationDestination::Detail,
            "review" => ExecutionNotificationDestination::Review,
            question => {
                ExecutionNotificationDestination::Question(uuid::Uuid::parse_str(question).ok()?)
            }
        };
        if parts.next().is_some() {
            return None;
        }
        if kind == "worker"
            && run_id.is_none()
            && destination != ExecutionNotificationDestination::Detail
        {
            return None;
        }
        Some(Self {
            resource,
            run_id,
            destination,
        })
    }
}

fn execution_notification_url(routing_id: &str) -> Option<String> {
    use ExecutionNotificationDestination::*;
    let route = ExecutionNotificationRoute::parse(routing_id)?;
    let suffix = match route.destination {
        Detail => String::new(),
        Review => "/review".to_string(),
        Question(id) => format!("/questions/{id}"),
    };
    Some(match route.resource {
        ExecutionNotificationResource::Worker(id) => match route.run_id {
            Some(run) => format!("apas://code/workers/{id}/runs/{run}{suffix}"),
            None => format!("apas://code/workers/{id}"),
        },
        ExecutionNotificationResource::Job(id) => format!(
            "apas://code/jobs/{id}{suffix}{}",
            route
                .run_id
                .map(|run| format!("?run={run}"))
                .unwrap_or_default()
        ),
    })
}

fn job_event_destination(
    detail: &shared::JobDetail,
    event: &shared::JobEvent,
) -> Option<(&'static str, String)> {
    use shared::{JobEventKind, JobState};
    let worker = detail.task.worker.is_some();
    let run = event
        .run_id
        .and_then(|id| detail.runs.iter().find(|run| run.id == id));
    if worker && run.is_none() {
        return None;
    }
    let blocked = run.is_some_and(|run| {
        matches!(
            run.state,
            JobState::Failed | JobState::Blocked | JobState::RecoveryRequired
        )
    });
    match event.kind {
        JobEventKind::Question => {
            let id = event
                .detail
                .as_ref()
                .and_then(|value| {
                    value
                        .get("question_id")
                        .or_else(|| value.get("id"))
                        .or_else(|| value.get("question").and_then(|q| q.get("id")))
                })
                .and_then(|value| value.as_str())
                .and_then(|value| uuid::Uuid::parse_str(value).ok())?;
            detail
                .questions
                .iter()
                .find(|question| question.id == id && Some(question.run_id) == event.run_id)?;
            Some(("decision", id.to_string()))
        }
        JobEventKind::Failed | JobEventKind::RecoveryRequired => Some(("failure", "detail".into())),
        JobEventKind::Verification | JobEventKind::Publication | JobEventKind::Stopped
            if worker && blocked =>
        {
            Some(("failure", "detail".into()))
        }
        JobEventKind::Publication if worker => (run
            .is_some_and(|run| run.state == JobState::ReviewReady)
            && detail.publication.is_some())
        .then(|| ("pull_request", "review".into())),
        JobEventKind::Verification | JobEventKind::Stopped if worker => None,
        JobEventKind::Verification | JobEventKind::Publication => {
            Some(("pull_request", "review".into()))
        }
        JobEventKind::Stopped => Some(("completion", "detail".into())),
        _ => None,
    }
}

pub async fn enqueue_job_event(
    state: &AppState,
    project_id: &str,
    event: &shared::JobEvent,
) -> Result<bool> {
    if !state.config.mobile.features.notifications {
        return Ok(false);
    }
    let Some(job) = state.db.job(event.job_id).await? else {
        return Ok(false);
    };
    if job.detail.job.project_id != project_id {
        return Ok(false);
    }
    let Some((category, destination)) = job_event_destination(&job.detail, event) else {
        return Ok(false);
    };
    let routing = if let Some(worker) = &job.detail.task.worker {
        let Some(run) = event.run_id else {
            return Ok(false);
        };
        format!("worker:{}:{run}:{destination}", worker.worker_id)
    } else {
        format!(
            "job:{}:{}:{destination}",
            event.job_id,
            event
                .run_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string())
        )
    };
    let mut inserted = false;
    for user_id in state.db.active_project_user_ids(project_id).await? {
        let Ok(actor) = uuid::Uuid::parse_str(&user_id) else {
            continue;
        };
        if crate::jobs::detail(state, actor, project_id, event.job_id)
            .await
            .is_err()
        {
            continue;
        }
        let dedupe = if job.detail.task.worker.is_some() {
            format!("{routing}:{category}:{user_id}")
        } else {
            format!("job:{}:{user_id}", event.id)
        };
        inserted |= state
            .db
            .enqueue_mobile_notification_event(
                &uuid::Uuid::new_v4().to_string(),
                &user_id,
                project_id,
                None,
                None,
                category,
                &routing,
                &dedupe,
            )
            .await?;
    }
    Ok(inserted)
}

/// Only the durable worker journal supplies worker-only blockers. Execution
/// outcomes use the job journal instead, so one failure cannot notify twice.
pub async fn enqueue_worker_event(state: &AppState, event: &shared::WorkerEvent) -> Result<bool> {
    use shared::{WorkerEventKind, WorkerState};
    if !state.config.mobile.features.notifications
        || event.job_id.is_some()
        || event.run_id.is_some()
        || event.question_id.is_some()
        || !matches!(
            event.kind,
            WorkerEventKind::StateChanged | WorkerEventKind::Review
        )
    {
        return Ok(false);
    }
    let Some(worker) = state.db.worker(event.worker_id).await? else {
        return Ok(false);
    };
    let current = &worker.detail.worker;
    if current.project_id != event.project_id
        || current.active_job_id.is_some()
        || !matches!(
            current.state,
            WorkerState::Blocked | WorkerState::RecoveryRequired
        )
    {
        return Ok(false);
    }
    let routing = format!("worker:{}:-:detail", event.worker_id);
    let mut inserted = false;
    for user_id in state.db.active_project_user_ids(&event.project_id).await? {
        if !state
            .db
            .has_project_content_access(&event.project_id, &user_id)
            .await?
        {
            continue;
        }
        inserted |= state
            .db
            .enqueue_mobile_notification_event(
                &uuid::Uuid::new_v4().to_string(),
                &user_id,
                &event.project_id,
                None,
                None,
                "failure",
                &routing,
                &format!("worker:{}:{}:{user_id}", event.worker_id, event.seq),
            )
            .await?;
    }
    Ok(inserted)
}

/// Bounded dispatcher work, independent of websocket subscriptions and replay
/// retention. A crash after enqueue replays the same recipient deduplication key.
pub async fn drain_worker_notifications(state: &AppState) -> Result<()> {
    for event in state.db.pending_worker_notifications(100).await? {
        enqueue_worker_event(state, &event).await?;
        state
            .db
            .acknowledge_worker_notification(event.worker_id, event.seq)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_job() -> shared::JobDetail {
        use shared::{JobAllowedActions, JobLimits, JobUsage};
        let worker = uuid::Uuid::new_v4();
        let job = uuid::Uuid::new_v4();
        let run = uuid::Uuid::new_v4();
        let owner = uuid::Uuid::new_v4();
        let machine = uuid::Uuid::new_v4();
        serde_json::from_value(serde_json::json!({
            "job": {"id":job,"project_id":"project","project_name":"private project","title":"private task",
                "state":"completed","state_version":1,"current_run_id":run,"cluster_owner_user_id":owner,
                "machine_id":machine,"hostname":"host","created_at":"now","updated_at":"now",
                "usage":JobUsage::default(),"limits":JobLimits::default(),"attention_count":0,
                "reason":null,"allowed_actions":JobAllowedActions::default()},
            "task": {"instruction":"private instruction","acceptance_criteria":["private criterion"],
                "repository":"owner/repo","base_ref":"main","base_sha":"a".repeat(40),
                "worker_profile":"worker","worker_profile_digest":"digest","verification_profile":"verify",
                "verification_profile_digest":"digest","model_profile":"model",
                "worker":{"worker_id":worker,"mandate_revision":1,"backlog_revision":1,"schedule_generation":1,
                    "reservation_id":uuid::Uuid::new_v4(),"purpose":"inspection",
                    "mandate":{"title":"private title","description":"private description","checklist":["private checklist"],
                        "machine_id":machine,"worker_profile":"worker","verification_profile":"verify","model_profile":"model",
                        "base_ref":"main","interval_seconds":900,
                        "limits":{"per_run":JobLimits::default(),"elapsed_seconds_24h":7200,
                            "spend_micro_usd_24h":20000000,"runs_24h":24,"attempts_per_item":1}},
                    "items":[],"selected_item_id":null,"checkpoint":null}},
            "runs":[{"id":run,"job_id":job,"attempt":1,"epoch":1,"state":"completed",
                "usage":JobUsage::default()}],
            "questions":[],"high_watermark":0,"policy_digest":"digest","publication":null
        })).unwrap()
    }

    fn event(detail: &shared::JobDetail, kind: shared::JobEventKind) -> shared::JobEvent {
        shared::JobEvent {
            id: uuid::Uuid::new_v4(),
            job_id: detail.job.id,
            run_id: Some(detail.runs[0].id),
            seq: 1,
            kind,
            summary: "private summary".into(),
            created_at: "now".into(),
            detail: None,
        }
    }

    #[tokio::test]
    async fn worker_only_failures_survive_journal_replay_without_socket_or_duplicate_push() {
        use crate::db::{workers as store, Database, User};
        use shared::*;
        let job = worker_job();
        let context = job.task.worker.as_ref().unwrap();
        let owner = job.job.cluster_owner_user_id;
        let timestamp = chrono::Utc::now().to_rfc3339();
        let root = std::env::temp_dir().join(format!(
            "apas-worker-notifications-{}",
            uuid::Uuid::new_v4()
        ));
        let db = Database::new(&root.join("db.sqlite").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db.create_user(&User {
            id: owner.to_string(),
            email: format!("{owner}@test"),
            password_hash: "unused".into(),
            created_at: None,
            cluster_role: "user".into(),
            account_status: "active".into(),
        })
        .await
        .unwrap();
        db.authorize_project_registration("project", &owner.to_string())
            .await
            .unwrap();
        db.create_mobile_device_session(
            "device",
            &owner.to_string(),
            "installation",
            "ios",
            None,
            "test",
            "refresh",
            &(chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
        )
        .await
        .unwrap();
        db.register_mobile_push_token(
            "push",
            &owner.to_string(),
            "device",
            "installation",
            "ios",
            "ExponentPushToken[worker]",
            "hash",
        )
        .await
        .unwrap();
        let mut worker = store::StoredWorker {
            detail: WorkerDetail {
                worker: WorkerSummary {
                    id: context.worker_id,
                    project_id: "project".into(),
                    title: "private title".into(),
                    description: "private description".into(),
                    state: WorkerState::Idle,
                    reason: None,
                    desired_state: WorkerDesiredState::Active,
                    mandate_revision: 1,
                    backlog_revision: 1,
                    state_version: 1,
                    high_watermark: 0,
                    next_check_at: None,
                    active_job_id: None,
                    active_run_id: None,
                    current_item_id: None,
                    machine_id: context.mandate.machine_id,
                    cluster_owner_user_id: owner,
                    attention_run_id: None,
                    attention_question_id: None,
                    hostname: "host".into(),
                    attention: false,
                    created_at: timestamp.clone(),
                    updated_at: timestamp.clone(),
                },
                mandate: WorkerMandate {
                    revision: 1,
                    spec: context.mandate.clone(),
                    repository: "owner/repo".into(),
                    worker_profile_digest: "digest".into(),
                    verification_profile_digest: "digest".into(),
                    policy_digest: "digest".into(),
                    approved_by: owner,
                    approved_at: timestamp.clone(),
                },
                checkpoint: None,
                budget: WorkerBudget::default(),
                allowed_actions: WorkerAllowedActions::default(),
                current_job: None,
            },
            schedule_generation: 1,
            grant_active: true,
            stop_requested: false,
            accounting_at: timestamp,
        };
        let mut tx = db.job_transaction().await.unwrap();
        store::save(&mut tx, &worker).await.unwrap();
        store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Routine idle inspection",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(db
            .pending_worker_notifications(100)
            .await
            .unwrap()
            .is_empty());
        let mut tx = db.job_transaction().await.unwrap();
        worker.detail.worker.state = WorkerState::Blocked;
        worker.detail.worker.reason = Some("worker_reapproval_required".into());
        worker.grant_active = false;
        let notice = store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Private grant details",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(db.pending_worker_notifications(100).await.unwrap().len(), 1);
        let mut config = crate::config::Config::default();
        config.mobile.features.notifications = true;
        let state = AppState::new(db, config);
        // Simulate a dispatcher crash after enqueue but before journal acknowledgement.
        assert!(enqueue_worker_event(&state, &notice).await.unwrap());
        drain_worker_notifications(&state).await.unwrap();
        assert!(state
            .db
            .pending_worker_notifications(100)
            .await
            .unwrap()
            .is_empty());
        let deliveries = state
            .db
            .claim_mobile_notification_deliveries(100)
            .await
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        let message = message_for(&deliveries[0]);
        assert_eq!(
            message.data.url,
            Some(format!("apas://code/workers/{}", context.worker_id))
        );
        assert_eq!(message.body, "A coding task needs your attention.");
        assert!(!serde_json::to_string(&message)
            .unwrap()
            .contains("Private grant"));
        let mut tx = state.db.job_transaction().await.unwrap();
        store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Same blocker",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(state
            .db
            .pending_worker_notifications(100)
            .await
            .unwrap()
            .is_empty());
        let mut tx = state.db.job_transaction().await.unwrap();
        worker.detail.worker.state = WorkerState::RecoveryRequired;
        worker.detail.worker.reason = Some("worker_accounting_uncertain".into());
        store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Preparation recovery",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        drain_worker_notifications(&state).await.unwrap();
        assert_eq!(
            state
                .db
                .claim_mobile_notification_deliveries(100)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn worker_decisions_require_the_exact_retained_question_and_run() {
        let mut detail = worker_job();
        let mut notice = event(&detail, shared::JobEventKind::Question);
        let question = uuid::Uuid::new_v4();
        notice.detail = Some(serde_json::json!({"question_id":question}));
        assert!(job_event_destination(&detail, &notice).is_none());
        detail.questions.push(shared::JobQuestion {
            id: question,
            run_id: detail.runs[0].id,
            epoch: 1,
            version: 1,
            dialog_id: "private dialog".into(),
            tool_call_id: "private tool".into(),
            state: shared::JobQuestionState::Pending,
            questions: vec![],
            answers: None,
            created_at: "now".into(),
            confirmed_at: None,
        });
        assert_eq!(
            job_event_destination(&detail, &notice),
            Some(("decision", question.to_string()))
        );
        detail.questions[0].run_id = uuid::Uuid::new_v4();
        assert!(job_event_destination(&detail, &notice).is_none());
        notice.detail = Some(serde_json::json!({"question_id":"private question text"}));
        assert!(job_event_destination(&detail, &notice).is_none());
    }

    #[test]
    fn routine_worker_activity_is_silent_but_failed_inspections_are_actionable() {
        use shared::{JobEventKind, JobState};
        let mut detail = worker_job();
        for kind in [
            JobEventKind::Admitted,
            JobEventKind::PhaseChanged,
            JobEventKind::Progress,
            JobEventKind::Usage,
            JobEventKind::Stopped,
            JobEventKind::Verification,
            JobEventKind::Publication,
        ] {
            assert!(job_event_destination(&detail, &event(&detail, kind)).is_none());
        }
        for kind in [JobEventKind::Failed, JobEventKind::RecoveryRequired] {
            assert_eq!(
                job_event_destination(&detail, &event(&detail, kind)),
                Some(("failure", "detail".into()))
            );
        }
        detail.runs[0].state = JobState::Blocked;
        assert_eq!(
            job_event_destination(&detail, &event(&detail, JobEventKind::Stopped)),
            Some(("failure", "detail".into()))
        );
        detail.task.worker = None;
        assert_eq!(
            job_event_destination(&detail, &event(&detail, JobEventKind::Stopped)),
            Some(("completion", "detail".into()))
        );
    }

    #[test]
    fn worker_reviews_require_delivered_drafts_and_keep_the_event_run() {
        use shared::{JobEventKind, JobState};
        let mut detail = worker_job();
        let mut notice = event(&detail, JobEventKind::Publication);
        detail.runs[0].state = JobState::Publishing;
        assert!(job_event_destination(&detail, &notice).is_none());
        detail.runs[0].state = JobState::Blocked;
        notice.kind = JobEventKind::Verification;
        assert_eq!(
            job_event_destination(&detail, &notice),
            Some(("failure", "detail".into()))
        );
        detail.runs[0].state = JobState::ReviewReady;
        notice.kind = JobEventKind::Publication;
        assert!(job_event_destination(&detail, &notice).is_none());
        detail.publication = Some(shared::JobPublication {
            repository: "owner/repo".into(),
            number: 1,
            url: "https://example.test/pull/1".into(),
            branch: "candidate".into(),
            base_sha: "a".repeat(40),
            head_sha: "b".repeat(40),
            draft: true,
            state: "open".into(),
            observed_at: "now".into(),
        });
        assert_eq!(
            job_event_destination(&detail, &notice),
            Some(("pull_request", "review".into()))
        );
        notice.run_id = Some(uuid::Uuid::new_v4());
        assert!(job_event_destination(&detail, &notice).is_none());
    }

    #[test]
    fn worker_payload_routes_exact_history_without_private_text_or_fallbacks() {
        let worker = uuid::Uuid::new_v4();
        let run = uuid::Uuid::new_v4();
        let question = uuid::Uuid::new_v4();
        for (destination, suffix) in [
            ("detail".to_string(), String::new()),
            ("review".to_string(), "/review".to_string()),
            (question.to_string(), format!("/questions/{question}")),
        ] {
            let delivery = MobileNotificationDeliveryRecord {
                id: 1,
                event_id: "event".into(),
                push_token_id: "token".into(),
                token: "opaque".into(),
                category: "decision".into(),
                routing_id: format!("worker:{worker}:{run}:{destination}"),
                session_id: None,
                attempt_count: 0,
                provider_ticket_id: None,
            };
            let message = message_for(&delivery);
            assert_eq!(
                message.data.url,
                Some(format!("apas://code/workers/{worker}/runs/{run}{suffix}"))
            );
            assert_eq!(message.body, "A coding decision needs your attention.");
        }
        assert_eq!(
            execution_notification_url(&format!("worker:{worker}:-:detail")),
            Some(format!("apas://code/workers/{worker}"))
        );
        for route in [
            format!("worker:{worker}:-:review"),
            format!("worker:{worker}:{run}:review:extra"),
            format!("worker:{worker}:{run}:private-question"),
            format!("worker:../../private:{run}:detail"),
        ] {
            assert!(execution_notification_url(&route).is_none());
        }
    }

    #[test]
    fn job_notification_routes_only_validated_opaque_context_without_a_session() {
        let job = uuid::Uuid::new_v4();
        let run = uuid::Uuid::new_v4();
        let question = uuid::Uuid::new_v4();
        let delivery = MobileNotificationDeliveryRecord {
            id: 1,
            event_id: uuid::Uuid::new_v4().to_string(),
            push_token_id: "token-id".to_string(),
            token: "ExponentPushToken[opaque]".to_string(),
            category: "decision".to_string(),
            routing_id: format!("job:{job}:{run}:{question}"),
            session_id: None,
            attempt_count: 0,
            provider_ticket_id: None,
        };
        let message = message_for(&delivery);
        assert_eq!(message.body, "A coding decision needs your attention.");
        assert_eq!(
            message.data.url,
            Some(format!(
                "apas://code/jobs/{job}/questions/{question}?run={run}"
            ))
        );
        assert_eq!(
            execution_notification_url(&format!("job:{job}:{run}:review")),
            Some(format!("apas://code/jobs/{job}/review?run={run}"))
        );
        assert!(
            execution_notification_url(&format!("job:{job}:{run}:private-question-text")).is_none()
        );
        assert!(execution_notification_url("job:../../secret:-:detail").is_none());
        assert!(execution_notification_url(&format!("job:{job}:invalid:detail")).is_none());
    }

    #[test]
    fn generic_payload_contains_no_project_content_or_paths() {
        let delivery = MobileNotificationDeliveryRecord {
            id: 1,
            event_id: "event".to_string(),
            push_token_id: "token-id".to_string(),
            token: "ExponentPushToken[opaque]".to_string(),
            category: "decision".to_string(),
            routing_id: "route-opaque".to_string(),
            session_id: Some("58dbd62d-c40f-4b5b-a1d4-96aca52ea595".to_string()),
            attempt_count: 0,
            provider_ticket_id: None,
        };
        let json = serde_json::to_value(message_for(&delivery)).unwrap();
        assert_eq!(json["title"], "APAS Code");
        assert_eq!(json["data"]["category"], "decision");
        let serialized = json.to_string().to_lowercase();
        for forbidden_key in [
            "\"prompt\"",
            "\"output\"",
            "\"code\"",
            "\"diff\"",
            "\"terminal\"",
            "\"secret\"",
            "\"filesystem_path\"",
            "\"project_name\"",
            "\"working_dir\"",
        ] {
            assert!(!serialized.contains(forbidden_key));
        }
    }

    #[test]
    fn retries_are_bounded() {
        assert_eq!(retry_delay(0), 5);
        assert!(retry_delay(100) <= 2560);
    }

    #[test]
    fn expo_errors_distinguish_invalid_tokens_from_transient_failures() {
        assert_eq!(
            expo_error(
                Some("device is no longer registered".to_string()),
                Some(ExpoErrorDetails {
                    error: Some("DeviceNotRegistered".to_string()),
                }),
            ),
            PushOutcome::PermanentInvalidToken("DeviceNotRegistered".to_string())
        );
        assert!(matches!(
            expo_error(
                Some("provider busy".to_string()),
                Some(ExpoErrorDetails {
                    error: Some("MessageRateExceeded".to_string()),
                }),
            ),
            PushOutcome::TransientFailure(message) if message.contains("provider busy")
        ));
    }
}
