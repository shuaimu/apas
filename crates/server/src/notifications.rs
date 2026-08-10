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
            url: delivery
                .session_id
                .as_deref()
                .map(|session_id| format!("apas://code/session/{session_id}")),
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

#[cfg(test)]
mod tests {
    use super::*;

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
