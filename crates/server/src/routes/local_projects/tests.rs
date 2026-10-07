use super::*;
use crate::db::{AccountStatus, Database, ProjectLifecycle, User};
use futures::{SinkExt, StreamExt};
use shared::{DaemonToServer, MachineInfo, MachineProjectInfo, WebToServer};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Fixture {
    state: AppState,
    dir: PathBuf,
    owner: Uuid,
    machine: Uuid,
    connection: Uuid,
    web: mpsc::Receiver<ServerToWeb>,
    daemon: mpsc::Receiver<ServerToDaemon>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn machine(machine_id: Uuid) -> MachineInfo {
    MachineInfo {
        machine_id,
        hostname: "local-registration-test".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        daemon_version: Some("test".to_string()),
        deepseek_backend: None,
        last_seen: None,
    }
}

fn project(id: &str) -> MachineProjectInfo {
    MachineProjectInfo {
        project_id: id.to_string(),
        name: Some("Plain folder".to_string()),
        path: format!("/plain/{id}"),
        is_running: false,
        pid: None,
        memory_kb: None,
        last_error: None,
    }
}

async fn add_user(state: &AppState) -> Uuid {
    let id = Uuid::new_v4();
    state
        .db
        .create_user(&User {
            id: id.to_string(),
            email: format!("{id}@local-registration.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
    id
}

fn connect_web(state: &AppState, user: Uuid) -> (Uuid, mpsc::Receiver<ServerToWeb>) {
    let connection = Uuid::new_v4();
    let (tx, rx) = mpsc::channel(64);
    state.sessions.register_web(connection, tx);
    state.sessions.set_web_user(connection, user);
    state.sessions.set_web_capabilities(
        connection,
        vec![shared::PROJECT_POLICY_CAPABILITY.to_string()],
    );
    (connection, rx)
}

fn connect_daemon(state: &AppState, owner: Uuid, id: Uuid) -> mpsc::Receiver<ServerToDaemon> {
    let (tx, rx) = mpsc::channel(64);
    state
        .sessions
        .register_daemon(id, owner, tx, machine(id), Vec::new());
    state.sessions.set_daemon_capabilities(
        id,
        vec![
            shared::LOCAL_PROJECT_REGISTRATION_CAPABILITY.to_string(),
            shared::PROJECT_POLICY_CAPABILITY.to_string(),
        ],
    );
    rx
}

impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("apas-local-registration-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.db").to_string_lossy().into_owned();
        let db = Database::new(&path).await.unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = path;
        let state = AppState::new(db, config);
        let owner = add_user(&state).await;
        let machine = Uuid::new_v4();
        let daemon = connect_daemon(&state, owner, machine);
        let (connection, web) = connect_web(&state, owner);
        Self {
            state,
            dir,
            owner,
            machine,
            connection,
            web,
            daemon,
        }
    }

    async fn dispatch(&mut self, request: &str) -> String {
        register(
            &self.state,
            &self.connection,
            self.machine,
            None,
            "~/plain folder".to_string(),
            request.to_string(),
        )
        .await;
        match self.daemon.recv().await.unwrap() {
            ServerToDaemon::RegisterLocalProject { path, request_id } => {
                assert_eq!(path, "~/plain folder");
                assert_ne!(request_id, request);
                request_id
            }
            other => panic!("Unexpected daemon command: {other:?}"),
        }
    }

    async fn success(&self, wire_id: String, project: MachineProjectInfo) {
        self.state
            .sessions
            .update_daemon_projects(&self.machine, vec![project.clone()]);
        registered(
            &self.state,
            &self.machine,
            wire_id,
            LocalProjectRegistrationResult::Registered { project },
        )
        .await;
    }

    async fn result(&mut self, request: &str) -> LocalProjectRegistrationResult {
        result(&mut self.web, self.machine, request).await
    }
}

async fn result(
    rx: &mut mpsc::Receiver<ServerToWeb>,
    machine: Uuid,
    request: &str,
) -> LocalProjectRegistrationResult {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match rx.recv().await.unwrap() {
                ServerToWeb::LocalProjectRegistered {
                    machine_id,
                    request_id,
                    result,
                } => {
                    assert_eq!(machine_id, machine);
                    assert_eq!(request_id, request);
                    return result;
                }
                ServerToWeb::Machines { .. } => {}
                other => panic!("Unexpected web message: {other:?}"),
            }
        }
    })
    .await
    .expect("registration must have a terminal result")
}

fn failure(result: LocalProjectRegistrationResult) -> String {
    match result {
        LocalProjectRegistrationResult::Failed { error } => error,
        other => panic!("Expected failure, got {other:?}"),
    }
}

#[tokio::test]
async fn owner_registration_finalizes_identity_without_starting_or_creating_sessions() {
    let mut f = Fixture::new().await;
    let wire = f.dispatch("owner-request").await;
    let project = project("plain-no-git");
    f.success(wire, project.clone()).await;
    let LocalProjectRegistrationResult::Registered { project: confirmed } =
        f.result("owner-request").await
    else {
        panic!("owner registration should succeed");
    };
    assert_eq!(confirmed.project_id, project.project_id);
    assert_eq!(confirmed.path, project.path);
    assert!(!confirmed.is_running);
    assert!(f.daemon.try_recv().is_err());
    let canonical = f
        .state
        .db
        .get_project(&project.project_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canonical.owner_user_id, f.owner.to_string());
    assert!(f
        .state
        .db
        .project_is_placed_in_cluster(&project.project_id, &f.owner.to_string())
        .await
        .unwrap());
    assert!(
        super::super::ws_web::has_machine_project_runtime_access(
            &f.state,
            &f.owner,
            &f.machine,
            &project.project_id
        )
        .await
    );
    assert!(f
        .state
        .db
        .get_sessions_for_user(&f.owner.to_string())
        .await
        .unwrap()
        .is_empty());
    assert!(!f.state.sessions.is_project_connected(&project.project_id));
    let shared_refs = f.state.sessions.get_machines_for_project_refs(
        &std::collections::HashSet::from([(
            "local-registration-test".to_string(),
            project.path.clone(),
        )]),
        &std::collections::HashSet::new(),
    );
    assert_eq!(shared_refs.len(), 1);
    assert!(!shared_refs[0].local_project_registration_available);
}

#[tokio::test]
async fn guest_is_denied_even_with_cluster_membership_and_has_no_local_capability() {
    let mut f = Fixture::new().await;
    let guest = add_user(&f.state).await;
    let user = f
        .state
        .db
        .get_user_by_id(&guest.to_string())
        .await
        .unwrap()
        .unwrap();
    let token = Uuid::new_v4().to_string();
    f.state
        .db
        .create_shared_cluster_invitation(
            &Uuid::new_v4().to_string(),
            &token,
            &f.owner.to_string(),
            &user.email,
            &(chrono::Utc::now() + chrono::Duration::hours(1))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        )
        .await
        .unwrap();
    f.state
        .db
        .accept_shared_cluster_invitation(&token, &guest.to_string())
        .await
        .unwrap()
        .unwrap();
    let (connection, mut web) = connect_web(&f.state, guest);
    register(
        &f.state,
        &connection,
        f.machine,
        Some(&f.owner.to_string()),
        "/private/folder".to_string(),
        "guest".to_string(),
    )
    .await;
    assert!(failure(result(&mut web, f.machine, "guest").await).contains("owner"));
    assert!(f.daemon.try_recv().is_err());
    let owned = super::super::ws_web::list_accessible_machines_for_user(&f.state, &f.owner).await;
    assert!(owned[0].local_project_registration_available);
    let shared = super::super::ws_web::list_accessible_machines_for_user(&f.state, &guest).await;
    assert_eq!(shared.len(), 1);
    assert!(!shared[0].local_project_registration_available);
    let claims = crate::routes::auth::Claims {
        sub: guest.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        device_session_id: None,
        token_kind: None,
        credential_version: None,
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(f.state.config.auth.jwt_secret.as_bytes()),
    )
    .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    let axum::Json(bootstrap) =
        super::super::mobile::bootstrap(axum::extract::State(f.state.clone()), headers)
            .await
            .unwrap();
    assert_eq!(bootstrap.machines.len(), 1);
    assert!(!bootstrap.machines[0].local_project_registration_available);
    // The cached, heartbeat-driven projection used by web/mobile stays redacted.
    f.state.sessions.update_daemon_projects(&f.machine, vec![]);
    f.state.sessions.broadcast_machines_update_for_user(&guest);
    let ServerToWeb::Machines { machines } = web.try_recv().expect("guest inventory update") else {
        panic!("expected inventory update");
    };
    assert!(!machines[0].local_project_registration_available);
}

#[tokio::test]
async fn pre_dispatch_failures_are_correlated_and_never_send_paths() {
    let mut f = Fixture::new().await;
    for path in ["", "relative", "~other/folder", "/folder\0bad"] {
        register(
            &f.state,
            &f.connection,
            f.machine,
            None,
            path.to_string(),
            "path".to_string(),
        )
        .await;
        assert!(failure(f.result("path").await).contains("absolute"));
    }
    register(
        &f.state,
        &f.connection,
        f.machine,
        Some(&Uuid::new_v4().to_string()),
        "/folder".to_string(),
        "cluster".to_string(),
    )
    .await;
    assert!(failure(f.result("cluster").await).contains("selected cluster"));
    f.state.sessions.set_daemon_capabilities(f.machine, vec![]);
    register(
        &f.state,
        &f.connection,
        f.machine,
        None,
        "/folder".to_string(),
        "unsupported".to_string(),
    )
    .await;
    assert!(failure(f.result("unsupported").await).contains("Update"));
    assert!(
        !f.state.sessions.get_machines_for_user(&f.owner)[0].local_project_registration_available
    );
    f.state.sessions.unregister_daemon(&f.machine);
    register(
        &f.state,
        &f.connection,
        f.machine,
        None,
        "/folder".to_string(),
        "offline".to_string(),
    )
    .await;
    assert!(failure(f.result("offline").await).contains("offline"));
    assert!(f.daemon.try_recv().is_err());
}

#[tokio::test]
async fn suspended_account_is_checked_before_dispatch_and_again_on_completion() {
    let mut f = Fixture::new().await;
    let wire = f.dispatch("before-suspension").await;
    f.state
        .db
        .update_cluster_user_status(
            &f.owner.to_string(),
            &f.owner.to_string(),
            AccountStatus::Suspended,
        )
        .await
        .unwrap();
    register(
        &f.state,
        &f.connection,
        f.machine,
        None,
        "/folder".to_string(),
        "suspended".to_string(),
    )
    .await;
    assert!(failure(f.result("suspended").await).contains("suspended"));
    assert!(f.daemon.try_recv().is_err());
    f.success(wire, project("suspended-account-project")).await;
    assert!(failure(f.result("before-suspension").await).contains("suspended"));
    assert!(f
        .state
        .db
        .get_project("suspended-account-project")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn request_ids_are_isolated_by_connection_and_forged_results_do_not_consume_them() {
    let mut f = Fixture::new().await;
    let (second, mut second_web) = connect_web(&f.state, f.owner);
    let first_wire = f.dispatch("same-browser-id").await;
    register(
        &f.state,
        &second,
        f.machine,
        None,
        "/second".to_string(),
        "same-browser-id".to_string(),
    )
    .await;
    let ServerToDaemon::RegisterLocalProject {
        request_id: second_wire,
        ..
    } = f.daemon.recv().await.unwrap()
    else {
        panic!("register command");
    };
    assert_ne!(first_wire, second_wire);
    let wrong_machine = Uuid::new_v4();
    let _wrong_daemon = connect_daemon(&f.state, f.owner, wrong_machine);
    for (machine, wire) in [
        (wrong_machine, first_wire.clone()),
        (f.machine, "unsolicited".to_string()),
    ] {
        registered(
            &f.state,
            &machine,
            wire,
            LocalProjectRegistrationResult::Failed {
                error: "forged".to_string(),
            },
        )
        .await;
    }
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&first_wire, &f.machine)
        .is_some());
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&second_wire, &f.machine)
        .is_some());
    registered(
        &f.state,
        &f.machine,
        second_wire,
        LocalProjectRegistrationResult::Failed {
            error: "missing directory".to_string(),
        },
    )
    .await;
    assert_eq!(
        failure(result(&mut second_web, f.machine, "same-browser-id").await),
        "missing directory"
    );
    f.success(first_wire.clone(), project("first-project"))
        .await;
    assert!(matches!(
        f.result("same-browser-id").await,
        LocalProjectRegistrationResult::Registered { .. }
    ));
    registered(
        &f.state,
        &f.machine,
        first_wire,
        LocalProjectRegistrationResult::Failed {
            error: "duplicate".to_string(),
        },
    )
    .await;
    while let Ok(message) = f.web.try_recv() {
        assert!(!matches!(
            message,
            ServerToWeb::LocalProjectRegistered { .. }
        ));
    }
}

#[tokio::test]
async fn disconnect_fails_requests_and_old_results_cannot_complete_manual_retry() {
    let mut f = Fixture::new().await;
    let old_wire = f.dispatch("retry-id").await;
    f.state.sessions.unregister_daemon(&f.machine);
    assert!(failure(f.result("retry-id").await).contains("disconnected"));
    f.daemon = connect_daemon(&f.state, f.owner, f.machine);
    let new_wire = f.dispatch("retry-id").await;
    assert_ne!(old_wire, new_wire);
    f.success(old_wire, project("stale-project")).await;
    assert!(f
        .state
        .db
        .get_project("stale-project")
        .await
        .unwrap()
        .is_none());
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&new_wire, &f.machine)
        .is_some());
    f.success(new_wire, project("retry-project")).await;
    assert!(matches!(
        f.result("retry-id").await,
        LocalProjectRegistrationResult::Registered { .. }
    ));
    assert!(f.daemon.try_recv().is_err());
}

#[tokio::test]
async fn browser_disconnect_clears_requests_without_replay_or_finalization() {
    let mut f = Fixture::new().await;
    let wire = f.dispatch("closed-web").await;
    f.state.sessions.unregister_web(&f.connection);
    f.success(wire.clone(), project("closed-web-project")).await;
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&wire, &f.machine)
        .is_none());
    assert!(f
        .state
        .db
        .get_project("closed-web-project")
        .await
        .unwrap()
        .is_none());
    assert!(f.daemon.try_recv().is_err());
}

#[tokio::test]
async fn inventory_confirmation_is_required_and_runtime_fields_are_not_fabricated() {
    let mut f = Fixture::new().await;
    let wire = f.dispatch("missing-inventory").await;
    let snapshot = project("inventory-project");
    registered(
        &f.state,
        &f.machine,
        wire,
        LocalProjectRegistrationResult::Registered {
            project: snapshot.clone(),
        },
    )
    .await;
    assert!(failure(f.result("missing-inventory").await).contains("inventory"));
    assert!(f
        .state
        .db
        .get_project(&snapshot.project_id)
        .await
        .unwrap()
        .is_none());
    let wire = f.dispatch("wrong-path").await;
    let mut incorrect = snapshot.clone();
    incorrect.path = "/another/path".to_string();
    f.state
        .sessions
        .update_daemon_projects(&f.machine, vec![incorrect]);
    registered(
        &f.state,
        &f.machine,
        wire,
        LocalProjectRegistrationResult::Registered {
            project: snapshot.clone(),
        },
    )
    .await;
    assert!(failure(f.result("wrong-path").await).contains("inventory"));
    let wire = f.dispatch("running").await;
    let mut running = snapshot.clone();
    running.is_running = true;
    running.pid = Some(42);
    running.memory_kb = Some(100);
    f.state
        .sessions
        .update_daemon_projects(&f.machine, vec![running]);
    registered(
        &f.state,
        &f.machine,
        wire,
        LocalProjectRegistrationResult::Registered { project: snapshot },
    )
    .await;
    let LocalProjectRegistrationResult::Registered { project } = f.result("running").await else {
        panic!("running registration");
    };
    assert!(project.is_running);
    assert_eq!(project.pid, Some(42));
    assert_eq!(project.memory_kb, Some(100));
    assert!(f.daemon.try_recv().is_err());
}

#[tokio::test]
async fn foreign_suspended_and_deleting_identities_fail_without_cleanup_or_ownership_changes() {
    let mut f = Fixture::new().await;
    let foreign = add_user(&f.state).await;
    for (id, owner) in [
        ("foreign", foreign),
        ("suspended", f.owner),
        ("deleting", f.owner),
    ] {
        f.state
            .db
            .authorize_project_registration(id, &owner.to_string())
            .await
            .unwrap();
        if id == "suspended" {
            f.state
                .db
                .set_project_lifecycle(&owner.to_string(), id, ProjectLifecycle::Suspended)
                .await
                .unwrap();
        } else if id == "deleting" {
            f.state
                .db
                .begin_project_deletion(&owner.to_string(), id, id)
                .await
                .unwrap();
        }
        let wire = f.dispatch(id).await;
        let snapshot = project(id);
        f.success(wire, snapshot.clone()).await;
        let error = failure(f.result(id).await);
        assert!(
            error.contains("member") || error.contains("suspended") || error.contains("deletion"),
            "{error}"
        );
        assert_eq!(
            f.state
                .db
                .get_project(id)
                .await
                .unwrap()
                .unwrap()
                .owner_user_id,
            owner.to_string()
        );
        assert!(f
            .state
            .sessions
            .registered_local_project(&f.machine, id, &snapshot.path)
            .is_some());
        assert!(f
            .state
            .db
            .list_project_members(id)
            .await
            .unwrap()
            .is_empty());
        assert!(
            f.daemon.try_recv().is_err(),
            "no Start or destructive compensation command"
        );
    }
    assert!(!f
        .state
        .db
        .project_is_placed_in_cluster("foreign", &f.owner.to_string())
        .await
        .unwrap());
}

#[tokio::test]
async fn authorized_existing_foreign_identity_preserves_owner_membership_and_override() {
    let mut f = Fixture::new().await;
    let foreign = add_user(&f.state).await;
    let id = "authorized-foreign";
    f.state
        .db
        .authorize_project_registration(id, &foreign.to_string())
        .await
        .unwrap();
    f.state
        .db
        .add_project_member(&foreign.to_string(), id, &f.owner.to_string())
        .await
        .unwrap();
    f.state
        .db
        .set_project_policy_override(&foreign.to_string(), id, Some(false), Some(vec![]))
        .await
        .unwrap();
    let before = f.state.db.get_project_policy_override(id).await.unwrap();
    let before_members = f.state.db.list_project_members(id).await.unwrap();
    let wire = f.dispatch("preserve").await;
    f.success(wire, project(id)).await;
    assert!(matches!(
        f.result("preserve").await,
        LocalProjectRegistrationResult::Registered { .. }
    ));
    assert_eq!(
        f.state
            .db
            .get_project(id)
            .await
            .unwrap()
            .unwrap()
            .owner_user_id,
        foreign.to_string()
    );
    assert_eq!(
        serde_json::to_value(f.state.db.get_project_policy_override(id).await.unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(
        serde_json::to_value(f.state.db.list_project_members(id).await.unwrap()).unwrap(),
        serde_json::to_value(before_members).unwrap()
    );
    assert!(f
        .state
        .db
        .get_effective_project_policy(id)
        .await
        .unwrap()
        .allowed_launch_profiles
        .is_empty());
    assert!(f.daemon.try_recv().is_err());
}

async fn socket_send(socket: &mut Socket, value: &impl serde::Serialize) {
    socket
        .send(Message::Text(serde_json::to_string(value).unwrap().into()))
        .await
        .unwrap();
}

async fn socket_read<T: serde::de::DeserializeOwned>(socket: &mut Socket) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                other => panic!("Unexpected websocket frame: {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn websocket_no_git_registration_is_ready_for_a_subsequent_explicit_start() {
    let f = Fixture::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::routes::create_router(f.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let claims = crate::routes::auth::Claims {
        sub: f.owner.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        device_session_id: None,
        token_kind: None,
        credential_version: None,
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(f.state.config.auth.jwt_secret.as_bytes()),
    )
    .unwrap();
    let (mut daemon, _) = connect_async(format!("ws://{address}/ws/daemon"))
        .await
        .unwrap();
    socket_send(
        &mut daemon,
        &DaemonToServer::Register {
            token: token.clone(),
            machine: machine(f.machine),
            projects: vec![],
            capabilities: vec![
                shared::LOCAL_PROJECT_REGISTRATION_CAPABILITY.to_string(),
                shared::PROJECT_POLICY_CAPABILITY.to_string(),
            ],
        },
    )
    .await;
    assert!(matches!(
        socket_read::<ServerToDaemon>(&mut daemon).await,
        ServerToDaemon::Registered { .. }
    ));
    let (mut web, _) = connect_async(format!("ws://{address}/ws/web"))
        .await
        .unwrap();
    socket_send(
        &mut web,
        &WebToServer::Authenticate {
            token,
            capabilities: vec![shared::PROJECT_POLICY_CAPABILITY.to_string()],
            client_kind: Some(shared::ClientKind::Web),
            app_version: None,
            protocol_version: None,
        },
    )
    .await;
    loop {
        if matches!(
            socket_read::<ServerToWeb>(&mut web).await,
            ServerToWeb::Authenticated { .. }
        ) {
            break;
        }
    }
    let folder = f.dir.join("plain folder");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "keep this content").unwrap();
    let path = folder.to_string_lossy().into_owned();
    socket_send(
        &mut web,
        &WebToServer::RegisterLocalProject {
            machine_id: f.machine,
            cluster_owner_user_id: Some(f.owner.to_string()),
            path: path.clone(),
            request_id: "websocket-local".to_string(),
        },
    )
    .await;
    let ServerToDaemon::RegisterLocalProject {
        request_id,
        path: dispatched_path,
    } = socket_read::<ServerToDaemon>(&mut daemon).await
    else {
        panic!("registration command");
    };
    assert_eq!(dispatched_path, path);
    let mut snapshot = project("websocket-plain-folder");
    snapshot.path = path;
    // Exercise the real daemon route's inventory-before-result ordering. The
    // filesystem implementation itself is covered by isolated daemon tests.
    socket_send(
        &mut daemon,
        &DaemonToServer::Heartbeat {
            projects: vec![snapshot.clone()],
        },
    )
    .await;
    socket_send(
        &mut daemon,
        &DaemonToServer::LocalProjectRegistered {
            request_id,
            result: LocalProjectRegistrationResult::Registered {
                project: snapshot.clone(),
            },
        },
    )
    .await;
    loop {
        if let ServerToWeb::LocalProjectRegistered {
            request_id, result, ..
        } = socket_read::<ServerToWeb>(&mut web).await
        {
            assert_eq!(request_id, "websocket-local");
            assert!(matches!(
                result,
                LocalProjectRegistrationResult::Registered { .. }
            ));
            break;
        }
    }
    assert!(f
        .state
        .db
        .get_sessions_for_user(&f.owner.to_string())
        .await
        .unwrap()
        .is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), daemon.next())
            .await
            .is_err(),
        "registration never sends Start"
    );
    socket_send(
        &mut web,
        &WebToServer::StartMachineProjectCli {
            machine_id: f.machine,
            project_id: snapshot.project_id.clone(),
        },
    )
    .await;
    let ServerToDaemon::StartProjectCli { project_id, policy } =
        socket_read::<ServerToDaemon>(&mut daemon).await
    else {
        panic!("explicit Start must pass the normal authorization gate");
    };
    assert_eq!(project_id, snapshot.project_id);
    assert!(policy.is_some());
    assert!(!folder.join(".git").exists());
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.txt")).unwrap(),
        "keep this content"
    );
    web.close(None).await.unwrap();
    daemon.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn policy_resolution_failure_retains_registration_and_is_manually_retryable() {
    let mut f = Fixture::new().await;
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(f.dir.join("server.db")),
    )
    .await
    .unwrap();
    sqlx::query("ALTER TABLE project_policy_overrides RENAME TO saved_policy_overrides")
        .execute(&pool)
        .await
        .unwrap();
    let snapshot = project("policy-failure");
    let wire = f.dispatch("policy-failure").await;
    f.success(wire, snapshot.clone()).await;
    assert!(failure(f.result("policy-failure").await).contains("project_policy_overrides"));
    assert_eq!(
        f.state
            .db
            .get_project(&snapshot.project_id)
            .await
            .unwrap()
            .unwrap()
            .owner_user_id,
        f.owner.to_string()
    );
    assert!(f
        .state
        .sessions
        .registered_local_project(&f.machine, &snapshot.project_id, &snapshot.path)
        .is_some());
    assert!(
        f.daemon.try_recv().is_err(),
        "no destructive cleanup or automatic retry"
    );
    sqlx::query("ALTER TABLE saved_policy_overrides RENAME TO project_policy_overrides")
        .execute(&pool)
        .await
        .unwrap();
    let wire = f.dispatch("manual-retry").await;
    f.success(wire, snapshot).await;
    assert!(matches!(
        f.result("manual-retry").await,
        LocalProjectRegistrationResult::Registered { .. }
    ));
    assert!(f.daemon.try_recv().is_err());
    pool.close().await;
}

#[tokio::test]
async fn replacing_a_daemon_fails_only_its_requests_and_preserves_other_machines() {
    let mut f = Fixture::new().await;
    let old_sender = f.state.sessions.daemon_connection(&f.machine).unwrap();
    let old_wire = f.dispatch("replaced").await;
    let other_machine = Uuid::new_v4();
    let mut other_daemon = connect_daemon(&f.state, f.owner, other_machine);
    register(
        &f.state,
        &f.connection,
        other_machine,
        None,
        "/other".to_string(),
        "other".to_string(),
    )
    .await;
    let ServerToDaemon::RegisterLocalProject {
        request_id: other_wire,
        ..
    } = other_daemon.recv().await.unwrap()
    else {
        panic!("register command");
    };
    f.daemon = connect_daemon(&f.state, f.owner, f.machine);
    assert!(failure(f.result("replaced").await).contains("disconnected"));
    assert!(!f
        .state
        .sessions
        .is_daemon_connection(&f.machine, &old_sender));
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&old_wire, &f.machine)
        .is_none());
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&other_wire, &other_machine)
        .is_some());
    registered(
        &f.state,
        &other_machine,
        other_wire,
        LocalProjectRegistrationResult::Failed {
            error: "other directory missing".to_string(),
        },
    )
    .await;
    assert_eq!(
        failure(result(&mut f.web, other_machine, "other").await),
        "other directory missing"
    );
    assert!(
        f.daemon.try_recv().is_err(),
        "replacement never replays registration"
    );
}

#[tokio::test]
async fn disconnect_terminal_result_survives_a_full_inventory_queue() {
    let mut f = Fixture::new().await;
    let (tx, rx) = mpsc::channel(1);
    f.state.sessions.register_web(f.connection, tx);
    f.web = rx;
    let wire = f.dispatch("full-queue").await;
    assert!(
        f.state
            .sessions
            .send_to_web(&f.connection, ServerToWeb::Machines { machines: vec![] })
            .await
    );
    f.state.sessions.unregister_daemon(&f.machine);
    assert!(f
        .state
        .sessions
        .pending_local_project_registration(&wire, &f.machine)
        .is_none());
    assert!(failure(f.result("full-queue").await).contains("disconnected"));
}

#[tokio::test]
async fn closed_dispatch_channel_and_unauthenticated_requests_fail_with_correlation() {
    let mut f = Fixture::new().await;
    f.daemon.close();
    register(
        &f.state,
        &f.connection,
        f.machine,
        None,
        "/plain".to_string(),
        "closed-channel".to_string(),
    )
    .await;
    assert!(failure(f.result("closed-channel").await).contains("offline"));
    f.state.sessions.unregister_web(&f.connection);
    let (tx, rx) = mpsc::channel(4);
    f.state.sessions.register_web(f.connection, tx);
    f.web = rx;
    register(
        &f.state,
        &f.connection,
        f.machine,
        None,
        "/plain".to_string(),
        "anonymous".to_string(),
    )
    .await;
    assert!(failure(f.result("anonymous").await).contains("authenticated"));
}
