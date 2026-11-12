use super::*;
use crate::db::{Database, Session, User};
use crate::session::AttachOutcome;
use futures::{SinkExt, StreamExt};
use shared::{
    DaemonToServer, MachineInfo, MachineProjectInfo, ProjectProvisioningMode, ServerToCli,
    WebToServer, PROJECT_INSTANCES_CAPABILITY, PROJECT_POLICY_CAPABILITY,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const SECRET: &str = "project-join-test-secret";
const REPO: &str = "github.com/owner/repo";
/// What a daemon able to run a project beside other machines advertises.
const CAPABLE_DAEMON: &[&str] = &[PROJECT_INSTANCES_CAPABILITY, PROJECT_POLICY_CAPABILITY];

struct Fixture {
    state: AppState,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("apas-project-join-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("apas.db").to_string_lossy().into_owned();
        let db = Database::new(&path).await.unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = path;
        config.auth.jwt_secret = SECRET.to_string();
        Self {
            state: AppState::new(db, config),
            dir,
        }
    }

    async fn user(&self) -> Uuid {
        let id = Uuid::new_v4();
        self.state
            .db
            .create_user(&User {
                id: id.to_string(),
                email: email(id),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
        id
    }

    /// A project `owner` registered, so owns and hosts.
    async fn project(&self, owner: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        self.state
            .db
            .authorize_project_registration(&id.to_string(), &owner.to_string())
            .await
            .unwrap();
        id
    }

    /// The project's session, run under `user`, recording `git_remote`.
    async fn session(&self, project: Uuid, user: Uuid, git_remote: Option<&str>) {
        self.state
            .db
            .create_session(&Session {
                id: project.to_string(),
                user_id: user.to_string(),
                cli_client_id: None,
                working_dir: Some(format!("/work/{project}")),
                hostname: Some("zoo-005".to_string()),
                status: "inactive".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some(project.to_string()),
                git_remote: git_remote.map(str::to_string),
                git_remote_url: None,
            })
            .await
            .unwrap();
    }

    fn token(&self, user: Uuid) -> String {
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &crate::routes::auth::Claims {
                sub: user.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
                credential_version: None,
            },
            &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap()
    }

    /// `user` asks for their projects that use `git_remote`, as `apas
    /// register` does.
    async fn lookup_raw(
        &self,
        user: Uuid,
        git_remote: &str,
        project_id: Option<&str>,
    ) -> Result<ProjectsByRemoteResponse, AppError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", self.token(user)).parse().unwrap(),
        );
        projects_by_remote(
            State(self.state.clone()),
            headers,
            Query(ProjectsByRemoteQuery {
                git_remote: Some(git_remote.to_string()),
                project_id: project_id.map(str::to_string),
            }),
        )
        .await
        .map(|Json(answer)| answer)
    }

    async fn lookup(
        &self,
        user: Uuid,
        git_remote: &str,
        project_id: Option<Uuid>,
    ) -> ProjectsByRemoteResponse {
        self.lookup_raw(
            user,
            git_remote,
            project_id.map(|id| id.to_string()).as_deref(),
        )
        .await
        .unwrap()
    }

    /// A connected daemon of `owner`'s on `machine_id`, listing `projects`.
    fn daemon(
        &self,
        owner: Uuid,
        machine_id: Uuid,
        host: &str,
        projects: Vec<MachineProjectInfo>,
        capabilities: &[&str],
    ) -> mpsc::Receiver<ServerToDaemon> {
        let (tx, rx) = mpsc::channel(16);
        self.state.sessions.register_daemon(
            machine_id,
            owner,
            tx,
            machine(machine_id, host),
            projects,
        );
        self.state.sessions.set_daemon_capabilities(
            machine_id,
            capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        );
        rx
    }

    /// A web connection of `user`'s.
    fn web(&self, user: Uuid) -> (Uuid, mpsc::Receiver<ServerToWeb>) {
        let connection = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(64);
        self.state.sessions.register_web(connection, tx);
        self.state.sessions.set_web_user(connection, user);
        self.state.sessions.set_web_capabilities(
            connection,
            vec![
                PROJECT_INSTANCES_CAPABILITY.to_string(),
                PROJECT_POLICY_CAPABILITY.to_string(),
            ],
        );
        (connection, rx)
    }

    /// An instance of `project` attached through a CLI of `user`'s, on
    /// `machine_id` (none for an older or standalone CLI).
    fn instance(
        &self,
        project: Uuid,
        user: Uuid,
        machine_id: Option<Uuid>,
        host: &str,
        path: &str,
    ) -> mpsc::Receiver<ServerToCli> {
        let cli = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(16);
        self.state.sessions.register_cli(cli, user, tx, None);
        if machine_id.is_some() {
            self.state
                .sessions
                .set_cli_capabilities(cli, vec![PROJECT_INSTANCES_CAPABILITY.to_string()]);
        }
        assert!(matches!(
            self.state.sessions.attach_cli_instance(
                project,
                cli,
                machine_id,
                Some(path.to_string()),
                Some(host.to_string()),
            ),
            AttachOutcome::Attached { .. }
        ));
        rx
    }
}

fn email(user: Uuid) -> String {
    format!("{user}@project-join.test")
}

fn machine(machine_id: Uuid, host: &str) -> MachineInfo {
    MachineInfo {
        machine_id,
        hostname: host.to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        daemon_version: Some("test".to_string()),
        deepseek_backend: None,
        last_seen: None,
    }
}

/// A checkout registered on a machine, as its daemon reports it.
fn registered(project: Uuid, path: &str, git_remote: Option<&str>) -> MachineProjectInfo {
    MachineProjectInfo {
        project_id: project.to_string(),
        name: Some("repo".to_string()),
        path: path.to_string(),
        preferred_machine_id: None,
        is_running: false,
        pid: None,
        memory_kb: None,
        last_error: None,
        git_remote: git_remote.map(str::to_string),
    }
}

fn ids(projects: &[ProjectRemoteMatch]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = projects.iter().map(|project| project.project_id).collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

fn checkouts_of(project: &ProjectRemoteMatch) -> Vec<(Option<Uuid>, &str, &str, bool)> {
    project
        .instances
        .iter()
        .map(|checkout| {
            (
                checkout.machine_id,
                checkout.hostname.as_str(),
                checkout.path.as_str(),
                checkout.running,
            )
        })
        .collect()
}

fn drain<T>(rx: &mut mpsc::Receiver<T>) -> Vec<T> {
    let mut received = Vec::new();
    while let Ok(message) = rx.try_recv() {
        received.push(message);
    }
    received
}

fn join(machine_id: Uuid, project_id: Uuid, request_id: Option<&str>) -> JoinRequest {
    JoinRequest {
        machine_id,
        cluster_owner_user_id: None,
        git_remote: REPO.to_string(),
        instance_name: "repo".to_string(),
        branch: String::new(),
        clone_url: Some("https://github.com/owner/repo.git".to_string()),
        base_path: Some("~/checkouts".to_string()),
        request_id: request_id.map(str::to_string),
        project_id,
    }
}

/// The server canonicalizes a remote exactly as the CLI does, on the CLI's
/// own examples.
#[test]
fn canonical_git_remote_matches_the_cli() {
    for (raw, canonical) in [
        ("git@github.com:Owner/Repo.git", "github.com/owner/repo"),
        ("https://github.com/owner/repo.git", "github.com/owner/repo"),
        ("ssh://git@github.com/owner/repo", "github.com/owner/repo"),
        ("https://github.com/Owner/Repo/", "github.com/owner/repo"),
        (
            "ssh://git@github.com:22/owner/repo.git",
            "github.com/owner/repo",
        ),
        (
            "git@gitlab.example.com:team/sub/proj.git",
            "gitlab.example.com/team/sub/proj",
        ),
        (
            "https://github.com/owner/repo.git/",
            "github.com/owner/repo",
        ),
        ("git@github.com:owner/repo.git/", "github.com/owner/repo"),
        ("ssh://git@[::1]:22/owner/repo.git", "::1/owner/repo"),
        ("  github.com/Owner/Repo  ", "github.com/owner/repo"),
        ("/srv/git/repo.git", "srv/git/repo"),
        ("", ""),
        ("/", ""),
    ] {
        assert_eq!(canonical_git_remote(raw), canonical, "{raw:?}");
    }
    assert_ne!(
        canonical_git_remote("ssh://git@[::1]:22/owner/repo"),
        canonical_git_remote("ssh://git@[::2]:22/owner/repo")
    );
}

/// Only projects the caller owns or hosts match, by the remote one of their
/// sessions recorded. Hosting by placement counts, and so does ownership
/// alone; belonging to a project, a stranger's project, another repository
/// and a project being deleted do not. A suspended project is still the
/// caller's to join.
#[tokio::test]
async fn matches_projects_the_caller_owns_or_hosts_by_their_sessions_remote() {
    let f = Fixture::new().await;
    let (caller, other, stranger) = (f.user().await, f.user().await, f.user().await);

    let owned = f.project(caller).await;
    f.session(owned, caller, Some(REPO)).await;

    // Another account's project, hosted in the caller's cluster.
    let hosted = f.project(other).await;
    f.session(hosted, other, Some(REPO)).await;
    assert!(f
        .state
        .db
        .add_project_cluster_placement(
            &hosted.to_string(),
            &caller.to_string(),
            &caller.to_string(),
            "test",
        )
        .await
        .unwrap());

    // Shared with the caller, who neither owns nor hosts it.
    let shared_with = f.project(other).await;
    f.session(shared_with, other, Some(REPO)).await;
    f.state
        .db
        .add_project_member(
            &other.to_string(),
            &shared_with.to_string(),
            &caller.to_string(),
        )
        .await
        .unwrap();

    let strangers = f.project(stranger).await;
    f.session(strangers, stranger, Some(REPO)).await;

    let other_repository = f.project(caller).await;
    f.session(other_repository, caller, Some("github.com/owner/other"))
        .await;

    let suspended = f.project(caller).await;
    f.session(suspended, caller, Some(REPO)).await;
    assert!(f
        .state
        .db
        .set_project_lifecycle(
            &caller.to_string(),
            &suspended.to_string(),
            ProjectLifecycle::Suspended,
        )
        .await
        .unwrap());

    let deleting = f.project(caller).await;
    f.session(deleting, caller, Some(REPO)).await;
    assert!(f
        .state
        .db
        .begin_project_deletion(
            &caller.to_string(),
            &deleting.to_string(),
            &deleting.to_string()
        )
        .await
        .unwrap());

    // Ownership alone: handed to the caller, whose cluster never hosted it.
    let transferred = f.project(other).await;
    f.session(transferred, other, Some(REPO)).await;
    f.state
        .db
        .add_project_member(
            &other.to_string(),
            &transferred.to_string(),
            &caller.to_string(),
        )
        .await
        .unwrap();
    assert!(f
        .state
        .db
        .transfer_project_ownership_by_owner(
            &other.to_string(),
            &transferred.to_string(),
            &caller.to_string(),
        )
        .await
        .unwrap());
    assert!(!f
        .state
        .db
        .project_in_user_cluster(&transferred.to_string(), &caller.to_string())
        .await
        .unwrap());

    let answer = f.lookup(caller, REPO, None).await;
    assert_eq!(answer.git_remote, REPO);
    assert_eq!(
        ids(&answer.projects),
        sorted(vec![owned, hosted, suspended, transferred])
    );
    assert!(answer.projects.iter().all(|project| project.has_session));
    assert!(answer.local.is_none());

    // Each account sees its own: the stranger theirs, and the account that
    // shared one project with the caller and handed over another those it
    // owns, and the handed-over one, which its cluster still hosts.
    assert_eq!(
        ids(&f.lookup(stranger, REPO, None).await.projects),
        vec![strangers]
    );
    assert_eq!(
        ids(&f.lookup(other, REPO, None).await.projects),
        sorted(vec![hosted, shared_with, transferred])
    );
}

/// A registered project that never started has no session to carry its
/// remote, so the caller's own machines' registrations match it, even before
/// the server holds any record of it. A record naming another owner wins
/// over a registration, and another account's machines never count, in
/// either direction.
#[tokio::test]
async fn matches_registered_projects_through_the_callers_own_machines() {
    let f = Fixture::new().await;
    let (caller, stranger) = (f.user().await, f.user().await);
    let (laptop, theirs) = (Uuid::new_v4(), Uuid::new_v4());

    // `apas register` in a checkout, never started: no record on the server.
    let pristine = Uuid::new_v4();
    // On the caller's machine, but the server records it as the stranger's.
    let foreign = f.project(stranger).await;
    // The caller's project, registered only on the stranger's machine.
    let mine_elsewhere = f.project(caller).await;
    let other_repository = Uuid::new_v4();
    let _laptop = f.daemon(
        caller,
        laptop,
        "laptop",
        vec![
            registered(pristine, "/home/me/repo", Some(REPO)),
            registered(foreign, "/home/me/foreign", Some(REPO)),
            registered(
                other_repository,
                "/home/me/other",
                Some("github.com/owner/other"),
            ),
        ],
        &[],
    );
    let _theirs = f.daemon(
        stranger,
        theirs,
        "theirs",
        vec![registered(mine_elsewhere, "/home/them/repo", Some(REPO))],
        &[],
    );

    let answer = f.lookup(caller, REPO, None).await;
    assert_eq!(ids(&answer.projects), vec![pristine]);
    let project = &answer.projects[0];
    assert_eq!(project.name.as_deref(), Some("repo"));
    assert!(!project.has_session);
    assert_eq!(
        checkouts_of(project),
        vec![(Some(laptop), "laptop", "/home/me/repo", false)]
    );
    assert!(f.lookup(stranger, REPO, None).await.projects.is_empty());

    // A machine that disconnected still says where its checkout is.
    f.state.sessions.unregister_daemon(&laptop);
    assert_eq!(
        ids(&f.lookup(caller, REPO, None).await.projects),
        vec![pristine]
    );
}

/// `local` answers for the checkout's own identity whatever remote its
/// sessions recorded, and only to an account that owns or hosts it, so the
/// CLI never re-identifies a project with history. When its remote matches
/// it is listed in `projects` too, and the CLI drops it there.
#[tokio::test]
async fn local_answers_for_the_checkouts_own_identity_whatever_its_remote() {
    let f = Fixture::new().await;
    let (caller, other) = (f.user().await, f.user().await);

    // History under another remote, as after its origin changed.
    let moved = f.project(caller).await;
    f.session(moved, caller, Some("github.com/owner/old-name"))
        .await;
    let answer = f.lookup(caller, REPO, Some(moved)).await;
    let local = answer.local.expect("the caller's own project is answered");
    assert_eq!(local.project_id, moved);
    assert!(local.has_session);
    assert!(answer.projects.is_empty());

    // An older CLI recorded no remote at all.
    let unrecorded = f.project(caller).await;
    f.session(unrecorded, caller, None).await;
    assert!(
        f.lookup(caller, REPO, Some(unrecorded))
            .await
            .local
            .expect("answered")
            .has_session
    );

    // Registered on the caller's machine and never started: no history.
    let pristine = Uuid::new_v4();
    let _laptop = f.daemon(
        caller,
        Uuid::new_v4(),
        "laptop",
        vec![registered(pristine, "/home/me/repo", Some(REPO))],
        &[],
    );
    let answer = f.lookup(caller, REPO, Some(pristine)).await;
    assert!(
        !answer
            .local
            .expect("registered with the caller")
            .has_session
    );
    assert_eq!(ids(&answer.projects), vec![pristine]);

    // A project being deleted is no longer offered, but its history counts.
    let deleting = f.project(caller).await;
    f.session(deleting, caller, Some(REPO)).await;
    f.state
        .db
        .begin_project_deletion(
            &caller.to_string(),
            &deleting.to_string(),
            &deleting.to_string(),
        )
        .await
        .unwrap();
    let answer = f.lookup(caller, REPO, Some(deleting)).await;
    assert!(answer.local.expect("still the caller's").has_session);
    assert!(!ids(&answer.projects).contains(&deleting));

    // Someone else's project is not answered, even one shared with the
    // caller, and neither is an unknown id or a query naming none.
    let shared_with = f.project(other).await;
    f.session(shared_with, other, Some(REPO)).await;
    f.state
        .db
        .add_project_member(
            &other.to_string(),
            &shared_with.to_string(),
            &caller.to_string(),
        )
        .await
        .unwrap();
    assert!(f
        .lookup(caller, REPO, Some(shared_with))
        .await
        .local
        .is_none());
    assert!(f
        .lookup(caller, REPO, Some(Uuid::new_v4()))
        .await
        .local
        .is_none());
    assert!(f.lookup(caller, REPO, None).await.local.is_none());
}

/// The remote as the CLI sends it and other spellings of the same URL find
/// what sessions recorded, and the answer names the canonical form. An IPv6
/// host's canonical form, which canonicalizing again would change, matches
/// as sent. A query without a repository, or with a project id that is not
/// one, is a bad request.
#[tokio::test]
async fn lookups_compare_canonical_remotes() {
    let f = Fixture::new().await;
    let caller = f.user().await;
    let project = f.project(caller).await;
    f.session(project, caller, Some(REPO)).await;
    for spelling in [
        REPO,
        "git@github.com:Owner/Repo.git",
        "https://github.com/owner/repo",
        "GitHub.com/Owner/Repo/",
    ] {
        let answer = f.lookup(caller, spelling, None).await;
        assert_eq!(answer.git_remote, REPO, "{spelling}");
        assert_eq!(ids(&answer.projects), vec![project], "{spelling}");
    }

    let ipv6_url = "ssh://git@[::1]:22/owner/repo.git";
    let ipv6 = canonical_git_remote(ipv6_url);
    let ipv6_project = f.project(caller).await;
    f.session(ipv6_project, caller, Some(&ipv6)).await;
    assert_eq!(
        ids(&f.lookup(caller, &ipv6, None).await.projects),
        vec![ipv6_project]
    );
    assert_eq!(
        ids(&f.lookup(caller, ipv6_url, None).await.projects),
        vec![ipv6_project]
    );

    for empty in ["", "   ", "/", ".git"] {
        assert!(
            matches!(
                f.lookup_raw(caller, empty, None).await,
                Err(AppError::BadRequest(_))
            ),
            "{empty:?}"
        );
    }
    assert!(matches!(
        f.lookup_raw(caller, REPO, Some("not-a-project")).await,
        Err(AppError::BadRequest(_))
    ));
}

/// Each checkout is listed once, with whether it runs. The caller's machines
/// that list the project come from their registrations; an attached instance
/// marks its machine's entry running, adds its own on a machine the caller
/// does not own, and is matched by host and path when it reported no
/// machine.
#[tokio::test]
async fn lists_each_checkout_once_with_whether_it_runs() {
    let f = Fixture::new().await;
    let (caller, host) = (f.user().await, f.user().await);
    let path = "/home/me/repo";

    // On a home zoo-005 and zoo-006 share, running on zoo-006 only.
    let shared_home = f.project(caller).await;
    f.session(shared_home, caller, Some(REPO)).await;
    let (zoo5, zoo6) = (Uuid::new_v4(), Uuid::new_v4());
    let _zoo5 = f.daemon(
        caller,
        zoo5,
        "zoo-005",
        vec![registered(shared_home, path, Some(REPO))],
        &[],
    );
    let _zoo6 = f.daemon(
        caller,
        zoo6,
        "zoo-006",
        vec![registered(shared_home, path, Some(REPO))],
        &[],
    );
    let _zoo6_cli = f.instance(shared_home, caller, Some(zoo6), "zoo-006", path);

    // The caller's project, hosted and run in another account's cluster.
    let hosted_elsewhere = f.project(caller).await;
    f.session(hosted_elsewhere, host, Some(REPO)).await;
    let theirs = Uuid::new_v4();
    let _theirs = f.daemon(host, theirs, "theirs", Vec::new(), &[]);
    let _theirs_cli = f.instance(hosted_elsewhere, host, Some(theirs), "theirs", "/srv/repo");

    // Run by an older CLI that reported no machine.
    let older = f.project(caller).await;
    f.session(older, caller, Some(REPO)).await;
    let laptop = Uuid::new_v4();
    let _laptop = f.daemon(
        caller,
        laptop,
        "laptop",
        vec![registered(older, "/home/me/older", Some(REPO))],
        &[],
    );
    let _older_cli = f.instance(older, caller, None, "Laptop", "/home/me/older/");

    let answer = f.lookup(caller, REPO, None).await;
    let checkouts = |project: Uuid| {
        checkouts_of(
            answer
                .projects
                .iter()
                .find(|candidate| candidate.project_id == project)
                .expect("listed"),
        )
        .into_iter()
        .map(|(machine, host, path, running)| {
            (machine, host.to_string(), path.to_string(), running)
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(
        checkouts(shared_home),
        vec![
            (Some(zoo6), "zoo-006".to_string(), path.to_string(), true),
            (Some(zoo5), "zoo-005".to_string(), path.to_string(), false),
        ]
    );
    assert_eq!(
        checkouts(hosted_elsewhere),
        vec![(
            Some(theirs),
            "theirs".to_string(),
            "/srv/repo".to_string(),
            true
        )]
    );
    assert_eq!(
        checkouts(older),
        vec![(
            Some(laptop),
            "laptop".to_string(),
            "/home/me/older".to_string(),
            true
        )]
    );
}

/// The lookup answers JSON through the real router, beside the project
/// routes that take a project id in the same place, and only to an account.
/// No match is an empty answer rather than a 404, which the CLI would take
/// for a server without this route.
#[tokio::test]
async fn the_lookup_is_routed_and_answers_json() {
    let f = Fixture::new().await;
    let caller = f.user().await;
    let project = f.project(caller).await;
    f.session(project, caller, Some(REPO)).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::routes::create_router(f.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{address}/projects/by-remote");
    let token = f.token(caller);

    let response = client
        .get(&url)
        .bearer_auth(&token)
        .query(&[
            ("git_remote", REPO.to_string()),
            ("project_id", project.to_string()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response.headers()[reqwest::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    let answer: ProjectsByRemoteResponse = response.json().await.unwrap();
    assert_eq!(ids(&answer.projects), vec![project]);
    assert_eq!(answer.local.map(|local| local.project_id), Some(project));

    let response = client
        .get(&url)
        .bearer_auth(&token)
        .query(&[("git_remote", "github.com/owner/unknown")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response
        .json::<ProjectsByRemoteResponse>()
        .await
        .unwrap()
        .projects
        .is_empty());

    let unauthenticated = client
        .get(&url)
        .query(&[("git_remote", REPO)])
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);
    let without_remote = client.get(&url).bearer_auth(&token).send().await.unwrap();
    assert_eq!(without_remote.status(), reqwest::StatusCode::BAD_REQUEST);

    let usage = client
        .get(format!("http://{address}/projects/{project}/usage"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(usage.status(), reqwest::StatusCode::OK);
    server.abort();
}

/// A capable daemon's canonical remote for each project it lists survives
/// every machine listing registration-only entries are built from: the
/// listing on request, which the mobile bootstrap uses too, the live list
/// pushed on each heartbeat, and a disconnected machine's last inventory.
#[tokio::test]
async fn machine_listings_keep_each_projects_git_remote() {
    let f = Fixture::new().await;
    let owner = f.user().await;
    let laptop = Uuid::new_v4();
    let project = Uuid::new_v4();
    let (_connection, mut web) = f.web(owner);
    let _daemon = f.daemon(
        owner,
        laptop,
        "laptop",
        vec![registered(project, "/home/me/repo", Some(REPO))],
        &[],
    );
    let remote = |machines: &[MachineWithProjects]| {
        machines
            .iter()
            .find(|entry| entry.machine.machine_id == laptop)
            .and_then(|entry| {
                entry
                    .projects
                    .iter()
                    .find(|listed| listed.project_id == project.to_string())
            })
            .and_then(|listed| listed.git_remote.clone())
    };
    assert_eq!(
        remote(&f.state.sessions.get_machines_for_user(&owner)).as_deref(),
        Some(REPO)
    );
    assert_eq!(
        remote(&crate::routes::ws_web::list_accessible_machines_for_user(&f.state, &owner).await)
            .as_deref(),
        Some(REPO)
    );

    drain(&mut web);
    let renamed = "github.com/owner/renamed";
    f.state.sessions.update_daemon_projects(
        &laptop,
        vec![registered(project, "/home/me/repo", Some(renamed))],
    );
    let pushed = drain(&mut web)
        .into_iter()
        .rev()
        .find_map(|message| match message {
            ServerToWeb::Machines { machines } => Some(machines),
            _ => None,
        })
        .expect("the heartbeat pushes the machine list");
    assert_eq!(remote(&pushed).as_deref(), Some(renamed));

    f.state.sessions.unregister_daemon(&laptop);
    assert_eq!(
        remote(&f.state.sessions.get_machines_for_user(&owner)).as_deref(),
        Some(renamed)
    );
}

/// A clone-to-join from the owner of a connected, capable machine reaches
/// its daemon as a join of that project: the project's identity, the
/// owner's own provisioning, and nothing for the requester to read but the
/// daemon's answer when it comes.
#[tokio::test]
async fn a_join_onto_the_owners_own_machine_reaches_its_daemon() {
    let f = Fixture::new().await;
    let owner = f.user().await;
    let project = f.project(owner).await;
    f.session(project, owner, Some(REPO)).await;
    let laptop = Uuid::new_v4();
    let mut daemon = f.daemon(owner, laptop, "laptop", Vec::new(), CAPABLE_DAEMON);
    let (connection, mut web) = f.web(owner);

    create_joined_instance(
        &f.state,
        &connection,
        owner,
        join(laptop, project, Some("clone-1")),
    )
    .await;
    let sent = drain(&mut daemon);
    let [ServerToDaemon::CreateProjectInstance {
        git_remote,
        instance_name,
        branch: _,
        clone_url,
        base_path,
        request_id,
        project_id,
        provisioning_mode,
        join_existing,
    }] = sent.as_slice()
    else {
        panic!("expected one join, got {sent:?}");
    };
    assert!(*join_existing);
    assert_eq!(project_id.as_deref(), Some(project.to_string().as_str()));
    assert_eq!(*provisioning_mode, ProjectProvisioningMode::TrustedOwner);
    assert_eq!(request_id.as_deref(), Some("clone-1"));
    assert_eq!(git_remote, REPO);
    assert_eq!(instance_name, "repo");
    assert_eq!(
        clone_url.as_deref(),
        Some("https://github.com/owner/repo.git")
    );
    assert_eq!(base_path.as_deref(), Some("~/checkouts"));
    assert!(drain(&mut web).is_empty(), "nothing was refused");

    // A request without a usable correlation id still gets one, so the
    // daemon's answer always carries an id.
    let too_long = "x".repeat(129);
    for request_id in [None, Some(""), Some(too_long.as_str())] {
        create_joined_instance(
            &f.state,
            &connection,
            owner,
            join(laptop, project, request_id),
        )
        .await;
        let sent = drain(&mut daemon);
        let [ServerToDaemon::CreateProjectInstance {
            request_id: Some(sent_id),
            ..
        }] = sent.as_slice()
        else {
            panic!("expected one join, got {sent:?}");
        };
        assert!(
            Uuid::parse_str(sent_id).is_ok(),
            "{request_id:?} -> {sent_id}"
        );
    }
}

/// Every refusal names its reason, reaches only the requesting connection as
/// the acknowledgement it waits on for that request, and sends nothing to
/// any daemon. A machine the requester has no relation to is not named.
#[tokio::test]
async fn each_refused_join_says_why_and_reaches_no_daemon() {
    let f = Fixture::new().await;
    let (owner, other, stranger) = (f.user().await, f.user().await, f.user().await);
    let project = f.project(owner).await;
    f.session(project, owner, Some(REPO)).await;

    let laptop = Uuid::new_v4();
    let mut laptop_daemon = f.daemon(owner, laptop, "laptop", Vec::new(), CAPABLE_DAEMON);
    let older = Uuid::new_v4();
    let mut older_daemon = f.daemon(
        owner,
        older,
        "older",
        Vec::new(),
        &[PROJECT_POLICY_CAPABILITY],
    );
    let offline = Uuid::new_v4();
    drop(f.daemon(owner, offline, "offline", Vec::new(), CAPABLE_DAEMON));
    f.state.sessions.unregister_daemon(&offline);
    let has_it = Uuid::new_v4();
    let mut has_it_daemon = f.daemon(
        owner,
        has_it,
        "has-it",
        vec![registered(project, "/home/me/repo", Some(REPO))],
        CAPABLE_DAEMON,
    );
    // A machine of another account's cluster the owner may use as a member.
    let shared = Uuid::new_v4();
    let mut shared_daemon = f.daemon(other, shared, "shared", Vec::new(), CAPABLE_DAEMON);
    f.state
        .db
        .add_cluster_member(&other.to_string(), &email(owner), None, None)
        .await
        .unwrap();
    let strangers = Uuid::new_v4();
    let mut strangers_daemon =
        f.daemon(stranger, strangers, "strangers", Vec::new(), CAPABLE_DAEMON);

    let shared_with = f.project(other).await;
    f.session(shared_with, other, Some(REPO)).await;
    f.state
        .db
        .add_project_member(
            &other.to_string(),
            &shared_with.to_string(),
            &owner.to_string(),
        )
        .await
        .unwrap();
    let suspended = f.project(owner).await;
    f.state
        .db
        .set_project_lifecycle(
            &owner.to_string(),
            &suspended.to_string(),
            ProjectLifecycle::Suspended,
        )
        .await
        .unwrap();
    let deleting = f.project(owner).await;
    f.state
        .db
        .begin_project_deletion(
            &owner.to_string(),
            &deleting.to_string(),
            &deleting.to_string(),
        )
        .await
        .unwrap();
    // Hosted by the owner, but its session runs under another account.
    let foreign_session = f.project(other).await;
    f.session(foreign_session, other, Some(REPO)).await;
    f.state
        .db
        .add_project_cluster_placement(
            &foreign_session.to_string(),
            &owner.to_string(),
            &owner.to_string(),
            "test",
        )
        .await
        .unwrap();

    let (connection, mut web) = f.web(owner);
    let (_bystander, mut bystander) = f.web(owner);
    let not_owned = "Only a project you own or host can be cloned onto your machines";
    let cases: Vec<(Uuid, Uuid, Option<String>, String)> = vec![
        (strangers, project, None, "That machine is not available to you".into()),
        (Uuid::new_v4(), project, None, "That machine is not available to you".into()),
        (
            shared,
            project,
            None,
            "Only your own machines can clone a project to join it, and shared is in another account's cluster".into(),
        ),
        (
            laptop,
            project,
            Some(other.to_string()),
            "Machine does not belong to the selected cluster".into(),
        ),
        (laptop, shared_with, None, not_owned.into()),
        (laptop, Uuid::new_v4(), None, not_owned.into()),
        (
            laptop,
            suspended,
            None,
            "This project is suspended, so no instance of it can start".into(),
        ),
        (laptop, deleting, None, "This project is being deleted".into()),
        (
            laptop,
            foreign_session,
            None,
            "This project runs under another account, so an instance on laptop could not join it".into(),
        ),
        (offline, project, None, "offline is offline".into()),
        (
            older,
            project,
            None,
            "older runs an older APAS that cannot run this project beside other machines. Reboot older to update it.".into(),
        ),
        (
            has_it,
            project,
            None,
            "has-it already has this project. Add the tab there instead.".into(),
        ),
    ];
    for (index, (machine_id, project_id, cluster, reason)) in cases.into_iter().enumerate() {
        let id = format!("refused-{index}");
        let mut request = join(machine_id, project_id, Some(&id));
        request.cluster_owner_user_id = cluster;
        create_joined_instance(&f.state, &connection, owner, request).await;
        let answers = drain(&mut web);
        assert!(
            matches!(
                answers.as_slice(),
                [ServerToWeb::ProjectInstanceCreated {
                    machine_id: answered,
                    request_id: Some(answered_id),
                    project_id: None,
                    error: Some(error),
                }] if *answered == machine_id && *answered_id == id && *error == reason
            ),
            "case {index}, expected {reason:?}: {answers:?}"
        );
    }
    assert!(drain(&mut bystander).is_empty());
    for daemon in [
        &mut laptop_daemon,
        &mut older_daemon,
        &mut has_it_daemon,
        &mut shared_daemon,
        &mut strangers_daemon,
    ] {
        assert!(drain(daemon).is_empty());
    }
}

async fn send(socket: &mut Socket, value: &impl serde::Serialize) {
    socket
        .send(Message::Text(serde_json::to_string(value).unwrap()))
        .await
        .unwrap();
}

async fn read<T: serde::de::DeserializeOwned>(socket: &mut Socket) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                other => panic!("unexpected websocket frame: {other:?}"),
            }
        }
    })
    .await
    .expect("timed out waiting for the server")
}

/// Every frame up to and including the first that `last` accepts.
async fn read_until(socket: &mut Socket, last: impl Fn(&ServerToWeb) -> bool) -> Vec<ServerToWeb> {
    let mut frames = Vec::new();
    loop {
        let frame: ServerToWeb = read(socket).await;
        let done = last(&frame);
        frames.push(frame);
        if done {
            return frames;
        }
    }
}

/// Clone-to-join over real sockets. The web's request reaches the daemon as
/// a join. The daemon's inventory listing the new checkout, then its
/// acknowledgement, reach the web in that order, so the machine lists the
/// checkout by the time the web asks for its first tab. The server chains
/// nothing after the acknowledgement, and refuses a second clone onto the
/// machine that now has the checkout.
#[tokio::test]
async fn a_join_over_the_sockets_is_acknowledged_after_the_inventory_lists_it() {
    let f = Fixture::new().await;
    let owner = f.user().await;
    let project = f.project(owner).await;
    f.session(project, owner, Some(REPO)).await;
    let laptop = Uuid::new_v4();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::routes::create_router(f.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let token = f.token(owner);

    let (mut daemon, _) = connect_async(format!("ws://{address}/ws/daemon"))
        .await
        .unwrap();
    send(
        &mut daemon,
        &DaemonToServer::Register {
            token: token.clone(),
            machine: machine(laptop, "laptop"),
            projects: Vec::new(),
            capabilities: CAPABLE_DAEMON
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        },
    )
    .await;
    assert!(matches!(
        read::<ServerToDaemon>(&mut daemon).await,
        ServerToDaemon::Registered { .. }
    ));
    let (mut web, _) = connect_async(format!("ws://{address}/ws/web"))
        .await
        .unwrap();
    send(
        &mut web,
        &WebToServer::Authenticate {
            token,
            capabilities: vec![
                PROJECT_INSTANCES_CAPABILITY.to_string(),
                PROJECT_POLICY_CAPABILITY.to_string(),
            ],
            client_kind: Some(shared::ClientKind::Web),
            app_version: None,
            protocol_version: None,
        },
    )
    .await;
    read_until(&mut web, |frame| {
        matches!(frame, ServerToWeb::Authenticated { .. })
    })
    .await;

    let clone = |request_id: &str| WebToServer::CreateProjectInstance {
        machine_id: laptop,
        cluster_owner_user_id: Some(owner.to_string()),
        git_remote: REPO.to_string(),
        instance_name: "repo".to_string(),
        branch: String::new(),
        clone_url: Some("https://github.com/owner/repo.git".to_string()),
        base_path: None,
        request_id: Some(request_id.to_string()),
        join_project_id: Some(project),
    };
    send(&mut web, &clone("join-1")).await;
    let ServerToDaemon::CreateProjectInstance {
        request_id,
        project_id,
        provisioning_mode,
        join_existing,
        ..
    } = read::<ServerToDaemon>(&mut daemon).await
    else {
        panic!("expected the join");
    };
    assert!(join_existing);
    assert_eq!(project_id, Some(project.to_string()));
    assert_eq!(provisioning_mode, ProjectProvisioningMode::TrustedOwner);
    assert_eq!(request_id.as_deref(), Some("join-1"));

    // The daemon's answer, in its order: the inventory with the checkout,
    // then the acknowledgement.
    let checkout = registered(project, "/home/me/apas_projects/repo", Some(REPO));
    send(
        &mut daemon,
        &DaemonToServer::Heartbeat {
            projects: vec![checkout.clone()],
        },
    )
    .await;
    send(
        &mut daemon,
        &DaemonToServer::ProjectInstanceCreated {
            request_id: Some("join-1".to_string()),
            project_id: Some(project.to_string()),
            path: Some(checkout.path.clone()),
            error: None,
        },
    )
    .await;
    let frames = read_until(&mut web, |frame| {
        matches!(frame, ServerToWeb::ProjectInstanceCreated { .. })
    })
    .await;
    assert!(
        frames.iter().any(|frame| matches!(
            frame,
            ServerToWeb::Machines { machines } if machines.iter().any(|entry| {
                entry.machine.machine_id == laptop
                    && entry
                        .projects
                        .iter()
                        .any(|listed| listed.project_id == project.to_string())
            })
        )),
        "the machine list naming the checkout comes before the acknowledgement: {frames:?}"
    );
    assert!(matches!(
        frames.last(),
        Some(ServerToWeb::ProjectInstanceCreated {
            machine_id,
            request_id: Some(id),
            project_id: Some(joined),
            error: None,
        }) if *machine_id == laptop && id == "join-1" && *joined == project.to_string()
    ));
    assert!(f
        .state
        .sessions
        .machines_listing_project(&project.to_string())
        .contains(&laptop));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), daemon.next())
            .await
            .is_err(),
        "the server starts nothing and adds no tab after a join"
    );

    send(&mut web, &clone("join-2")).await;
    let frames = read_until(&mut web, |frame| {
        matches!(frame, ServerToWeb::ProjectInstanceCreated { .. })
    })
    .await;
    assert!(matches!(
        frames.last(),
        Some(ServerToWeb::ProjectInstanceCreated {
            request_id: Some(id),
            project_id: None,
            error: Some(error),
            ..
        }) if id == "join-2" && error == "laptop already has this project. Add the tab there instead."
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), daemon.next())
            .await
            .is_err(),
        "a refused join reaches no daemon"
    );

    // `apas register` elsewhere now finds the new checkout.
    let answer = f.lookup(owner, REPO, None).await;
    assert_eq!(ids(&answer.projects), vec![project]);
    assert!(checkouts_of(&answer.projects[0]).contains(&(
        Some(laptop),
        "laptop",
        "/home/me/apas_projects/repo",
        false
    )));
    server.abort();
}
