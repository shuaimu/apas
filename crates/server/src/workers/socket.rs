//! Capability-gated hints and ordered, currently authorized worker replay.
use crate::{jobs, state::AppState};
use shared::{ServerToWeb, AUTONOMOUS_WORKERS_CAPABILITY};
use uuid::Uuid;

fn capable(state: &AppState, connection: Uuid) -> bool {
    state
        .sessions
        .web_supports_capability(&connection, AUTONOMOUS_WORKERS_CAPABILITY)
}

pub fn unsubscribe(state: &AppState, connection: Uuid, worker: Uuid) {
    state
        .jobs
        .worker_subscriptions
        .remove(&(connection, worker));
    state
        .jobs
        .worker_subscription_cursors
        .remove(&(connection, worker));
}

async fn reject(
    state: &AppState,
    connection: Uuid,
    project: &str,
    worker: Uuid,
    code: &str,
    resync_required: bool,
) {
    unsubscribe(state, connection, worker);
    if !capable(state, connection) {
        return;
    }
    state
        .sessions
        .send_to_web(
            &connection,
            ServerToWeb::WorkerSubscriptionRejected {
                project_id: project.into(),
                worker_id: worker,
                code: code.into(),
                resync_required,
            },
        )
        .await;
}

// The socket lock prevents catch-up and simultaneous committed-change hints from
// interleaving pages. Every queued content frame is reauthorized again at send.
async fn replay(
    state: &AppState,
    connection: Uuid,
    actor: Uuid,
    project: &str,
    worker: Uuid,
    after: u64,
) {
    match super::events(state, actor, project, worker, after, 1000).await {
        Ok(page) if !page.resync_required => {
            let mut cursor = after;
            for event in page.events {
                if event.seq != cursor.saturating_add(1) {
                    reject(
                        state,
                        connection,
                        project,
                        worker,
                        "snapshot_resync_required",
                        true,
                    )
                    .await;
                    return;
                }
                cursor = event.seq;
                state
                    .sessions
                    .send_to_web(&connection, ServerToWeb::WorkerEvent { event })
                    .await;
            }
            if page.has_more {
                reject(
                    state,
                    connection,
                    project,
                    worker,
                    "replay_page_required",
                    true,
                )
                .await;
            } else {
                state
                    .jobs
                    .worker_subscription_cursors
                    .insert((connection, worker), cursor);
            }
        }
        Ok(_) => {
            reject(
                state,
                connection,
                project,
                worker,
                "snapshot_resync_required",
                true,
            )
            .await
        }
        Err(_) => {
            reject(
                state,
                connection,
                project,
                worker,
                "worker_access_denied",
                false,
            )
            .await
        }
    }
}

pub async fn subscribe(
    state: &AppState,
    connection: Uuid,
    project: String,
    worker: Uuid,
    after: u64,
) {
    if !capable(state, connection) {
        return;
    }
    let _guard = state.jobs.worker_socket_lock.lock().await;
    let Some(actor) = jobs::connection_actor(state, connection).await else {
        reject(
            state,
            connection,
            &project,
            worker,
            "worker_access_denied",
            false,
        )
        .await;
        return;
    };
    let snapshot = match super::detail(state, actor, &project, worker).await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            reject(
                state,
                connection,
                &project,
                worker,
                "worker_access_denied",
                false,
            )
            .await;
            return;
        }
    };
    state
        .jobs
        .worker_subscriptions
        .insert((connection, worker), project.clone());
    state
        .sessions
        .send_to_web(
            &connection,
            ServerToWeb::WorkerSnapshot {
                detail: Box::new(snapshot),
            },
        )
        .await;
    replay(state, connection, actor, &project, worker, after).await;
}

pub async fn fanout(state: &AppState, worker: Uuid) {
    let _guard = state.jobs.worker_socket_lock.lock().await;
    let subscribers: Vec<_> = state
        .jobs
        .worker_subscriptions
        .iter()
        .filter(|entry| entry.key().1 == worker)
        .map(|entry| (entry.key().0, entry.value().clone()))
        .collect();
    for (connection, project) in subscribers {
        let actor = jobs::connection_actor(state, connection).await;
        if !capable(state, connection) || actor.is_none() {
            reject(
                state,
                connection,
                &project,
                worker,
                "worker_access_denied",
                false,
            )
            .await;
            continue;
        }
        let after = state
            .jobs
            .worker_subscription_cursors
            .get(&(connection, worker))
            .map(|v| *v)
            .unwrap_or(0);
        replay(state, connection, actor.unwrap(), &project, worker, after).await;
    }
    let connections: Vec<_> = state
        .jobs
        .connection_auth
        .iter()
        .map(|entry| *entry.key())
        .collect();
    for connection in connections {
        if !capable(state, connection) {
            continue;
        }
        if let Some(actor) = jobs::connection_actor(state, connection).await {
            if super::detail_by_id(state, actor, worker).await.is_ok() {
                state
                    .sessions
                    .send_to_web(&connection, ServerToWeb::WorkerInboxChanged {})
                    .await;
            }
        }
    }
}

pub async fn reconcile_subscriptions(state: &AppState) -> anyhow::Result<()> {
    let _guard = state.jobs.worker_socket_lock.lock().await;
    let subscriptions: Vec<_> = state
        .jobs
        .worker_subscriptions
        .iter()
        .map(|entry| (*entry.key(), entry.value().clone()))
        .collect();
    for ((connection, worker), project) in subscriptions {
        let allowed = if let Some(actor) = jobs::connection_actor(state, connection).await {
            capable(state, connection)
                && super::detail(state, actor, &project, worker).await.is_ok()
        } else {
            false
        };
        if !allowed {
            reject(
                state,
                connection,
                &project,
                worker,
                "worker_access_denied",
                false,
            )
            .await;
        }
    }
    Ok(())
}

pub async fn web_outbound_allowed(
    state: &AppState,
    connection: Uuid,
    message: &ServerToWeb,
) -> bool {
    let worker = match message {
        ServerToWeb::WorkerSnapshot { detail } => Some(detail.worker.id),
        ServerToWeb::WorkerEvent { event } => Some(event.worker_id),
        ServerToWeb::WorkerInboxChanged {} => None,
        ServerToWeb::WorkerSubscriptionRejected { .. } => return capable(state, connection),
        _ => return true,
    };
    if !capable(state, connection) {
        return false;
    }
    let Some(actor) = jobs::connection_actor(state, connection).await else {
        return false;
    };
    match worker {
        Some(worker) => super::detail_by_id(state, actor, worker).await.is_ok(),
        None => state
            .db
            .get_user_by_id(&actor.to_string())
            .await
            .is_ok_and(|user| user.is_some_and(|user| user.is_active())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::workers::tests::Fixture;
    use chrono::{Duration, Utc};
    use shared::{WorkerState, AUTONOMOUS_JOBS_CAPABILITY};
    use tokio::sync::mpsc::{self, Receiver};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn websocket_authentication_delivers_worker_snapshot_on_default_stack() {
        use futures::{SinkExt, StreamExt};
        use shared::WebToServer;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let f = Fixture::new().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = crate::routes::create_router(f.state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &crate::routes::auth::Claims {
                sub: f.owner().to_string(),
                exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
                credential_version: None,
            },
            &jsonwebtoken::EncodingKey::from_secret(f.state.config.auth.jwt_secret.as_bytes()),
        )
        .unwrap();
        let (mut socket, _) = connect_async(format!("ws://{address}/ws/web"))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            socket
                .send(Message::Text(
                    serde_json::to_string(&WebToServer::Authenticate {
                        token,
                        capabilities: vec![AUTONOMOUS_WORKERS_CAPABILITY.into()],
                        client_kind: Some(shared::ClientKind::Web),
                        app_version: None,
                        protocol_version: None,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = message {
                    if matches!(
                        serde_json::from_str::<ServerToWeb>(&text).unwrap(),
                        ServerToWeb::Authenticated { .. }
                    ) {
                        break;
                    }
                }
            }
            socket
                .send(Message::Text(
                    serde_json::to_string(&WebToServer::SubscribeWorker {
                        project_id: f.project(),
                        worker_id: f.id(),
                        after_seq: 0,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = message {
                    let frame: ServerToWeb = serde_json::from_str(&text).unwrap();
                    if let ServerToWeb::WorkerSnapshot { detail } = frame {
                        assert_eq!(detail.worker.id, f.id());
                        assert_eq!(detail.worker.project_id, f.project());
                        let wire: serde_json::Value = serde_json::from_str(&text).unwrap();
                        assert_eq!(wire["type"], "worker_snapshot");
                        assert_eq!(wire["detail"], serde_json::to_value(&detail).unwrap());
                        break;
                    }
                }
            }
        })
        .await
        .expect("authenticated worker snapshot must arrive on the default runtime stack");
        socket.close(None).await.unwrap();
        server.abort();
    }

    fn connect(
        f: &Fixture,
        capabilities: &[&str],
        device: Option<&str>,
    ) -> (Uuid, Receiver<ServerToWeb>) {
        let id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(32);
        f.state.sessions.register_web(id, tx);
        f.state.sessions.set_web_user(id, f.owner());
        f.state
            .sessions
            .set_web_capabilities(id, capabilities.iter().map(|s| (*s).into()).collect());
        f.state.jobs.connection_auth.insert(
            id,
            (
                f.owner(),
                device.map(str::to_owned),
                (Utc::now() + Duration::hours(1)).timestamp() as usize,
            ),
        );
        (id, rx)
    }

    fn drain(rx: &mut Receiver<ServerToWeb>) -> Vec<ServerToWeb> {
        let mut frames = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            frames.push(frame);
        }
        frames
    }

    fn assert_rejected(frames: &[ServerToWeb], worker: Uuid, code: &str, resync: bool) {
        assert!(
            matches!(frames.last(), Some(ServerToWeb::WorkerSubscriptionRejected {
            worker_id, code: actual, resync_required, ..
        }) if *worker_id == worker && actual == code && *resync_required == resync)
        );
        assert!(!frames
            .iter()
            .any(|frame| matches!(frame, ServerToWeb::WorkerEvent { .. })));
    }

    #[tokio::test]
    async fn unnegotiated_and_legacy_only_clients_receive_no_worker_variants() {
        let f = Fixture::new().await;
        let snapshot = ServerToWeb::WorkerSnapshot {
            detail: Box::new(f.worker.detail.clone()),
        };
        let event = ServerToWeb::WorkerEvent {
            event: f
                .state
                .db
                .worker_events(f.id(), 0, 10)
                .await
                .unwrap()
                .events
                .remove(0),
        };
        let rejection = ServerToWeb::WorkerSubscriptionRejected {
            project_id: f.project(),
            worker_id: f.id(),
            code: "worker_access_denied".into(),
            resync_required: false,
        };
        for capabilities in [&[][..], &[AUTONOMOUS_JOBS_CAPABILITY][..]] {
            let (connection, mut rx) = connect(&f, capabilities, None);
            subscribe(&f.state, connection, f.project(), f.id(), 0).await;
            fanout(&f.state, f.id()).await;
            assert!(drain(&mut rx).is_empty());
            assert!(!f
                .state
                .jobs
                .worker_subscriptions
                .contains_key(&(connection, f.id())));
            for frame in [
                &snapshot,
                &event,
                &rejection,
                &ServerToWeb::WorkerInboxChanged {},
            ] {
                assert!(!web_outbound_allowed(&f.state, connection, frame).await);
            }
        }
        let (connection, mut rx) = connect(&f, &[AUTONOMOUS_WORKERS_CAPABILITY], None);
        subscribe(&f.state, connection, f.project(), f.id(), 0).await;
        assert_eq!(drain(&mut rx).len(), 2);
        f.state
            .sessions
            .set_web_capabilities(connection, vec![AUTONOMOUS_JOBS_CAPABILITY.into()]);
        fanout(&f.state, f.id()).await;
        assert!(drain(&mut rx).is_empty());
        assert!(!f
            .state
            .jobs
            .worker_subscriptions
            .contains_key(&(connection, f.id())));
        assert!(!f
            .state
            .jobs
            .worker_subscription_cursors
            .contains_key(&(connection, f.id())));
    }

    #[tokio::test]
    async fn replay_is_ordered_and_fanout_never_repeats_committed_events() {
        let mut f = Fixture::new().await;
        f.append().await;
        let (connection, mut rx) = connect(&f, &[AUTONOMOUS_WORKERS_CAPABILITY], None);
        subscribe(&f.state, connection, f.project(), f.id(), 0).await;
        let frames = drain(&mut rx);
        assert!(matches!(&frames[0], ServerToWeb::WorkerSnapshot { detail }
            if detail.worker.id == f.id() && detail.worker.high_watermark == 2));
        let sequences: Vec<_> = frames
            .iter()
            .filter_map(|frame| match frame {
                ServerToWeb::WorkerEvent { event } => Some(event.seq),
                _ => None,
            })
            .collect();
        assert_eq!(sequences, vec![1, 2]);
        assert_eq!(
            *f.state
                .jobs
                .worker_subscription_cursors
                .get(&(connection, f.id()))
                .unwrap(),
            2
        );
        f.append().await;
        fanout(&f.state, f.id()).await;
        let frames = drain(&mut rx);
        assert!(matches!(frames.as_slice(), [
            ServerToWeb::WorkerEvent { event }, ServerToWeb::WorkerInboxChanged {}
        ] if event.seq == 3 && event.worker_id == f.id()));
        fanout(&f.state, f.id()).await;
        assert!(matches!(
            drain(&mut rx).as_slice(),
            [ServerToWeb::WorkerInboxChanged {}]
        ));
        unsubscribe(&f.state, connection, f.id());
        assert!(!f
            .state
            .jobs
            .worker_subscriptions
            .contains_key(&(connection, f.id())));
        assert!(!f
            .state
            .jobs
            .worker_subscription_cursors
            .contains_key(&(connection, f.id())));
    }

    #[tokio::test]
    async fn retained_cursor_gaps_and_future_cursors_require_snapshot_resynchronization() {
        let mut f = Fixture::new().await;
        f.append().await;
        f.append().await;
        // Model the retained journal boundary without changing its durable
        // snapshot/watermark, exactly as replay retention does.
        let mut tx = f.state.db.job_transaction().await.unwrap();
        sqlx::query("DELETE FROM autonomous_worker_events WHERE worker_id=? AND seq<=1")
            .bind(f.id().to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let (connection, mut rx) = connect(&f, &[AUTONOMOUS_WORKERS_CAPABILITY], None);
        for after in [0, 4] {
            subscribe(&f.state, connection, f.project(), f.id(), after).await;
            let frames = drain(&mut rx);
            assert!(matches!(&frames[0], ServerToWeb::WorkerSnapshot { detail }
                if detail.worker.high_watermark == 3));
            assert_rejected(&frames, f.id(), "snapshot_resync_required", true);
            assert!(!f
                .state
                .jobs
                .worker_subscriptions
                .contains_key(&(connection, f.id())));
            assert!(!f
                .state
                .jobs
                .worker_subscription_cursors
                .contains_key(&(connection, f.id())));
        }
        subscribe(&f.state, connection, f.project(), f.id(), 1).await;
        let frames = drain(&mut rx);
        assert!(matches!(frames.as_slice(), [
            ServerToWeb::WorkerSnapshot { .. },
            ServerToWeb::WorkerEvent { event: second },
            ServerToWeb::WorkerEvent { event: third },
        ] if second.seq == 2 && third.seq == 3));
        subscribe(&f.state, connection, f.project(), f.id(), 3).await;
        assert!(matches!(
            drain(&mut rx).as_slice(),
            [ServerToWeb::WorkerSnapshot { .. }]
        ));
        assert_eq!(
            *f.state
                .jobs
                .worker_subscription_cursors
                .get(&(connection, f.id()))
                .unwrap(),
            3
        );
    }

    #[tokio::test]
    async fn queued_content_and_subscriptions_reauthorize_devices_and_accounts() {
        let mut f = Fixture::new().await;
        let device = f.device().await;
        let (mobile, mut mobile_rx) = connect(&f, &[AUTONOMOUS_WORKERS_CAPABILITY], Some(&device));
        let (browser, mut browser_rx) = connect(&f, &[AUTONOMOUS_WORKERS_CAPABILITY], None);
        for connection in [mobile, browser] {
            subscribe(&f.state, connection, f.project(), f.id(), 0).await;
        }
        let queued_snapshot = drain(&mut mobile_rx).remove(0);
        drain(&mut browser_rx);
        assert!(web_outbound_allowed(&f.state, mobile, &queued_snapshot).await);
        f.append().await;
        fanout(&f.state, f.id()).await;
        let queued = drain(&mut mobile_rx);
        assert!(queued
            .iter()
            .any(|frame| matches!(frame, ServerToWeb::WorkerEvent { .. })));
        assert!(queued
            .iter()
            .any(|frame| matches!(frame, ServerToWeb::WorkerInboxChanged {})));
        drain(&mut browser_rx);
        f.state
            .db
            .revoke_mobile_device_session(&f.owner().to_string(), &device, false, "test")
            .await
            .unwrap();
        for frame in queued.iter().chain(std::iter::once(&queued_snapshot)) {
            assert!(!web_outbound_allowed(&f.state, mobile, frame).await);
            assert!(web_outbound_allowed(&f.state, browser, frame).await);
        }
        reconcile_subscriptions(&f.state).await.unwrap();
        assert_rejected(
            &drain(&mut mobile_rx),
            f.id(),
            "worker_access_denied",
            false,
        );
        assert!(!f
            .state
            .jobs
            .worker_subscriptions
            .contains_key(&(mobile, f.id())));
        assert!(f
            .state
            .jobs
            .worker_subscriptions
            .contains_key(&(browser, f.id())));
        let retained = f.state.db.worker(f.id()).await.unwrap().unwrap();
        assert!(retained.grant_active);
        assert_eq!(retained.detail.worker.state, WorkerState::Idle);
        assert_eq!(retained.schedule_generation, f.worker.schedule_generation);
        f.state
            .db
            .update_cluster_user_status(
                crate::db::SYSTEM_ADMIN_ACTOR,
                &f.owner().to_string(),
                crate::db::AccountStatus::Suspended,
            )
            .await
            .unwrap();
        for frame in queued.iter().chain(std::iter::once(&queued_snapshot)) {
            assert!(!web_outbound_allowed(&f.state, browser, frame).await);
        }
        reconcile_subscriptions(&f.state).await.unwrap();
        assert_rejected(
            &drain(&mut browser_rx),
            f.id(),
            "worker_access_denied",
            false,
        );
        assert!(!f
            .state
            .jobs
            .worker_subscription_cursors
            .contains_key(&(browser, f.id())));
        f.state
            .db
            .update_cluster_user_status(
                crate::db::SYSTEM_ADMIN_ACTOR,
                &f.owner().to_string(),
                crate::db::AccountStatus::Active,
            )
            .await
            .unwrap();
        // Restoring account access does not resurrect a removed subscription,
        // and it does not un-revoke the old mobile session.
        assert!(!f
            .state
            .jobs
            .worker_subscriptions
            .contains_key(&(browser, f.id())));
        assert!(!web_outbound_allowed(&f.state, mobile, &queued_snapshot).await);
        subscribe(
            &f.state,
            browser,
            f.project(),
            f.id(),
            f.worker.detail.worker.high_watermark,
        )
        .await;
        assert!(matches!(
            drain(&mut browser_rx).as_slice(),
            [ServerToWeb::WorkerSnapshot { .. }]
        ));
    }
}
