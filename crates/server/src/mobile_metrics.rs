use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy)]
pub enum MobileMetric {
    AuthLoginSuccess,
    AuthRefreshSuccess,
    AuthFailure,
    DeviceRevocation,
    SocketAuthenticated,
    ProtocolIncompatible,
    CatchupRequest,
    MutationAccepted,
    MutationRejected,
    TerminalAttach,
    TerminalAttachEmpty,
    TerminalBridgeReady,
    TerminalBridgeRejectedMessage,
    TerminalBridgeCrash,
    PushTicketed,
    PushRetry,
    PushDelivered,
    PushInvalidToken,
}

#[derive(Default)]
pub struct MobileMetrics {
    auth_login_success: AtomicU64,
    auth_refresh_success: AtomicU64,
    auth_failure: AtomicU64,
    device_revocation: AtomicU64,
    socket_authenticated: AtomicU64,
    protocol_incompatible: AtomicU64,
    catchup_request: AtomicU64,
    mutation_accepted: AtomicU64,
    mutation_rejected: AtomicU64,
    terminal_attach: AtomicU64,
    terminal_attach_empty: AtomicU64,
    terminal_bridge_ready: AtomicU64,
    terminal_bridge_rejected_message: AtomicU64,
    terminal_bridge_crash: AtomicU64,
    push_ticketed: AtomicU64,
    push_retry: AtomicU64,
    push_delivered: AtomicU64,
    push_invalid_token: AtomicU64,
}

#[derive(Debug, Serialize)]
pub struct MobileMetricsSnapshot {
    pub auth_login_success: u64,
    pub auth_refresh_success: u64,
    pub auth_failure: u64,
    pub device_revocation: u64,
    /// Counts authenticated mobile sockets. Re-authentication after every
    /// foreground/reconnect is intentionally included.
    pub socket_authenticated: u64,
    pub protocol_incompatible: u64,
    pub catchup_request: u64,
    pub mutation_accepted: u64,
    pub mutation_rejected: u64,
    pub terminal_attach: u64,
    pub terminal_attach_empty: u64,
    pub terminal_bridge_ready: u64,
    pub terminal_bridge_rejected_message: u64,
    pub terminal_bridge_crash: u64,
    pub push_ticketed: u64,
    pub push_retry: u64,
    pub push_delivered: u64,
    pub push_invalid_token: u64,
}

impl MobileMetrics {
    pub fn increment(&self, metric: MobileMetric) {
        let counter = match metric {
            MobileMetric::AuthLoginSuccess => &self.auth_login_success,
            MobileMetric::AuthRefreshSuccess => &self.auth_refresh_success,
            MobileMetric::AuthFailure => &self.auth_failure,
            MobileMetric::DeviceRevocation => &self.device_revocation,
            MobileMetric::SocketAuthenticated => &self.socket_authenticated,
            MobileMetric::ProtocolIncompatible => &self.protocol_incompatible,
            MobileMetric::CatchupRequest => &self.catchup_request,
            MobileMetric::MutationAccepted => &self.mutation_accepted,
            MobileMetric::MutationRejected => &self.mutation_rejected,
            MobileMetric::TerminalAttach => &self.terminal_attach,
            MobileMetric::TerminalAttachEmpty => &self.terminal_attach_empty,
            MobileMetric::TerminalBridgeReady => &self.terminal_bridge_ready,
            MobileMetric::TerminalBridgeRejectedMessage => &self.terminal_bridge_rejected_message,
            MobileMetric::TerminalBridgeCrash => &self.terminal_bridge_crash,
            MobileMetric::PushTicketed => &self.push_ticketed,
            MobileMetric::PushRetry => &self.push_retry,
            MobileMetric::PushDelivered => &self.push_delivered,
            MobileMetric::PushInvalidToken => &self.push_invalid_token,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MobileMetricsSnapshot {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        MobileMetricsSnapshot {
            auth_login_success: load(&self.auth_login_success),
            auth_refresh_success: load(&self.auth_refresh_success),
            auth_failure: load(&self.auth_failure),
            device_revocation: load(&self.device_revocation),
            socket_authenticated: load(&self.socket_authenticated),
            protocol_incompatible: load(&self.protocol_incompatible),
            catchup_request: load(&self.catchup_request),
            mutation_accepted: load(&self.mutation_accepted),
            mutation_rejected: load(&self.mutation_rejected),
            terminal_attach: load(&self.terminal_attach),
            terminal_attach_empty: load(&self.terminal_attach_empty),
            terminal_bridge_ready: load(&self.terminal_bridge_ready),
            terminal_bridge_rejected_message: load(&self.terminal_bridge_rejected_message),
            terminal_bridge_crash: load(&self.terminal_bridge_crash),
            push_ticketed: load(&self.push_ticketed),
            push_retry: load(&self.push_retry),
            push_delivered: load(&self.push_delivered),
            push_invalid_token: load(&self.push_invalid_token),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_are_monotonic_and_payload_is_redacted() {
        let metrics = MobileMetrics::default();
        metrics.increment(MobileMetric::AuthLoginSuccess);
        metrics.increment(MobileMetric::PushRetry);
        metrics.increment(MobileMetric::PushRetry);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.auth_login_success, 1);
        assert_eq!(snapshot.push_retry, 2);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json
            .as_object()
            .unwrap()
            .values()
            .all(serde_json::Value::is_u64));
    }
}
