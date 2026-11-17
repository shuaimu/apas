//! A machine follows the recorded open state when its daemon registers
//! (design Decision 5): tests of `crate::open_state::follow`.

use super::*;
use crate::open_state::follow::{follow_project, follow_registered_daemon, Followed};
use shared::MachinePart;

pub(super) fn machine_info(machine_id: Uuid, host: &str) -> shared::MachineInfo {
    shared::MachineInfo {
        machine_id,
        hostname: host.to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        daemon_version: None,
        deepseek_backend: None,
        last_seen: None,
        providers: None,
    }
}

/// `machine`'s daemon connects and registers under `account`, listing the
/// project as running or not. The follow step has not run yet. Returns the
/// connection it registered with and what it is sent.
pub(super) fn connects(
    p: &Project,
    account: Uuid,
    machine: Uuid,
    host: &str,
    capabilities: &[&str],
    running: bool,
) -> (mpsc::Sender<ServerToDaemon>, Endpoint<ServerToDaemon>) {
    let (tx, rx) = mpsc::channel(16);
    p.state.sessions.register_daemon(
        machine,
        account,
        tx.clone(),
        machine_info(machine, host),
        vec![listed(p, host, running)],
    );
    p.state.sessions.set_daemon_capabilities(
        machine,
        capabilities
            .iter()
            .map(|capability| capability.to_string())
            .collect(),
    );
    (tx, Endpoint { id: machine, rx })
}

/// `machine`'s daemon registers and the server runs the follow step for
/// that registration, as `ws_daemon` does once `Registered` is written.
pub(super) async fn registers_as(
    p: &Project,
    account: Uuid,
    machine: Uuid,
    host: &str,
    capabilities: &[&str],
    running: bool,
) -> Endpoint<ServerToDaemon> {
    let (connection, daemon) = connects(p, account, machine, host, capabilities, running);
    follow_registered_daemon(&p.state, machine, &connection, vec![p.project_id()]).await;
    daemon
}

/// A capable daemon of the project's host account registers.
pub(super) async fn registers(
    p: &Project,
    machine: Uuid,
    host: &str,
    running: bool,
) -> Endpoint<ServerToDaemon> {
    registers_as(p, p.host, machine, host, CAPABLE_DAEMON, running).await
}

/// The machine's part in the project as viewers are told it, with its
/// reason. `None` when the machine is not listed.
pub(super) fn part(p: &Project, machine: Uuid) -> Option<(MachinePart, Option<String>)> {
    p.state
        .sessions
        .project_open_view(&p.project_id())?
        .machines
        .into_iter()
        .find(|listed| listed.machine_id == Some(machine))
        .map(|listed| (listed.part, listed.reason))
}

/// The one correlated start a daemon was sent, and nothing else.
pub(super) fn one_correlated_start(daemon: &mut Endpoint<ServerToDaemon>) -> Uuid {
    let (starts, stops) = sent(daemon);
    assert_eq!(stops, 0);
    match starts.as_slice() {
        [Some(request_id)] => *request_id,
        other => panic!("expected one correlated start, got {other:?}"),
    }
}

/// Scenario "A machine that was offline when the project was opened": it is
/// asked to start when its daemon registers, with a start its daemon
/// answers, and the recorded state is left as the open set it.
#[tokio::test]
async fn a_machine_that_was_offline_at_the_open_starts_when_it_registers() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;

    open(&p, &web, p.owner, p.m5).await;
    assert!(started_once(&mut d5));
    assert_eq!(part(&p, p.m6), Some((MachinePart::WaitingOffline, None)));
    let opened = record(&p).await;

    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    let request_id = one_correlated_start(&mut d6);
    assert!(matches!(
        outcome(&p, p.m6),
        Some(MachineOutcome::StartAsked { request_id: Some(asked), .. }) if asked == request_id
    ));
    assert_eq!(part(&p, p.m6), Some((MachinePart::Starting, None)));
    assert_eq!(record(&p).await, opened, "following records nothing");
    assert!(nothing_sent(&mut d5), "nothing changes on another machine");
}

/// Scenario "A machine that restarted": its daemon starts and registers
/// without running the open project, and is asked to start it again.
#[tokio::test]
async fn a_machine_that_restarted_starts_the_open_project_again() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _web = p.viewer(p.owner, KNOWS);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);
    assert!(nothing_sent(&mut d5));

    let mut restarted = registers(&p, p.m5, HOST_A, false).await;
    one_correlated_start(&mut restarted);
}

/// Scenario "A project closed while a machine was away": the machine lost
/// its connection still running the project, the close could not reach it,
/// and it is asked to stop when its daemon registers again.
#[tokio::test]
async fn a_project_closed_while_a_machine_was_disconnected_is_stopped_when_it_returns() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);
    reports_running(&p, p.m6, HOST_B);

    p.state.sessions.unregister_daemon(&p.m6);
    close(&p, &web, p.owner, p.m5).await;
    assert!(stopped_once(&mut d5));
    assert!(nothing_sent(&mut d6), "the close could not reach it");
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Closed));

    let mut returned = registers(&p, p.m6, HOST_B, true).await;
    assert!(stopped_once(&mut returned));
    // Its inventory still says running until its next heartbeat.
    assert_eq!(part(&p, p.m6), Some((MachinePart::StillRunning, None)));
    p.state
        .sessions
        .update_daemon_projects(&p.m6, vec![listed(&p, HOST_B, false)]);
    assert_eq!(part(&p, p.m6), Some((MachinePart::NotStarted, None)));
}

/// Scenario "A project closed while its daemon was being replaced": the
/// stop went to the daemon that was replacing itself and was lost with it;
/// the daemon that returns starts what it had been running, registers, and
/// is asked to stop the project.
#[tokio::test]
async fn a_project_closed_during_a_daemon_replacement_is_stopped_when_it_registers() {
    let p = project().await;
    let mut replaced = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);

    close(&p, &web, p.owner, p.m5).await;
    assert!(
        stopped_once(&mut replaced),
        "sent to the daemon being replaced"
    );

    // The new process restored the project from its resume manifest.
    let mut returned = registers(&p, p.m5, HOST_A, true).await;
    assert!(stopped_once(&mut returned));
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Closed));
}

/// Scenario "The opener lost access to the machine": a follow-start is
/// checked against the account that opened the project, at the moment the
/// daemon registers. The machine is left as it is and reported with the
/// reason; a direct request from someone who may use it still starts it.
#[tokio::test]
async fn a_machine_the_opener_may_no_longer_use_is_not_started() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    let member = cluster_member(&p, &[p.m5, p.m6]).await;
    let member_web = p.viewer(member, KNOWS);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;

    open(&p, &member_web, member, p.m5).await;
    assert!(started_once(&mut d5));
    assert_eq!(record(&p).await.set_by_account(), Some(member));

    // The membership no longer allows zoo-006.
    p.state
        .db
        .update_cluster_member_policy(
            &p.host.to_string(),
            &member.to_string(),
            Some(vec![p.m5.to_string()]),
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    assert!(nothing_sent(&mut d6));
    let reason = may_not_use(member, HOST_B);
    assert_eq!(
        outcome(&p, p.m6),
        Some(MachineOutcome::NotStarted(reason.clone()))
    );
    assert_eq!(
        part(&p, p.m6),
        Some((MachinePart::NotStarted, Some(reason)))
    );

    // Open again, by someone who may use it, is the retry.
    open(&p, &web, p.owner, p.m5).await;
    assert!(started_once(&mut d6));
    assert_eq!(
        record(&p).await.set_by_account(),
        Some(member),
        "the opener is recorded on the transition only"
    );
}

/// Scenario "A member's close does not reach a machine the member may not
/// use": the follow-stop is checked against the closer. The machine keeps
/// running and is reported as still running with the reason.
#[tokio::test]
async fn a_members_close_is_not_followed_on_a_machine_the_member_may_not_use() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    let member = cluster_member(&p, &[p.m5]).await;
    let member_web = p.viewer(member, KNOWS);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);
    reports_running(&p, p.m6, HOST_B);
    p.state.sessions.unregister_daemon(&p.m6);

    close(&p, &member_web, member, p.m5).await;
    assert!(stopped_once(&mut d5));
    let closed = record(&p).await;
    assert_eq!(closed.state, Some(ProjectOpenState::Closed));
    assert_eq!(closed.set_by_account(), Some(member));

    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(nothing_sent(&mut d6));
    let reason = may_not_use(member, HOST_B);
    assert_eq!(
        outcome(&p, p.m6),
        Some(MachineOutcome::NotStopped(reason.clone()))
    );
    assert_eq!(
        part(&p, p.m6),
        Some((MachinePart::StillRunning, Some(reason)))
    );

    // Close again, by someone who may use it, stops it.
    close(&p, &web, p.owner, p.m5).await;
    assert!(stopped_once(&mut d6));
}

/// Another account's checkout can carry the same project id. Its machine is
/// not one of the project's: a person's open or close is never followed
/// there, nothing of it is kept or told, and the project's viewers are not
/// shown it. An administrative stop is followed on every machine that lists
/// the project, as that stop always reached them.
#[tokio::test]
async fn another_accounts_checkout_with_the_same_id_does_not_follow() {
    let p = project().await;
    let stranger = Uuid::new_v4();
    p.state.db.create_user(&user(stranger)).await.unwrap();
    let theirs = Uuid::new_v4();
    let _d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _web = p.viewer(p.owner, KNOWS);
    // An untagged tab is attributed to zoo-006 by name, and the stranger's
    // machine carries that name.
    tabs(
        &p,
        vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, None, Some(HOST_B)),
        ],
    );

    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    let mut d = registers_as(&p, stranger, theirs, HOST_B, CAPABLE_DAEMON, false).await;
    assert!(nothing_sent(&mut d), "an open is not followed there");
    assert_eq!(outcome(&p, theirs), None);
    assert_eq!(part(&p, theirs), None, "its machine is not shown");
    let view = p.state.sessions.project_open_view(&p.project_id()).unwrap();
    let by_name: Vec<_> = view
        .machines
        .iter()
        .filter(|machine| machine.hostname == HOST_B)
        .collect();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].machine_id, None, "the tab's host stays a name");
    assert_eq!(by_name[0].part, MachinePart::WaitingOffline);

    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let mut d = registers_as(&p, stranger, theirs, HOST_B, CAPABLE_DAEMON, true).await;
    assert!(
        nothing_sent(&mut d),
        "a person's close is not followed there"
    );
    assert_eq!(outcome(&p, theirs), None);
    assert_eq!(part(&p, theirs), None);

    assert!(p
        .state
        .db
        .set_project_open_state(
            &p.project_id(),
            ProjectOpenState::Closed,
            &p.owner.to_string(),
            OpenStateSetKind::Administrative,
            None,
        )
        .await
        .unwrap());
    let mut d = registers_as(&p, stranger, theirs, HOST_B, CAPABLE_DAEMON, true).await;
    assert!(
        stopped_once(&mut d),
        "an administrative stop reaches every machine that lists the project"
    );
}

/// Scenario "A project being deleted", and its neighbours: beginning a
/// deletion and suspending record the project closed, so a tab owner that
/// registers is not started, also after a reactivation.
#[tokio::test]
async fn a_project_being_deleted_or_suspended_is_not_started_at_registration() {
    let p = project().await;
    let _web = p.viewer(p.owner, KNOWS);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    assert!(p
        .state
        .db
        .begin_project_deletion(&p.owner.to_string(), &p.project_id(), &p.project_id())
        .await
        .unwrap());
    let mut d5 = registers(&p, p.m5, HOST_A, false).await;
    assert!(nothing_sent(&mut d5));
    // One that returns still running it is stopped, whoever began the
    // deletion: it is an administrative close.
    let mut d5 = registers(&p, p.m5, HOST_A, true).await;
    assert!(stopped_once(&mut d5));

    let q = project().await;
    let _web = q.viewer(q.owner, KNOWS);
    tabs(&q, vec![pane(3, Some(q.m5), Some(HOST_A))]);
    set_recorded(&q, ProjectOpenState::Open, q.owner, Some(q.m5)).await;
    assert!(q
        .state
        .db
        .set_project_lifecycle(
            &q.owner.to_string(),
            &q.project_id(),
            crate::db::ProjectLifecycle::Suspended,
        )
        .await
        .unwrap());
    let mut q5 = registers(&q, q.m5, HOST_A, false).await;
    assert!(nothing_sent(&mut q5));
    assert!(q
        .state
        .db
        .set_project_lifecycle(
            &q.owner.to_string(),
            &q.project_id(),
            crate::db::ProjectLifecycle::Active,
        )
        .await
        .unwrap());
    let mut q5 = registers(&q, q.m5, HOST_A, false).await;
    assert!(nothing_sent(&mut q5), "reactivation starts nothing");
    assert_eq!(recorded(&q).await, Some(ProjectOpenState::Closed));
}

/// Scenario "An older daemon registers before the others have returned":
/// the server has restarted and nothing is attached, so only the roster
/// says another machine owns tabs. The older daemon is sent nothing and is
/// reported with the older-APAS sentence. As the only tab owner it starts
/// the project as its only runtime, with a start it does not answer.
#[tokio::test]
async fn an_older_daemon_beside_another_tab_owner_is_not_started_at_registration() {
    let p = project().await;
    let laptop = Uuid::new_v4();
    let _web = p.viewer(p.owner, KNOWS);
    tabs(
        &p,
        vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(laptop), Some(LAPTOP)),
        ],
    );
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;

    let mut older = registers_as(&p, p.host, laptop, LAPTOP, OLDER_DAEMON, false).await;
    assert!(nothing_sent(&mut older));
    assert_eq!(
        outcome(&p, laptop),
        Some(MachineOutcome::NotStarted(older_apas(LAPTOP)))
    );
    assert_eq!(
        part(&p, laptop),
        Some((MachinePart::NotStarted, Some(older_apas(LAPTOP))))
    );
    let mut d5 = registers(&p, p.m5, HOST_A, false).await;
    one_correlated_start(&mut d5);

    let q = project().await;
    let laptop = Uuid::new_v4();
    let _web = q.viewer(q.owner, KNOWS);
    tabs(&q, vec![pane(3, Some(laptop), Some(LAPTOP))]);
    set_recorded(&q, ProjectOpenState::Open, q.owner, None).await;
    let mut alone = registers_as(&q, q.host, laptop, LAPTOP, OLDER_DAEMON, false).await;
    assert_eq!(sent(&mut alone), (vec![None], 0));
}

/// A project with no tabs is followed by the machine its open named, which
/// stands in as the one tab owner; another machine that lists the project
/// is not started.
#[tokio::test]
async fn a_tabless_open_project_is_followed_by_the_machine_its_open_named() {
    let p = project().await;
    let _web = p.viewer(p.owner, KNOWS);
    tabs(&p, Vec::new());
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;

    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    assert!(nothing_sent(&mut d6));
    assert_eq!(part(&p, p.m6), None);
    let mut d5 = registers(&p, p.m5, HOST_A, false).await;
    one_correlated_start(&mut d5);
    let view = p.state.sessions.project_open_view(&p.project_id()).unwrap();
    assert_eq!(view.machines.len(), 1);
    assert!(view.machines[0].owns_tabs, "the named machine stands in");
}

/// With no roster held for the project its tab owners are unknown, which is
/// not an empty roster: no follow step starts anything, not even the machine
/// the open named.
#[tokio::test]
async fn an_unknown_roster_starts_nothing_at_registration() {
    let p = project().await;
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    assert_eq!(
        project_tab_owners(&p.state, &p.project_id(), Some(p.m5)).await,
        TabOwners::Unknown
    );
    let mut d5 = registers(&p, p.m5, HOST_A, false).await;
    assert!(nothing_sent(&mut d5));
    assert_eq!(outcome(&p, p.m5), None);
    assert_eq!(part(&p, p.m5), None);
}

/// A daemon that registers twice: the step of a connection that was replaced
/// sends nothing, each registration that finds the machine out of line asks
/// it once, one that finds it running asks nothing, and a project listed
/// twice is followed once.
#[tokio::test]
async fn a_daemon_that_registers_twice_is_asked_once_per_registration() {
    let p = project().await;
    let _web = p.viewer(p.owner, KNOWS);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;

    let (first, mut first_daemon) = connects(&p, p.host, p.m5, HOST_A, CAPABLE_DAEMON, false);
    let (second, mut second_daemon) = connects(&p, p.host, p.m5, HOST_A, CAPABLE_DAEMON, false);
    assert_eq!(
        follow_project(&p.state, &p.m5, &first, &p.project_id()).await,
        Followed::Nothing
    );
    follow_registered_daemon(&p.state, p.m5, &first, vec![p.project_id()]).await;
    assert!(nothing_sent(&mut first_daemon));
    assert!(
        nothing_sent(&mut second_daemon),
        "the replaced step sent nothing"
    );
    follow_registered_daemon(
        &p.state,
        p.m5,
        &second,
        vec![p.project_id(), p.project_id()],
    )
    .await;
    let asked = one_correlated_start(&mut second_daemon);

    // Registering again replaces the connection that start was sent on, so
    // the machine is asked again, once, under a new id.
    let mut third_daemon = registers(&p, p.m5, HOST_A, false).await;
    assert_ne!(one_correlated_start(&mut third_daemon), asked);
    assert!(nothing_sent(&mut second_daemon));

    let mut running = registers(&p, p.m5, HOST_A, true).await;
    assert!(
        nothing_sent(&mut running),
        "a machine in line is sent nothing"
    );
    assert_eq!(part(&p, p.m5), Some((MachinePart::Running, None)));
}

/// An Open racing a follow-stop: the step for a machine that registered
/// running a closed project waits for the project's open-state lock and
/// reads the state again once it has it. An Open that got there first found
/// the machine running and sent it only the start its daemon answers by
/// changing nothing, so a stop decided on the earlier state would end what
/// was just opened. None is sent.
#[tokio::test]
async fn a_follow_stop_is_never_sent_after_a_later_open() {
    let p = project().await;
    let _web = p.viewer(p.owner, KNOWS);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let (connection, mut d5) = connects(&p, p.host, p.m5, HOST_A, CAPABLE_DAEMON, true);

    // The Open holds the lock; the follow step starts and waits behind it.
    let opening = p
        .state
        .sessions
        .project_open_state_guard(&p.project_id())
        .await;
    let follow = {
        let state = p.state.clone();
        let project_id = p.project_id();
        let machine = p.m5;
        tokio::spawn(
            async move { follow_project(&state, &machine, &connection, &project_id).await },
        )
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!follow.is_finished(), "the step waits for the lock");
    let opened = crate::open_state::open_project_locked(
        &p.state,
        &p.owner,
        &p.project_id(),
        Some(p.m5),
        "open",
        None,
    )
    .await
    .unwrap();
    assert!(opened.recorded);
    assert_eq!(opened.asked, vec![p.m5], "the start that changes nothing");
    drop(opening);

    let followed = tokio::time::timeout(Duration::from_secs(5), follow)
        .await
        .expect("the step runs once the lock is free")
        .unwrap();
    assert_eq!(followed, Followed::Nothing);
    assert_eq!(sent(&mut d5), (vec![None], 0), "no stop after the open");
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Open));
    assert_eq!(part(&p, p.m5), Some((MachinePart::Running, None)));
}

/// Nothing is started or stopped for a project nobody has recorded, and a
/// machine that refused stays as it is: heartbeats ask it nothing, and it is
/// asked again only when someone opens the project again or its daemon
/// registers again.
#[tokio::test]
async fn nothing_follows_a_not_recorded_project_and_no_heartbeat_starts_anything() {
    // A project nobody has recorded that runs on zoo-005, as at a deployment
    // before that instance's `SessionStart` records it open: zoo-006's
    // registration starts nothing, and stops nothing on its return.
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _running = p.instance(p.m5, HOST_A);
    let _web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    reports_running(&p, p.m5, HOST_A);
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    assert!(nothing_sent(&mut d6) && nothing_sent(&mut d5));
    assert_eq!(recorded(&p).await, None);
    assert_eq!(part(&p, p.m5), Some((MachinePart::Running, None)));
    assert_eq!(part(&p, p.m6), Some((MachinePart::NotStarted, None)));
    // A machine that returns running it is not stopped either.
    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(nothing_sent(&mut d6));

    // Scenario "A failed start is not retried by itself".
    let q = project().await;
    let mut q5 = q.daemon(q.m5, HOST_A, CAPABLE_DAEMON);
    let web = q.viewer(q.owner, KNOWS);
    two_machine_roster(&q);
    set_recorded(&q, ProjectOpenState::Open, q.owner, None).await;
    reports_running(&q, q.m5, HOST_A);
    let mut q6 = registers(&q, q.m6, HOST_B, false).await;
    let request_id = one_correlated_start(&mut q6);
    let refused = "Another host holds this project's claim";
    assert!(q.state.sessions.machine_start_answered(
        &q.m6,
        &request_id,
        &q.project_id(),
        false,
        Some(refused.to_string()),
    ));
    assert_eq!(
        part(&q, q.m6),
        Some((MachinePart::NotStarted, Some(refused.to_string())))
    );
    for _ in 0..3 {
        q.state
            .sessions
            .update_daemon_projects(&q.m6, vec![listed(&q, HOST_B, false)]);
        crate::open_state::load_missing_bases(&q.state, &q.m6).await;
        q.state
            .sessions
            .update_daemon_projects(&q.m5, vec![listed(&q, HOST_A, true)]);
    }
    assert!(nothing_sent(&mut q6) && nothing_sent(&mut q5));
    assert_eq!(
        part(&q, q.m6),
        Some((MachinePart::NotStarted, Some(refused.to_string())))
    );

    open(&q, &web, q.owner, q.m5).await;
    assert!(
        started_once(&mut q6),
        "Open on an open project is the retry"
    );
    assert!(nothing_sent(&mut q5));
    let mut again = registers(&q, q.m6, HOST_B, false).await;
    one_correlated_start(&mut again);
}

/// A member closed the project and one machine, which the member may not
/// use, was away. The owner closes it again while that machine is still
/// away, so nothing can be sent. The owner may stop every machine the member
/// may and one more, so the owner becomes the recorded closer, and the
/// machine is stopped on the owner's authority when its daemon registers.
#[tokio::test]
async fn a_second_close_by_a_wider_authority_reaches_the_machine_that_was_away() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    let member = cluster_member(&p, &[p.m5]).await;
    let member_web = p.viewer(member, KNOWS);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);
    reports_running(&p, p.m6, HOST_B);
    p.state.sessions.unregister_daemon(&p.m6);

    close(&p, &member_web, member, p.m5).await;
    assert!(stopped_once(&mut d5));
    assert_eq!(record(&p).await.set_by_account(), Some(member));

    close(&p, &web, p.owner, p.m5).await;
    assert!(errors(&web.drain()).is_empty());
    let closed = record(&p).await;
    assert_eq!(closed.state, Some(ProjectOpenState::Closed));
    assert_eq!(closed.set_kind, Some(OpenStateSetKind::Request));
    assert_eq!(
        closed.set_by_account(),
        Some(p.owner),
        "the closer who may stop more machines is recorded"
    );

    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(stopped_once(&mut d6), "stopped on the owner's close");
    assert_eq!(outcome(&p, p.m6), None);
}

/// The mirror: the owner closed the project while a machine was away, and a
/// member who may not use that machine closes it again. The member may stop
/// fewer machines, so the owner stays the recorded closer and the machine is
/// still stopped when its daemon registers. An administrative close is not
/// replaced by anyone's repeated close.
#[tokio::test]
async fn a_second_close_by_a_narrower_authority_leaves_the_recorded_closer() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    let member = cluster_member(&p, &[p.m5]).await;
    let mut member_web = p.viewer(member, KNOWS);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);
    reports_running(&p, p.m6, HOST_B);
    p.state.sessions.unregister_daemon(&p.m6);

    close(&p, &web, p.owner, p.m5).await;
    assert!(stopped_once(&mut d5));
    let closed = record(&p).await;
    assert_eq!(closed.set_by_account(), Some(p.owner));

    close(&p, &member_web, member, p.m5).await;
    assert!(errors(&member_web.drain()).is_empty());
    assert!(stopped_once(&mut d5), "the member may stop zoo-005 again");
    assert_eq!(record(&p).await, closed, "the owner stays the closer");

    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(stopped_once(&mut d6), "still stopped on the owner's close");

    // An operator's stop stays the record whoever closes again.
    crate::open_state::record_administrative_close(
        &p.state,
        &member.to_string(),
        &p.project_id(),
        "stop_runtime",
    )
    .await
    .unwrap();
    let administrative = record(&p).await;
    close(&p, &web, p.owner, p.m5).await;
    assert_eq!(record(&p).await, administrative);
}

/// A follow-start is made on the opener's authority, and a suspended
/// account can make no request: a machine that registers is not started for
/// a project that account opened, and says why. A close that account made
/// before it was suspended still stops a machine that returns running it.
#[tokio::test]
async fn a_suspended_openers_project_is_not_started_and_its_close_still_stops() {
    let p = project().await;
    let _web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    let member = cluster_member(&p, &[p.m5, p.m6]).await;
    set_recorded(&p, ProjectOpenState::Open, member, Some(p.m5)).await;

    // While the account is active its open is followed.
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    one_correlated_start(&mut d6);

    assert!(p
        .state
        .db
        .update_cluster_user_status(
            crate::db::SYSTEM_ADMIN_ACTOR,
            &member.to_string(),
            crate::db::AccountStatus::Suspended,
        )
        .await
        .unwrap());
    let opened = record(&p).await;
    assert_eq!(
        recorded_actor_may_use_machine(&p.state, &opened, &p.m6).await,
        Err(may_not_use(member, HOST_B))
    );
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    assert!(nothing_sent(&mut d6), "a suspended account starts nothing");
    let reason = may_not_use(member, HOST_B);
    assert_eq!(
        outcome(&p, p.m6),
        Some(MachineOutcome::NotStarted(reason.clone()))
    );
    assert_eq!(
        part(&p, p.m6),
        Some((MachinePart::NotStarted, Some(reason)))
    );

    // Its close is still followed: stopping needs no active account.
    set_recorded(&p, ProjectOpenState::Closed, member, None).await;
    let closed = record(&p).await;
    assert_eq!(
        recorded_actor_may_use_machine(&p.state, &closed, &p.m6).await,
        Ok(())
    );
    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(stopped_once(&mut d6));

    // An opener whose account no longer exists starts nothing either.
    let gone = Uuid::new_v4();
    set_recorded(&p, ProjectOpenState::Open, gone, Some(p.m5)).await;
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    assert!(nothing_sent(&mut d6));
}

/// The open and close events of the project in the deployment's audit
/// history, newest first: (action, actor, kind, source, hosts).
async fn open_state_audit(p: &Project) -> Vec<(String, String, String, String, Vec<String>)> {
    p.state
        .db
        .list_audit_events(200, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| {
            event.project_id.as_deref() == Some(p.project_id().as_str())
                && (event.action == "project.opened" || event.action == "project.closed")
        })
        .map(|event| {
            let details: serde_json::Value =
                serde_json::from_str(event.details.as_deref().unwrap()).unwrap();
            (
                event.action,
                event.actor_user_id,
                details["kind"].as_str().unwrap().to_string(),
                details["source"].as_str().unwrap().to_string(),
                details["machines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|machine| machine["hostname"].as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

/// A start or a stop the follow step sends is written to the audit history
/// as the open or close it carries out: the same event, by whoever set the
/// state, with source "registration" and the one machine. So the history of
/// a close also shows the machine that was away and was stopped later. A
/// registration that sends nothing, or leaves the machine as it is, writes
/// nothing.
#[tokio::test]
async fn a_start_or_stop_sent_at_registration_is_audited_with_who_set_the_state() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let registration = |action: &str, actor: String, kind: &str, host: &str| {
        (
            action.to_string(),
            actor,
            kind.to_string(),
            "registration".to_string(),
            vec![host.to_string()],
        )
    };

    // Opened while zoo-006 was away: its start at registration is audited.
    open(&p, &web, p.owner, p.m5).await;
    assert!(started_once(&mut d5));
    let before = open_state_audit(&p).await;
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    one_correlated_start(&mut d6);
    let after = open_state_audit(&p).await;
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(
        after[0],
        registration("project.opened", p.owner.to_string(), "request", HOST_B)
    );
    // Every hosting cluster finds it in its own history too.
    assert!(p
        .state
        .db
        .list_cluster_audit_events(&p.owner.to_string(), 50, 0)
        .await
        .unwrap()
        .iter()
        .any(|event| event.action == "project.opened"
            && event
                .details
                .as_deref()
                .is_some_and(|details| details.contains("\"registration\""))));

    // A registration that finds the machine in line writes nothing.
    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(nothing_sent(&mut d6));
    assert_eq!(open_state_audit(&p).await, after);

    // Closed while zoo-006 was away: its stop at registration is audited.
    p.state.sessions.unregister_daemon(&p.m6);
    close(&p, &web, p.owner, p.m5).await;
    let before = open_state_audit(&p).await;
    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(stopped_once(&mut d6));
    let after = open_state_audit(&p).await;
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(
        after[0],
        registration("project.closed", p.owner.to_string(), "request", HOST_B)
    );

    // A machine left as it is writes nothing: the closer may not use it.
    let member = cluster_member(&p, &[p.m5]).await;
    set_recorded(&p, ProjectOpenState::Closed, member, None).await;
    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(nothing_sent(&mut d6));
    assert_eq!(open_state_audit(&p).await, after);

    // The system administrator's stop is followed in the administrator's
    // name, as an administrative close.
    crate::open_state::record_administrative_close(
        &p.state,
        crate::db::SYSTEM_ADMIN_ACTOR,
        &p.project_id(),
        "stop_runtime",
    )
    .await
    .unwrap();
    let before = open_state_audit(&p).await;
    let mut d6 = registers(&p, p.m6, HOST_B, true).await;
    assert!(stopped_once(&mut d6));
    let after = open_state_audit(&p).await;
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(
        after[0],
        registration(
            "project.closed",
            crate::db::SYSTEM_ADMIN_ACTOR.to_string(),
            "administrative",
            HOST_B
        )
    );
}
