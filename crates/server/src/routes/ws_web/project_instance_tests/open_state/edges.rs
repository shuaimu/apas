//! The edges of opening and closing: a requester that does not read its
//! socket, a tab that was waiting when the project was closed, a machine that
//! still runs a closed project (named by a tab request, or not), a machine
//! that is away, another account's machine of the same name, and a project
//! the server first hears of from an Open.

use super::follow::{part, registers_as};
use super::*;
use crate::session::open_state::closed_before_start_reason;
use shared::MachinePart;

/// A connection of `user` whose outbound queue is full and is never read, as
/// a client that stopped reading its socket: every frame sent to it waits
/// until the send gives up. It is attached to the project's session.
fn stalled_viewer(p: &Project, user: Uuid, capabilities: &[&str]) -> Endpoint<ServerToWeb> {
    let id = Uuid::new_v4();
    let (tx, rx) = mpsc::channel(1);
    p.state.sessions.register_web(id, tx.clone());
    p.state.sessions.set_web_user(id, user);
    p.state.sessions.set_web_capabilities(
        id,
        capabilities
            .iter()
            .map(|capability| capability.to_string())
            .collect(),
    );
    assert!(p.state.sessions.attach_web_to_session(&p.session, id, None));
    // Whatever attaching queued is discarded, and one unread frame fills it.
    let mut endpoint = Endpoint { id, rx };
    endpoint.drain();
    tx.try_send(ServerToWeb::Error {
        message: "unread".to_string(),
    })
    .expect("the queue has room for one frame");
    endpoint
}

/// A refusal is told to the requester after the project's open-state lock is
/// released. A requester that does not read its socket makes that answer
/// wait for seconds; meanwhile the project's lock is free for the next open,
/// close or follow step.
#[tokio::test]
async fn a_requester_that_does_not_read_its_socket_does_not_hold_the_projects_lock() {
    let p = project().await;
    let stranger = Uuid::new_v4();
    p.state.db.create_user(&user(stranger)).await.unwrap();
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let _web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let closed = record(&p).await;
    let project_id = p.project_id();

    #[derive(Debug, Clone, Copy)]
    enum Request {
        Start(bool),
        Stop(bool),
        Tab,
    }
    for request in [
        Request::Start(true),
        Request::Start(false),
        Request::Stop(true),
        Request::Stop(false),
        Request::Tab,
    ] {
        // A stranger's Start and Stop are refused whole. The owner's tab,
        // from a client that does not know the open state, is refused
        // because the closed project has tabs on another machine.
        let requester = match request {
            Request::Tab => stalled_viewer(&p, p.owner, OLDER_VIEWER),
            _ => stalled_viewer(&p, stranger, OLDER_VIEWER),
        };
        let state = p.state.clone();
        let (connection, session, machine) = (requester.id, p.session, p.m6);
        let asked = {
            let project_id = project_id.clone();
            tokio::spawn(async move {
                match request {
                    Request::Start(all) => {
                        start_machine_project(
                            &state,
                            &connection,
                            &stranger,
                            machine,
                            &project_id,
                            all,
                        )
                        .await
                    }
                    Request::Stop(all) => {
                        stop_machine_project(
                            &state,
                            &connection,
                            &stranger,
                            machine,
                            &project_id,
                            all,
                        )
                        .await
                    }
                    Request::Tab => {
                        handle_add_pane(
                            &state,
                            &connection,
                            Some(session),
                            None,
                            Some(machine),
                            Some("stalled".to_string()),
                            new_pane(),
                        )
                        .await
                    }
                }
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !asked.is_finished(),
            "{request:?}: its refusal is still waiting for the socket"
        );
        // The refusal waits five seconds for the socket; the lock is free
        // well before that.
        tokio::time::timeout(
            Duration::from_secs(4),
            p.state.sessions.project_open_state_guard(&project_id),
        )
        .await
        .unwrap_or_else(|_| panic!("{request:?}: the project's lock is free meanwhile"));
        asked.abort();
        p.state.sessions.unregister_web(&requester.id);
    }
    assert_eq!(record(&p).await, closed);
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));
}

/// A tab is waiting for a machine whose daemon accepted the start and whose
/// instance has not attached, and the project is closed. The close stops that machine and ends the tab's wait with the
/// reason, so the next tab request on that machine asks it to start again
/// instead of joining a wait for a start that was stopped.
#[tokio::test]
async fn a_close_ends_the_tab_start_it_stopped_so_the_next_tab_starts_the_machine_again() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let _a = p.running(p.m5, HOST_A);
    let mut web = p.viewer(p.owner, KNOWS);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    let pending = |p: &Project| {
        p.state
            .sessions
            .machines_with_pending_instance_start(&[p.session])
    };

    p.add_pane(&web, Some(p.m6), Some("first")).await;
    assert_eq!(
        only(add_results(&web.drain())).status,
        shared::AddPaneStatus::Starting
    );
    let (starts, _) = sent(&mut d6);
    let request_id = starts[0].expect("a correlated start");
    // Its daemon answers that the runtime started; the instance has not
    // attached, so the tab keeps waiting.
    assert!(p
        .state
        .sessions
        .instance_start_result(p.m6, request_id, &p.project_id(), true, None));
    assert_eq!(pending(&p), vec![p.m6]);

    close(&p, &web, p.owner, p.m5).await;

    assert!(stopped_once(&mut d5) && stopped_once(&mut d6));
    let failed = next_add_result(&mut web).await;
    assert_eq!(
        (failed.status, failed.machine_id, failed.error),
        (
            shared::AddPaneStatus::Failed,
            Some(p.m6),
            Some("The project was closed before it started on zoo-006.".to_string())
        )
    );
    assert!(pending(&p).is_empty(), "the wait is over");

    // The next tab on zoo-006 opens the project and asks zoo-006 to start.
    p.add_pane(&web, Some(p.m6), Some("second")).await;
    assert_eq!(
        only(add_results(&web.drain())).status,
        shared::AddPaneStatus::Starting
    );
    assert!(
        matches!(sent(&mut d6).0.as_slice(), [Some(_)]),
        "zoo-006 is asked to start again"
    );
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Open));

    // An administrative stop ends such a wait the same way.
    web.drain();
    crate::open_state::record_administrative_close(
        &p.state,
        &p.owner.to_string(),
        &p.project_id(),
        "stop_runtime",
    )
    .await
    .unwrap();
    p.state.sessions.stop_project_runtime(&p.project_id()).await;
    let failed = next_add_result(&mut web).await;
    assert_eq!(
        (failed.status, failed.error),
        (
            shared::AddPaneStatus::Failed,
            Some(closed_before_start_reason(HOST_B))
        )
    );
    assert!(pending(&p).is_empty());
}

/// A stop naming one machine of an open project closes the project when no
/// other machine owns tabs or runs it. That takes the authority to set the
/// project's state, as the project-wide Close does: while the machine is
/// away, a cluster member who neither owns nor hosts the project may not
/// record a close through the machine's own Stop.
#[tokio::test]
async fn a_stop_naming_a_machine_that_is_away_closes_nothing_for_an_account_that_may_not_set_the_state(
) {
    let p = project().await;
    let member = cluster_member(&p, &[p.m5]).await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut web = p.viewer(member, OLDER_VIEWER);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    let open_record = record(&p).await;

    p.state.sessions.unregister_daemon(&p.m5);
    assert!(
        has_machine_project_runtime_access(&p.state, &member, &p.m5, &p.project_id()).await,
        "the machine's own check still passes while it is away"
    );
    stop_one(&p, &web, member, p.m5).await;
    close(&p, &web, member, p.m5).await;
    assert_eq!(
        errors(&web.drain()),
        vec!["Machine not found".to_string(); 2],
        "the stop is refused as the project-wide Close is"
    );
    assert_eq!(record(&p).await, open_record, "nothing is recorded");
    assert!(nothing_sent(&mut d5));

    // The owner's stop of the machine that is away is still the close its
    // registration then follows.
    let mut owner_web = p.viewer(p.owner, OLDER_VIEWER);
    stop_one(&p, &owner_web, p.owner, p.m5).await;
    assert!(errors(&owner_web.drain()).is_empty());
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Closed));
    assert_eq!(record(&p).await.set_by_account(), Some(p.owner));

    // While the machine is connected the member may stop it, and that is
    // the close it always was.
    let q = project().await;
    let member = cluster_member(&q, &[q.m5]).await;
    let mut q5 = q.daemon(q.m5, HOST_A, CAPABLE_DAEMON);
    let mut web = q.viewer(member, OLDER_VIEWER);
    tabs(&q, vec![pane(3, Some(q.m5), Some(HOST_A))]);
    set_recorded(&q, ProjectOpenState::Open, q.owner, Some(q.m5)).await;
    reports_running(&q, q.m5, HOST_A);
    stop_one(&q, &web, member, q.m5).await;
    assert!(errors(&web.drain()).is_empty());
    assert!(stopped_once(&mut q5));
    assert_eq!(record(&q).await.set_by_account(), Some(member));
}

/// An untagged tab is attributed to a host by name, and another account's
/// checkout with the same project id can sit on a machine of that name. That
/// machine owns none of the project's tabs: it does not make the project's
/// own machine one of two owners in any decision. A name only such a machine
/// carries stays a name.
#[tokio::test]
async fn another_accounts_machine_of_the_same_name_owns_none_of_the_projects_tabs() {
    let p = project().await;
    let stranger = Uuid::new_v4();
    p.state.db.create_user(&user(stranger)).await.unwrap();
    let theirs = Uuid::new_v4();
    let mut d5 = p.daemon(p.m5, HOST_A, OLDER_DAEMON);
    let mut df = foreign_daemon(
        &p,
        stranger,
        theirs,
        HOST_A,
        vec![listed(&p, HOST_A, false)],
    );
    let mut web = p.viewer(p.owner, OLDER_VIEWER);
    tabs(
        &p,
        vec![pane(3, None, Some(HOST_A)), pane(4, None, Some(HOST_A))],
    );
    let ours = TabOwner {
        machine_id: Some(p.m5),
        hostname: HOST_A.to_string(),
        stand_in: false,
    };

    let owners = project_tab_owners(&p.state, &p.project_id(), None).await;
    assert_eq!(owners, TabOwners::Known(vec![ours.clone()]));
    assert_eq!(beside_other_owners_refusal(&p.state, &owners, &p.m5), None);

    // A start naming zoo-005 of the closed project opens it: no other
    // machine owns tabs. Its older daemon is the only owner, and starts.
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    start_one(&p, &web, p.owner, p.m5).await;
    assert!(errors(&web.drain()).is_empty());
    assert_eq!(sent(&mut d5), (vec![None], 0));
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Open));
    assert_eq!(
        outcome(&p, theirs),
        None,
        "nothing is held for their machine"
    );

    // The older daemon is followed at its registration, as the only owner.
    let mut d5 = registers_as(&p, p.host, p.m5, HOST_A, OLDER_DAEMON, false).await;
    assert_eq!(sent(&mut d5), (vec![None], 0));

    // A stop naming zoo-005 closes the project: nothing else owns tabs.
    reports_running(&p, p.m5, HOST_A);
    stop_one(&p, &web, p.owner, p.m5).await;
    assert!(errors(&web.drain()).is_empty());
    assert!(stopped_once(&mut d5));
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Closed));
    assert!(df.drain().is_empty(), "their machine is sent nothing");

    // A host name only their machine carries stays a name, and a name is
    // another owner.
    let other = Uuid::new_v4();
    let _dg = foreign_daemon(
        &p,
        stranger,
        other,
        "zoo-009",
        vec![listed(&p, "zoo-009", false)],
    );
    tabs(
        &p,
        vec![pane(3, None, Some(HOST_A)), pane(5, None, Some("zoo-009"))],
    );
    assert_eq!(
        project_tab_owners(&p.state, &p.project_id(), None).await,
        TabOwners::Known(vec![
            ours,
            TabOwner {
                machine_id: None,
                hostname: "zoo-009".to_string(),
                stand_in: false,
            }
        ])
    );
}

/// The closed row of design Decision 3 for a tab on a machine that still has
/// an instance attached, as after a close that left the machine running or
/// could not reach its daemon: from a viewer that does not know the open
/// state the tab is refused while another machine owns tabs, and opens the
/// project when none does; from a viewer that knows, it opens the project as
/// the requester. A project that is open or not recorded is left as it is.
#[tokio::test]
async fn a_tab_where_a_closed_project_still_runs_follows_the_closed_row() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut b = p.running(p.m6, HOST_B);
    two_machine_roster(&p);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let closed = record(&p).await;

    // A viewer that does not know the open state: refused, nothing changes.
    let mut older = p.viewer(p.owner, OLDER_VIEWER);
    p.add_pane(&older, Some(p.m6), Some("older")).await;
    let failed = only(add_results(&older.drain()));
    assert_eq!(
        (failed.status, failed.machine_id, failed.error.as_deref()),
        (
            shared::AddPaneStatus::Failed,
            Some(p.m6),
            Some(CLOSED_WITH_TABS_ELSEWHERE)
        )
    );
    assert_eq!(record(&p).await, closed);
    assert!(added_panes(&b.drain()).is_empty(), "no tab is created");
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));

    // A project user may use an instance its owner runs, and may not open
    // the project: no machine is theirs to start. Their tab does not open
    // it, and is refused as it is once that instance has left.
    let reader = reader(&p).await;
    let mut reader_web = p.viewer(reader, KNOWS);
    p.add_pane(&reader_web, Some(p.m6), Some("reader")).await;
    let refused = only(add_results(&reader_web.drain()));
    assert_eq!(
        (refused.status, refused.error),
        (
            shared::AddPaneStatus::Failed,
            Some(format!(
                "Access denied: you cannot use {HOST_B} for this project"
            ))
        )
    );
    assert_eq!(record(&p).await, closed);
    assert!(added_panes(&b.drain()).is_empty());
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));

    // A tab that instance would refuse anyway opens nothing: it fails with
    // its own reason and the project stays closed.
    let mut knows = p.viewer(p.owner, KNOWS);
    handle_add_pane(
        &p.state,
        &knows.id,
        Some(p.session),
        None,
        Some(p.m6),
        Some("retired".to_string()),
        NewPaneSpec {
            kind: shared::PaneKind::Agent,
            ..new_pane()
        },
    )
    .await;
    let refused = only(add_results(&knows.drain()));
    assert_eq!(refused.status, shared::AddPaneStatus::Failed);
    assert!(
        refused
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("Conversation-only panes are retired")),
        "{refused:?}"
    );
    assert_eq!(record(&p).await, closed);
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));

    // A viewer that knows: the project is opened as the requester, the
    // other tab owner is started, and the tab goes to zoo-006's instance.
    p.add_pane(&knows, Some(p.m6), Some("knows")).await;
    let routed = only(add_results(&knows.drain()));
    assert_eq!(
        (routed.status, routed.machine_id),
        (shared::AddPaneStatus::Routed, Some(p.m6))
    );
    let opened = record(&p).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(p.owner));
    assert_eq!(opened.machine_id, Some(p.m6));
    assert!(matches!(sent(&mut d5).0.as_slice(), [Some(_)]));
    assert_eq!(
        sent(&mut d6),
        (vec![None], 0),
        "the machine that still runs it is sent the start that changes nothing"
    );
    assert_eq!(next_added_pane(&mut b).await.machine_id, Some(p.m6));
    assert_eq!(part(&p, p.m6), Some((MachinePart::Running, None)));

    // No other machine owns tabs: the older viewer's tab opens the project.
    let q = project().await;
    let mut q5 = q.daemon(q.m5, HOST_A, CAPABLE_DAEMON);
    let mut a = q.running(q.m5, HOST_A);
    tabs(&q, vec![pane(3, Some(q.m5), Some(HOST_A))]);
    set_recorded(&q, ProjectOpenState::Closed, q.owner, None).await;
    let mut older = q.viewer(q.owner, OLDER_VIEWER);
    q.add_pane(&older, Some(q.m5), Some("single")).await;
    assert_eq!(
        only(add_results(&older.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&q).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(q.owner));
    assert_eq!(opened.machine_id, Some(q.m5));
    assert_eq!(sent(&mut q5), (vec![None], 0));
    assert_eq!(next_added_pane(&mut a).await.machine_id, Some(q.m5));

    // Open: nothing changes, from either kind of viewer.
    let before = record(&q).await;
    let mut knows = q.viewer(q.owner, KNOWS);
    for (web, request) in [(&mut older, "open-older"), (&mut knows, "open-knows")] {
        q.add_pane(web, Some(q.m5), Some(request)).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Routed
        );
    }
    assert_eq!(record(&q).await, before);
    assert!(nothing_sent(&mut q5));

    // Not recorded: nothing is recorded and nothing is sent.
    let r = project().await;
    let mut r5 = r.daemon(r.m5, HOST_A, CAPABLE_DAEMON);
    let _a = r.running(r.m5, HOST_A);
    tabs(&r, vec![pane(3, Some(r.m5), Some(HOST_A))]);
    let mut knows = r.viewer(r.owner, KNOWS);
    r.add_pane(&knows, Some(r.m5), Some("unrecorded")).await;
    assert_eq!(
        only(add_results(&knows.drain())).status,
        shared::AddPaneStatus::Routed
    );
    assert_eq!(recorded(&r).await, None);
    assert!(nothing_sent(&mut r5));
}

/// Opening a closed project through a tab on a machine that still has an
/// instance attached takes the authority to set the project's state, as the
/// project-wide Open does. That machine's own check still passes while its
/// daemon's channel is away, so a cluster member who may use only that
/// machine, and whose Open is refused then, is refused the tab with the same
/// answer and records nothing. The owner's tab there still opens the
/// project, and so does the member's while the machine is connected.
#[tokio::test]
async fn a_tab_where_a_closed_project_still_runs_opens_nothing_for_an_account_that_may_not_set_the_state(
) {
    let p = project().await;
    let member = cluster_member(&p, &[p.m6]).await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut b = p.running(p.m6, HOST_B);
    tabs(&p, vec![pane(4, Some(p.m6), Some(HOST_B))]);
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let closed = record(&p).await;

    p.state.sessions.unregister_daemon(&p.m6);
    assert!(
        has_machine_project_runtime_access(&p.state, &member, &p.m6, &p.project_id()).await,
        "the machine's own check still passes while it is away"
    );
    let mut web = p.viewer(member, KNOWS);
    open(&p, &web, member, p.m6).await;
    assert_eq!(
        errors(&web.drain()),
        vec!["Machine not found".to_string()],
        "the member's Open is refused"
    );

    // Naming the machine, and naming none: zoo-006's instance is the primary.
    for (machine, request) in [(Some(p.m6), "named"), (None, "primary")] {
        p.add_pane(&web, machine, Some(request)).await;
        let refused = only(add_results(&web.drain()));
        assert_eq!(
            (refused.status, refused.machine_id, refused.error.as_deref()),
            (
                shared::AddPaneStatus::Failed,
                Some(p.m6),
                Some("Machine not found")
            ),
            "the tab is refused as the Open is"
        );
    }
    let mut older = p.viewer(member, OLDER_VIEWER);
    p.add_pane(&older, Some(p.m6), Some("older")).await;
    assert_eq!(
        only(add_results(&older.drain())).error.as_deref(),
        Some("Machine not found")
    );
    assert_eq!(record(&p).await, closed, "nothing is recorded");
    assert!(added_panes(&b.drain()).is_empty(), "no tab is created");
    assert!(nothing_sent(&mut d5));

    // The owner may set the state while zoo-005 is connected: their tab on
    // zoo-006's instance opens the project.
    let mut owner_web = p.viewer(p.owner, KNOWS);
    p.add_pane(&owner_web, Some(p.m6), Some("owner")).await;
    assert_eq!(
        only(add_results(&owner_web.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&p).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(p.owner));
    assert_eq!(next_added_pane(&mut b).await.machine_id, Some(p.m6));

    // While the machine is connected the member may open the project, and
    // their tab there does.
    let q = project().await;
    let member = cluster_member(&q, &[q.m6]).await;
    let mut q6 = q.daemon(q.m6, HOST_B, CAPABLE_DAEMON);
    let mut b = q.running(q.m6, HOST_B);
    tabs(&q, vec![pane(4, Some(q.m6), Some(HOST_B))]);
    set_recorded(&q, ProjectOpenState::Closed, q.owner, None).await;
    let mut web = q.viewer(member, KNOWS);
    q.add_pane(&web, Some(q.m6), Some("member")).await;
    assert_eq!(
        only(add_results(&web.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&q).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(member));
    assert_eq!(sent(&mut q6), (vec![None], 0));
    assert_eq!(next_added_pane(&mut b).await.machine_id, Some(q.m6));
}

/// An instance of a CLI that reported no machine, as an older one does.
fn machineless(p: &Project, host: &str) -> Endpoint<ServerToCli> {
    let cli = p.cli(&[shared::PROJECT_POLICY_CAPABILITY]);
    assert!(matches!(
        p.state.sessions.attach_cli_instance(
            p.session,
            cli.id,
            None,
            Some(format!("/work/{host}")),
            Some(host.to_string()),
        ),
        AttachOutcome::Attached { .. }
    ));
    cli
}

/// A tab request that names no machine goes to the primary instance, and
/// the closed row of design Decision 3 applies to it as to a request naming
/// that instance's machine. The web names no machine for an instance of an
/// older CLI that reported none; its machine is the one of the project that
/// carries its host. From a viewer that does not know the open state the tab
/// is refused while another machine owns tabs, and opens the project when
/// none does; from a viewer that knows, it opens the project as the
/// requester. A project that is open or not recorded is left as it is.
#[tokio::test]
async fn a_tab_that_names_no_machine_follows_the_closed_row_where_the_project_still_runs() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, OLDER_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut a = machineless(&p, HOST_A);
    reports_running(&p, p.m5, HOST_A);
    tabs(
        &p,
        vec![
            pane(3, None, Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ],
    );
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let closed = record(&p).await;

    // A viewer that does not know the open state: refused, with and without
    // a request id, and nothing changes.
    let mut older = p.viewer(p.owner, OLDER_VIEWER);
    p.add_pane(&older, None, Some("older")).await;
    let failed = only(add_results(&older.drain()));
    assert_eq!(
        (failed.status, failed.error.as_deref()),
        (
            shared::AddPaneStatus::Failed,
            Some(CLOSED_WITH_TABS_ELSEWHERE)
        )
    );
    p.add_pane(&older, None, None).await;
    assert_eq!(
        errors(&older.drain()),
        vec![CLOSED_WITH_TABS_ELSEWHERE.to_string()]
    );
    assert_eq!(record(&p).await, closed);
    assert!(added_panes(&a.drain()).is_empty(), "no tab is created");
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));

    // A viewer that knows: the project is opened as the requester, naming
    // the machine that carries the instance's host, and the tab is created.
    let mut knows = p.viewer(p.owner, KNOWS);
    p.add_pane(&knows, None, Some("knows")).await;
    assert_eq!(
        only(add_results(&knows.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&p).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(p.owner));
    assert_eq!(opened.machine_id, Some(p.m5));
    // Each machine goes through the check an open makes: zoo-005's daemon
    // cannot run the project beside another tab owner, and no machine
    // starts beside a CLI that cannot share the project.
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));
    assert_eq!(
        outcome(&p, p.m5),
        Some(MachineOutcome::NotStarted(older_apas(HOST_A)))
    );
    next_added_pane(&mut a).await;

    // No other machine owns tabs: the older viewer's tab opens the project.
    let q = project().await;
    let mut q5 = q.daemon(q.m5, HOST_A, OLDER_DAEMON);
    let mut a = machineless(&q, HOST_A);
    reports_running(&q, q.m5, HOST_A);
    tabs(&q, vec![pane(3, None, Some(HOST_A))]);
    set_recorded(&q, ProjectOpenState::Closed, q.owner, None).await;
    let mut older = q.viewer(q.owner, OLDER_VIEWER);
    q.add_pane(&older, None, Some("single")).await;
    assert_eq!(
        only(add_results(&older.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&q).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(q.owner));
    assert_eq!(opened.machine_id, Some(q.m5));
    assert_eq!(
        sent(&mut q5),
        (vec![None], 0),
        "the machine that still runs it is sent the start that changes nothing"
    );
    next_added_pane(&mut a).await;

    // Open: nothing changes, from either kind of viewer.
    let before = record(&q).await;
    let mut knows = q.viewer(q.owner, KNOWS);
    for (web, request) in [(&mut older, "open-older"), (&mut knows, "open-knows")] {
        q.add_pane(web, None, Some(request)).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Routed
        );
    }
    assert_eq!(record(&q).await, before);
    assert!(nothing_sent(&mut q5));

    // Not recorded: nothing is recorded and nothing is sent.
    let r = project().await;
    let mut r5 = r.daemon(r.m5, HOST_A, OLDER_DAEMON);
    let _a = machineless(&r, HOST_A);
    tabs(&r, vec![pane(3, None, Some(HOST_A))]);
    let mut knows = r.viewer(r.owner, KNOWS);
    r.add_pane(&knows, None, Some("unrecorded")).await;
    assert_eq!(
        only(add_results(&knows.drain())).status,
        shared::AddPaneStatus::Routed
    );
    assert_eq!(recorded(&r).await, None);
    assert!(nothing_sent(&mut r5));

    // An instance that reported its machine is the primary of a request
    // from a client that names none: the same row, for that machine.
    let t = project().await;
    let mut t5 = t.daemon(t.m5, HOST_A, CAPABLE_DAEMON);
    let mut t6 = t.daemon(t.m6, HOST_B, CAPABLE_DAEMON);
    let mut a = t.running(t.m5, HOST_A);
    two_machine_roster(&t);
    set_recorded(&t, ProjectOpenState::Closed, t.owner, None).await;
    let closed = record(&t).await;
    let mut older = t.viewer(t.owner, OLDER_VIEWER);
    t.add_pane(&older, None, Some("capable")).await;
    let failed = only(add_results(&older.drain()));
    assert_eq!(
        (failed.status, failed.machine_id, failed.error.as_deref()),
        (
            shared::AddPaneStatus::Failed,
            Some(t.m5),
            Some(CLOSED_WITH_TABS_ELSEWHERE)
        )
    );
    assert_eq!(record(&t).await, closed);
    assert!(added_panes(&a.drain()).is_empty());
    assert!(nothing_sent(&mut t5) && nothing_sent(&mut t6));
}

/// The same for an instance no machine can be named for: its CLI reported
/// none, and no machine of the project carries its host. Its own tabs are
/// the untagged ones attributed to that host; a tab of any other owner is on
/// another machine. The open it makes names no machine.
#[tokio::test]
async fn a_tab_on_an_instance_no_machine_can_be_named_for_follows_the_closed_row() {
    let p = project().await;
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut a = machineless(&p, LAPTOP);
    tabs(
        &p,
        vec![
            pane(3, None, Some(LAPTOP)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ],
    );
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;
    let closed = record(&p).await;

    let mut older = p.viewer(p.owner, OLDER_VIEWER);
    p.add_pane(&older, None, Some("elsewhere")).await;
    assert_eq!(
        only(add_results(&older.drain())).error.as_deref(),
        Some(CLOSED_WITH_TABS_ELSEWHERE)
    );
    assert_eq!(record(&p).await, closed);
    assert!(added_panes(&a.drain()).is_empty(), "no tab is created");
    assert!(nothing_sent(&mut d6));

    // Every tab is its own: the tab opens the project, naming no machine.
    tabs(&p, vec![pane(3, None, Some(LAPTOP))]);
    p.add_pane(&older, None, Some("own")).await;
    assert_eq!(
        only(add_results(&older.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&p).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(p.owner));
    assert_eq!(opened.machine_id, None);
    assert!(nothing_sent(&mut d6));
    next_added_pane(&mut a).await;

    // An account that may not set the project's state opens nothing.
    let q = project().await;
    let _q6 = q.daemon(q.m6, HOST_B, CAPABLE_DAEMON);
    let mut a = machineless(&q, LAPTOP);
    tabs(&q, vec![pane(3, None, Some(LAPTOP))]);
    set_recorded(&q, ProjectOpenState::Closed, q.owner, None).await;
    let closed = record(&q).await;
    let reader = reader(&q).await;
    let mut web = q.viewer(reader, KNOWS);
    q.add_pane(&web, None, Some("reader")).await;
    assert_eq!(
        only(add_results(&web.drain())).error.as_deref(),
        Some("Machine not found")
    );
    assert_eq!(record(&q).await, closed);
    assert!(added_panes(&a.drain()).is_empty());
}

/// A tab on a machine that still has an instance attached opens the closed
/// project, and that machine is sent the start whether or not it owns a tab:
/// the close sent it a stop that may still be taking effect, and the tab
/// would otherwise go to an instance that is ending. Its daemon takes the
/// frames in order, and starts the project again after the stop.
#[tokio::test]
async fn a_tab_that_opens_a_closed_project_starts_its_machine_again_though_it_owns_no_tab() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut b = p.running(p.m6, HOST_B);
    let mut web = p.viewer(p.owner, KNOWS);
    // Every tab is zoo-005's; zoo-006 runs the project with none.
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;

    // The close stops zoo-006, whose instance has not left yet.
    close(&p, &web, p.owner, p.m5).await;
    assert!(errors(&web.drain()).is_empty());
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Closed));
    assert!(stopped_once(&mut d6));
    d5.drain();

    p.add_pane(&web, Some(p.m6), Some("again")).await;
    assert_eq!(
        only(add_results(&web.drain())).status,
        shared::AddPaneStatus::Routed
    );
    let opened = record(&p).await;
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.machine_id, Some(p.m6));
    assert_eq!(
        sent(&mut d6),
        (vec![None], 0),
        "the chosen machine is started again after its stop"
    );
    assert!(
        matches!(sent(&mut d5).0.as_slice(), [Some(_)]),
        "the tab owner is started as an open starts it"
    );
    assert_eq!(next_added_pane(&mut b).await.machine_id, Some(p.m6));
    assert_eq!(part(&p, p.m6), Some((MachinePart::Running, None)));

    // A daemon that cannot run the project beside the tab owner goes
    // through the check every running machine goes through: it is sent
    // nothing, and the tab still goes to its instance.
    let q = project().await;
    let mut q5 = q.daemon(q.m5, HOST_A, CAPABLE_DAEMON);
    let mut q6 = q.daemon(q.m6, HOST_B, OLDER_DAEMON);
    let mut b = q.running(q.m6, HOST_B);
    let mut web = q.viewer(q.owner, KNOWS);
    tabs(&q, vec![pane(3, Some(q.m5), Some(HOST_A))]);
    set_recorded(&q, ProjectOpenState::Closed, q.owner, None).await;
    q.add_pane(&web, Some(q.m6), Some("older-daemon")).await;
    assert_eq!(
        only(add_results(&web.drain())).status,
        shared::AddPaneStatus::Routed
    );
    assert_eq!(recorded(&q).await, Some(ProjectOpenState::Open));
    assert!(nothing_sent(&mut q6));
    assert_eq!(
        outcome(&q, q.m6),
        Some(MachineOutcome::NotStarted(older_apas(HOST_B)))
    );
    assert_eq!(
        part(&q, q.m6).map(|(part, _)| part),
        Some(MachinePart::Running),
        "it still runs the project"
    );
    assert!(matches!(sent(&mut q5).0.as_slice(), [Some(_)]));
    next_added_pane(&mut b).await;
}

/// A start nothing answered keeps another machine from closing the project
/// by its own stop only while that start is within its time limit. Past it
/// the machine reads as not started, and the stop is the project's close.
/// The close still stops the machine that was asked to start.
#[tokio::test]
async fn a_start_nothing_answered_stops_counting_as_another_machine_after_its_time_limit() {
    let p = project().await;
    // Long enough for the first stop below to be decided within it.
    p.state.sessions.set_pane_launch_timing(
        crate::session::pane_launches::PANE_ID_RESERVATION_TTL,
        Duration::from_secs(2),
    );
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, OLDER_DAEMON);
    let _a = p.instance(p.m5, HOST_A);
    let mut web = p.viewer(p.owner, OLDER_VIEWER);
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    // zoo-006 was sent a start it answers none of, and never ran the project.
    p.state
        .sessions
        .record_machine_start_asked(&p.project_id(), p.m6, None);

    stop_one(&p, &web, p.owner, p.m5).await;
    assert_eq!(
        errors(&web.drain()),
        vec![RUNS_ON_OTHER_MACHINES.to_string()],
        "zoo-006 is still being started"
    );
    assert!(nothing_sent(&mut d5) && nothing_sent(&mut d6));

    tokio::time::sleep(Duration::from_millis(2200)).await;
    stop_one(&p, &web, p.owner, p.m5).await;
    assert!(errors(&web.drain()).is_empty());
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Closed));
    assert!(stopped_once(&mut d5));
    assert!(
        stopped_once(&mut d6),
        "a close stops a machine asked to start, however long ago"
    );
}

/// The path every new project takes: a machine lists a project the server
/// holds no record of, and a viewer that knows the open state opens it. The
/// project gets its record and its placement, is recorded open by that
/// account naming that machine, and the machine is sent exactly one start.
/// The `SessionStart` that follows finds the record and leaves it as it is.
#[tokio::test]
async fn opening_a_project_only_a_registration_knows_records_and_starts_it() {
    use jsonwebtoken::{encode, EncodingKey, Header};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let p = project().await;
    let registered = Uuid::new_v4();
    let project_id = registered.to_string();
    let m7 = Uuid::new_v4();
    let mut daemon = p.daemon_with(
        p.owner,
        m7,
        HOST_C,
        CAPABLE_DAEMON,
        vec![shared::MachineProjectInfo {
            project_id: project_id.clone(),
            ..listed(&p, HOST_C, false)
        }],
    );
    assert!(p.state.db.get_project(&project_id).await.unwrap().is_none());
    let mut web = p.viewer(p.owner, KNOWS);

    start_machine_project(&p.state, &web.id, &p.owner, m7, &project_id, true).await;

    assert!(errors(&web.drain()).is_empty());
    assert!(p.state.db.get_project(&project_id).await.unwrap().is_some());
    assert!(p
        .state
        .db
        .project_is_placed_in_cluster(&project_id, &p.owner.to_string())
        .await
        .unwrap());
    let opened = p
        .state
        .db
        .get_project_open_state(&project_id)
        .await
        .unwrap()
        .expect("the project has a record");
    assert_eq!(opened.state, Some(ProjectOpenState::Open));
    assert_eq!(opened.set_by_account(), Some(p.owner));
    assert_eq!(opened.set_kind, Some(OpenStateSetKind::Request));
    assert_eq!(opened.machine_id, Some(m7));
    let sent_to_daemon = daemon.drain();
    assert!(
        matches!(
            sent_to_daemon.as_slice(),
            [ServerToDaemon::StartProjectCli {
                project_id: started,
                policy: Some(_),
                request_id: Some(_),
            }] if *started == project_id
        ),
        "exactly one start: {sent_to_daemon:?}"
    );

    // The instance that start brings up sends its `SessionStart`.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = crate::routes::create_router(p.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let token = encode(
        &Header::default(),
        &crate::routes::auth::Claims {
            sub: p.owner.to_string(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
            device_session_id: None,
            token_kind: None,
            credential_version: None,
        },
        &EncodingKey::from_secret(p.state.config.auth.jwt_secret.as_bytes()),
    )
    .unwrap();
    let (mut cli, _) = connect_async(format!("ws://{address}/ws/cli"))
        .await
        .unwrap();
    let frames = [
        shared::CliToServer::Register {
            token,
            version: Some("26.11.0".to_string()),
            capabilities: CAPABLE_CLI
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        },
        shared::CliToServer::SessionStart {
            session_id: registered,
            project_id: Some(registered),
            machine_id: Some(m7),
            working_dir: Some(format!("/work/{HOST_C}")),
            hostname: Some(HOST_C.to_string()),
            git_remote: None,
            git_remote_url: None,
            pane_type: None,
            panes: Some(Vec::new()),
        },
    ];
    for frame in frames {
        cli.send(Message::Text(serde_json::to_string(&frame).unwrap()))
            .await
            .unwrap();
    }
    // The server ends an instance's start by sending the project's policy.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let Message::Text(text) = cli.next().await.unwrap().unwrap() else {
                continue;
            };
            if matches!(
                serde_json::from_str::<ServerToCli>(&text).unwrap(),
                ServerToCli::ProjectPolicy { .. }
            ) {
                return;
            }
        }
    })
    .await
    .expect("the instance's start was handled");
    assert!(p
        .state
        .sessions
        .instance_on_machine(&registered, &m7)
        .is_some());
    assert_eq!(
        p.state
            .db
            .get_project_open_state(&project_id)
            .await
            .unwrap(),
        Some(opened),
        "the SessionStart records nothing"
    );
    assert!(daemon.drain().is_empty());
    server.abort();
}
