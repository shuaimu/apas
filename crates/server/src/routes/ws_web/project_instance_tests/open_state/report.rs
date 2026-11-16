//! Each machine's part in a project and what viewers are told of it
//! (design Decisions 7 and 11): the outcome registry, the derived parts, the
//! `project_open_state` frame, the server-filled fields on machine and
//! session entries, and the refusal for a tab whose machine is not running.

use super::follow::{machine_info, one_correlated_start, part, registers};
use super::*;
use crate::open_state::refresh_basis;
use crate::session::open_state::{start_not_seen_reason, start_unanswered_reason};
use shared::{MachinePart, ProjectHostSummary, ProjectMachinePart};

/// Every `project_open_state` frame among `messages`, in order.
fn open_state_frames(messages: Vec<ServerToWeb>) -> Vec<ServerToWeb> {
    messages
        .into_iter()
        .filter(|message| matches!(message, ServerToWeb::ProjectOpenState { .. }))
        .collect()
}

/// The machines of a report as (host, part, reason, owns tabs, connected).
type Row = (String, MachinePart, Option<String>, bool, bool);

fn rows(machines: &[ProjectMachinePart]) -> Vec<Row> {
    machines
        .iter()
        .map(|machine| {
            (
                machine.hostname.clone(),
                machine.part,
                machine.reason.clone(),
                machine.owns_tabs,
                machine.connected,
            )
        })
        .collect()
}

fn row(host: &str, part: MachinePart, reason: Option<&str>, owns: bool, connected: bool) -> Row {
    (
        host.to_string(),
        part,
        reason.map(str::to_string),
        owns,
        connected,
    )
}

/// What viewers would be told of the project now.
fn told(p: &Project) -> Vec<Row> {
    rows(
        &p.state
            .sessions
            .project_open_view(&p.project_id())
            .expect("the project's state has been read")
            .machines,
    )
}

/// The last report a viewer was sent: the state and its machines.
fn last_report(messages: Vec<ServerToWeb>) -> (Option<ProjectOpenState>, Vec<Row>) {
    match open_state_frames(messages).pop() {
        Some(ServerToWeb::ProjectOpenState {
            state, machines, ..
        }) => (state, rows(&machines)),
        _ => panic!("the viewer was sent no report"),
    }
}

fn with_error(p: &Project, host: &str, error: &str) -> shared::MachineProjectInfo {
    shared::MachineProjectInfo {
        last_error: Some(error.to_string()),
        ..listed(p, host, false)
    }
}

/// This machine's entry for the project in a machine list.
fn entry(
    machines: &[shared::MachineWithProjects],
    machine: Uuid,
    project_id: &str,
) -> shared::MachineProjectInfo {
    machines
        .iter()
        .find(|listed| listed.machine.machine_id == machine)
        .expect("the machine is listed")
        .projects
        .iter()
        .find(|project| project.project_id == project_id)
        .expect("the machine lists the project")
        .clone()
}

fn no_open_state(entry: &shared::MachineProjectInfo) -> bool {
    entry.open_state.is_none()
        && entry.part.is_none()
        && entry.part_reason.is_none()
        && entry.other_hosts.is_empty()
}

/// Scenario "A project that is not recorded and runs on one of two
/// machines", and the rest of a not-recorded project's parts: a tab owner
/// that does not run it is not started, offline or not, since nothing starts
/// it by itself; a start an older client asks for reads as starting, then
/// running; and the reason a machine stopped is its daemon's.
#[tokio::test]
async fn parts_are_reported_for_a_project_nobody_recorded() {
    let p = project().await;
    let _d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _running = p.instance(p.m5, HOST_A);
    let older = p.viewer(p.owner, OLDER_VIEWER);
    two_machine_roster(&p);
    refresh_basis(&p.state, &p.project_id()).await;

    let view = p.state.sessions.project_open_view(&p.project_id()).unwrap();
    assert_eq!(
        (view.state, view.set_by, view.set_kind, view.set_at),
        (None, None, None, None),
        "not recorded is the absence of every one"
    );
    assert_eq!(
        told(&p),
        vec![
            row(HOST_A, MachinePart::Running, None, true, true),
            row(HOST_B, MachinePart::NotStarted, None, true, false),
        ]
    );

    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    assert_eq!(
        told(&p)[1],
        row(HOST_B, MachinePart::NotStarted, None, true, true)
    );

    // An older web bundle's "Start on zoo-006": as before, nothing recorded.
    start_one(&p, &older, p.owner, p.m6).await;
    assert_eq!(sent(&mut d6), (vec![None], 0));
    assert_eq!(recorded(&p).await, None);
    assert_eq!(part(&p, p.m6), Some((MachinePart::Starting, None)));
    reports_running(&p, p.m6, HOST_B);
    assert_eq!(part(&p, p.m6), Some((MachinePart::Running, None)));
    assert_eq!(outcome(&p, p.m6), Some(MachineOutcome::Started));

    // It ends by itself; the daemon's word is the reason.
    p.state
        .sessions
        .update_daemon_projects(&p.m6, vec![with_error(&p, HOST_B, "exited with status 1")]);
    assert_eq!(
        part(&p, p.m6),
        Some((
            MachinePart::NotStarted,
            Some("exited with status 1".to_string())
        ))
    );

    // A per-machine Stop ends a start still pending there.
    start_one(&p, &older, p.owner, p.m6).await;
    assert_eq!(part(&p, p.m6), Some((MachinePart::Starting, None)));
    stop_one(&p, &older, p.owner, p.m6).await;
    assert_eq!(sent(&mut d6), (vec![None], 1));
    assert_eq!(outcome(&p, p.m6), None);
    assert_eq!(part(&p, p.m6).unwrap().0, MachinePart::NotStarted);
    assert_eq!(recorded(&p).await, None);
}

/// The parts of an open project: starting until the machine is seen running
/// the project, waiting for a tab owner that is offline, and not started
/// with the reason once the time limit has passed, which viewers are told
/// then without any other event. The daemon's own reason comes before the
/// server's guess.
#[tokio::test]
async fn an_open_projects_parts_follow_each_machines_start() {
    let p = project().await;
    let m7 = Uuid::new_v4();
    let limit = Duration::from_millis(600);
    p.state
        .sessions
        .set_pane_launch_timing(Duration::from_secs(60), limit);
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut web = p.viewer(p.owner, KNOWS);
    tabs(
        &p,
        vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
            pane(5, Some(m7), Some(HOST_C)),
        ],
    );
    set_recorded(&p, ProjectOpenState::Closed, p.owner, None).await;

    open(&p, &web, p.owner, p.m5).await;
    let asked5 = one_correlated_start(&mut d5);
    let asked6 = one_correlated_start(&mut d6);
    assert_eq!(
        told(&p),
        vec![
            row(HOST_A, MachinePart::Starting, None, true, true),
            row(HOST_B, MachinePart::Starting, None, true, true),
            row(HOST_C, MachinePart::WaitingOffline, None, true, false),
        ]
    );

    // zoo-005's daemon answers that it started the project: still starting
    // until the machine is seen running it.
    assert!(p
        .state
        .sessions
        .machine_start_answered(&p.m5, &asked5, &p.project_id(), true, None));
    assert!(matches!(
        outcome(&p, p.m5),
        Some(MachineOutcome::StartAccepted { .. })
    ));
    assert_eq!(part(&p, p.m5), Some((MachinePart::Starting, None)));
    // An answer under another id, or from another machine, changes nothing.
    assert!(!p.state.sessions.machine_start_answered(
        &p.m6,
        &asked5,
        &p.project_id(),
        false,
        Some("not mine".to_string())
    ));
    assert!(!p.state.sessions.machine_start_answered(
        &p.m6,
        &Uuid::new_v4(),
        &p.project_id(),
        false,
        Some("not asked".to_string())
    ));
    assert_eq!(part(&p, p.m6), Some((MachinePart::Starting, None)));
    reports_running(&p, p.m5, HOST_A);
    assert_eq!(part(&p, p.m5), Some((MachinePart::Running, None)));
    assert_eq!(outcome(&p, p.m5), Some(MachineOutcome::Started));
    assert_eq!(
        last_report(web.drain()).1[1],
        row(HOST_B, MachinePart::Starting, None, true, true)
    );

    // zoo-006 never answers. At the limit it reads as not started, and the
    // viewer is told with nothing else happening.
    tokio::time::sleep(limit + Duration::from_millis(250)).await;
    let unanswered = start_unanswered_reason(HOST_B);
    assert_eq!(
        unanswered,
        "zoo-006 did not answer the request to start this project."
    );
    assert_eq!(
        last_report(web.drain()),
        (
            Some(ProjectOpenState::Open),
            vec![
                row(HOST_A, MachinePart::Running, None, true, true),
                row(
                    HOST_B,
                    MachinePart::NotStarted,
                    Some(&unanswered),
                    true,
                    true
                ),
                row(HOST_C, MachinePart::WaitingOffline, None, true, false),
            ]
        )
    );
    // An answer that comes after all is still taken.
    assert!(p.state.sessions.machine_start_answered(
        &p.m6,
        &asked6,
        &p.project_id(),
        false,
        Some("Another host holds this project's claim".to_string())
    ));
    assert_eq!(
        part(&p, p.m6),
        Some((
            MachinePart::NotStarted,
            Some("Another host holds this project's claim".to_string())
        ))
    );

    // Open again is the retry. This time its daemon answers that it started
    // the project, and the machine is never seen running it.
    open(&p, &web, p.owner, p.m5).await;
    let again = one_correlated_start(&mut d6);
    assert!(nothing_sent(&mut d5));
    assert_eq!(part(&p, p.m6), Some((MachinePart::Starting, None)));
    assert!(p
        .state
        .sessions
        .machine_start_answered(&p.m6, &again, &p.project_id(), true, None));
    tokio::time::sleep(limit + Duration::from_millis(250)).await;
    let not_seen = start_not_seen_reason(HOST_B);
    assert_eq!(
        not_seen,
        "zoo-006 answered that it started this project, but is not running it."
    );
    assert_eq!(
        last_report(web.drain()).1[1],
        row(HOST_B, MachinePart::NotStarted, Some(&not_seen), true, true)
    );
    // What its daemon reports as its last error is the better reason.
    p.state
        .sessions
        .update_daemon_projects(&p.m6, vec![with_error(&p, HOST_B, "exited with status 1")]);
    assert_eq!(
        last_report(web.drain()).1[1],
        row(
            HOST_B,
            MachinePart::NotStarted,
            Some("exited with status 1"),
            true,
            true
        )
    );
    // A machine whose daemon leaves is waited for; nothing held for it
    // survives its return.
    p.state.sessions.unregister_daemon(&p.m6);
    assert_eq!(part(&p, p.m6), Some((MachinePart::WaitingOffline, None)));
    assert_eq!(outcome(&p, p.m6), None);
}

/// The parts of a closed project: a machine that still runs it, with the
/// reason when a close left it so, and a tab owner that does not, with no
/// reason, since that is where a closed project's machine should be. A
/// machine that owns no tab and runs nothing is not listed.
#[tokio::test]
async fn a_closed_projects_parts_name_what_still_runs() {
    let p = project().await;
    let m7 = Uuid::new_v4();
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let _d7 = p.daemon(m7, HOST_C, CAPABLE_DAEMON);
    let web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    let member = cluster_member(&p, &[p.m5]).await;
    let member_web = p.viewer(member, KNOWS);
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    reports_running(&p, p.m5, HOST_A);
    reports_running(&p, p.m6, HOST_B);
    // A stale refusal from the open must not read as a failure once closed.
    p.state.sessions.update_daemon_projects(
        &p.m5,
        vec![shared::MachineProjectInfo {
            last_error: Some("an older refusal".to_string()),
            ..listed(&p, HOST_A, true)
        }],
    );

    close(&p, &member_web, member, p.m5).await;
    assert!(stopped_once(&mut d5) && nothing_sent(&mut d6));
    let reason = may_not_use(member, HOST_B);
    assert_eq!(
        told(&p),
        vec![
            row(HOST_A, MachinePart::StillRunning, None, true, true),
            row(HOST_B, MachinePart::StillRunning, Some(&reason), true, true),
        ]
    );
    p.state
        .sessions
        .update_daemon_projects(&p.m5, vec![with_error(&p, HOST_A, "an older refusal")]);
    assert_eq!(
        told(&p)[0],
        row(HOST_A, MachinePart::NotStarted, None, true, true)
    );

    close(&p, &web, p.owner, p.m5).await;
    assert!(stopped_once(&mut d6));
    assert_eq!(
        told(&p)[1],
        row(HOST_B, MachinePart::StillRunning, None, true, true),
        "asked to stop, and still reported running"
    );
    p.state
        .sessions
        .update_daemon_projects(&p.m6, vec![listed(&p, HOST_B, false)]);
    assert_eq!(
        told(&p),
        vec![
            row(HOST_A, MachinePart::NotStarted, None, true, true),
            row(HOST_B, MachinePart::NotStarted, None, true, true),
        ]
    );
}

/// A viewer that negotiated `project_open_state_v1` is sent the complete
/// report when it attaches and on every change of the state or of a
/// machine's part, and nothing when nothing changed. A viewer that did not
/// negotiate it is sent no such frame at any point, and neither is a
/// connection that is not attached to the project.
#[tokio::test]
async fn the_report_goes_to_viewers_that_negotiated_it_on_attach_and_on_every_change() {
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let mut knows = p.viewer(p.owner, KNOWS);
    let mut older = p.viewer(p.owner, OLDER_VIEWER);
    // The owner's Machines page in another window: not attached.
    let mut elsewhere = {
        let id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(256);
        p.state.sessions.register_web(id, tx);
        p.state.sessions.set_web_user(id, p.owner);
        p.state
            .sessions
            .set_web_capabilities(id, KNOWS.iter().map(|c| c.to_string()).collect());
        Endpoint { id, rx }
    };
    two_machine_roster(&p);
    knows.drain();

    // On attach: the report of a project nobody has recorded.
    for viewer in [&knows.id, &older.id] {
        replay_open_state_after_attach(&p.state, viewer, &p.project_id()).await;
    }
    // Reading the project for the first time is itself a change, which every
    // attached viewer is told; the report is complete each time.
    let attached = open_state_frames(knows.drain());
    assert!(!attached.is_empty());
    assert!(attached
        .iter()
        .all(|frame| serde_json::to_value(frame).unwrap()
            == serde_json::to_value(&attached[0]).unwrap()));
    replay_open_state_after_attach(&p.state, &knows.id, &p.project_id()).await;
    assert_eq!(
        open_state_frames(knows.drain()).len(),
        1,
        "a later attach is told once"
    );
    assert_eq!(
        serde_json::to_value(&attached[0]).unwrap(),
        serde_json::json!({
            "type": "project_open_state",
            "project_id": p.project_id(),
            "machines": [
                {
                    "machine_id": p.m5,
                    "hostname": HOST_A,
                    "part": "not_started",
                    "owns_tabs": true,
                    "connected": true,
                },
                {
                    "machine_id": p.m6,
                    "hostname": HOST_B,
                    "part": "not_started",
                    "owns_tabs": true,
                    "connected": false,
                },
            ],
        })
    );

    // An Open: who set it, how and when, and each machine's part.
    open(&p, &knows, p.owner, p.m5).await;
    let request_id = one_correlated_start(&mut d5);
    let Some(ServerToWeb::ProjectOpenState {
        state,
        set_by,
        set_kind,
        set_at,
        machines,
        ..
    }) = open_state_frames(knows.drain()).pop()
    else {
        panic!("the Open is confirmed by a report");
    };
    assert_eq!(state, Some(ProjectOpenState::Open));
    assert_eq!(set_by, Some(format!("{}@example.test", p.owner)));
    assert_eq!(set_kind, Some(OpenStateSetKind::Request));
    assert_eq!(set_at, record(&p).await.set_at);
    assert_eq!(
        rows(&machines),
        vec![
            row(HOST_A, MachinePart::Starting, None, true, true),
            row(HOST_B, MachinePart::WaitingOffline, None, true, false),
        ]
    );

    // Each change of a part is one report; an inventory that changes nothing
    // is none.
    assert!(p.state.sessions.machine_start_answered(
        &p.m5,
        &request_id,
        &p.project_id(),
        true,
        None
    ));
    assert!(
        open_state_frames(knows.drain()).is_empty(),
        "accepted is still starting"
    );
    reports_running(&p, p.m5, HOST_A);
    assert_eq!(
        last_report(knows.drain()).1[0],
        row(HOST_A, MachinePart::Running, None, true, true)
    );
    reports_running(&p, p.m5, HOST_A);
    assert!(open_state_frames(knows.drain()).is_empty());

    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    one_correlated_start(&mut d6);
    assert_eq!(
        last_report(knows.drain()).1[1],
        row(HOST_B, MachinePart::Starting, None, true, true)
    );
    let b = p.instance(p.m6, HOST_B);
    assert_eq!(
        last_report(knows.drain()).1[1],
        row(HOST_B, MachinePart::Running, None, true, true)
    );
    p.state
        .sessions
        .detach_cli_instance(&p.session, &b.id)
        .unwrap();
    assert_eq!(
        last_report(knows.drain()).1[1],
        row(HOST_B, MachinePart::NotStarted, None, true, true)
    );
    // The roster: zoo-006's last tab is closed, and it has no part left.
    p.state
        .sessions
        .set_session_panes(&p.session, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    p.state.sessions.route_to_web_now(
        &p.session,
        ServerToWeb::PaneList {
            session_id: p.session,
            panes: p.state.sessions.get_session_panes(&p.session),
        },
    );
    assert_eq!(
        last_report(knows.drain()).1,
        vec![row(HOST_A, MachinePart::Running, None, true, true)]
    );

    // A Close.
    close(&p, &knows, p.owner, p.m5).await;
    assert_eq!(
        last_report(knows.drain()),
        (
            Some(ProjectOpenState::Closed),
            vec![row(HOST_A, MachinePart::StillRunning, None, true, true)]
        )
    );
    p.state
        .sessions
        .update_daemon_projects(&p.m5, vec![listed(&p, HOST_A, false)]);
    assert_eq!(
        last_report(knows.drain()).1,
        vec![row(HOST_A, MachinePart::NotStarted, None, true, true)]
    );

    // An older web bundle or native build: no frame of the new kind, ever.
    let older_frames = older.drain();
    assert!(!older_frames.is_empty(), "it is sent the frames it knows");
    assert!(open_state_frames(older_frames).is_empty());
    // A connection that is not attached is told through its machine list.
    let elsewhere_frames = elsewhere.drain();
    assert!(elsewhere_frames
        .iter()
        .any(|message| matches!(message, ServerToWeb::Machines { .. })));
    assert!(open_state_frames(elsewhere_frames).is_empty());
}

/// Machine-list entries carry the project's open state, this machine's part
/// with its reason, and the other machines that own tabs or run it, so a
/// machine list can show and confirm without attaching (scenario "The
/// machine list without an attached project"). The four fields are the
/// server's: whatever a daemon sent in them is discarded. Another account's
/// checkout with the same id is told nothing, and a viewer that does not own
/// a machine sees them only for a project it holds a role in.
#[tokio::test]
async fn machine_list_entries_carry_the_servers_open_state_fields() {
    let p = project().await;
    let pid = p.project_id();
    // A daemon that fills in the server's fields.
    let lying = |running: bool| shared::MachineProjectInfo {
        open_state: Some(ProjectOpenState::Closed),
        part: Some(MachinePart::StillRunning),
        part_reason: Some("a daemon's word".to_string()),
        other_hosts: vec![ProjectHostSummary {
            hostname: "elsewhere".to_string(),
            connected: true,
            running: true,
            owns_tabs: true,
        }],
        ..listed(&p, HOST_A, running)
    };
    let (tx, rx) = mpsc::channel(16);
    p.state.sessions.register_daemon(
        p.m5,
        p.host,
        tx,
        machine_info(p.m5, HOST_A),
        vec![lying(true)],
    );
    p.state
        .sessions
        .set_daemon_capabilities(p.m5, CAPABLE_DAEMON.iter().map(|c| c.to_string()).collect());
    let _d5 = Endpoint { id: p.m5, rx };
    let _web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);

    for _ in 0..2 {
        // Not recorded: no state, and each machine's part all the same.
        let machines = list_accessible_machines_for_user(&p.state, &p.owner).await;
        let listed5 = entry(&machines, p.m5, &pid);
        assert_eq!(listed5.open_state, None);
        assert_eq!(listed5.part, Some(MachinePart::Running));
        assert_eq!(listed5.part_reason, None);
        assert_eq!(
            listed5.other_hosts,
            vec![ProjectHostSummary {
                hostname: HOST_B.to_string(),
                connected: false,
                running: false,
                owns_tabs: true,
            }]
        );
        // The same again in a heartbeat.
        p.state
            .sessions
            .update_daemon_projects(&p.m5, vec![lying(true)]);
    }

    // Open; zoo-006 registers and its daemon refuses the start.
    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    let mut d6 = registers(&p, p.m6, HOST_B, false).await;
    let request_id = one_correlated_start(&mut d6);
    let refused = "Another host holds this project's claim";
    let mut owner_list = {
        let id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(256);
        p.state.sessions.register_web(id, tx);
        p.state.sessions.set_web_user(id, p.owner);
        Endpoint { id, rx }
    };
    assert!(p.state.sessions.machine_start_answered(
        &p.m6,
        &request_id,
        &pid,
        false,
        Some(refused.to_string())
    ));
    let expected6 = |listed6: &shared::MachineProjectInfo| {
        assert_eq!(listed6.open_state, Some(ProjectOpenState::Open));
        assert_eq!(listed6.part, Some(MachinePart::NotStarted));
        assert_eq!(listed6.part_reason.as_deref(), Some(refused));
        assert_eq!(
            listed6.other_hosts,
            vec![ProjectHostSummary {
                hostname: HOST_A.to_string(),
                connected: true,
                running: true,
                owns_tabs: true,
            }]
        );
    };
    // Pushed to the owner's machine list as the part changes, with no
    // viewer attached to the project on that connection.
    let pushed = owner_list
        .drain()
        .into_iter()
        .filter_map(|message| match message {
            ServerToWeb::Machines { machines } => Some(machines),
            _ => None,
        })
        .last()
        .expect("the owner's machine list was pushed");
    expected6(&entry(&pushed, p.m6, &pid));
    let machines = list_accessible_machines_for_user(&p.state, &p.owner).await;
    expected6(&entry(&machines, p.m6, &pid));
    assert_eq!(
        entry(&machines, p.m5, &pid).part,
        Some(MachinePart::Running)
    );
    // What the daemons themselves reported holds none of it.
    for machine in p.state.sessions.get_machines_for_user(&p.host) {
        assert!(machine.projects.iter().all(no_open_state));
    }

    // Another account's checkout with the same project id.
    let stranger = Uuid::new_v4();
    p.state.db.create_user(&user(stranger)).await.unwrap();
    let theirs = Uuid::new_v4();
    let _ds = foreign_daemon(
        &p,
        stranger,
        theirs,
        "zoo-009",
        vec![shared::MachineProjectInfo {
            part: Some(MachinePart::Running),
            ..listed(&p, "zoo-009", true)
        }],
    );
    let machines = list_accessible_machines_for_user(&p.state, &stranger).await;
    assert!(no_open_state(&entry(&machines, theirs, &pid)));
    let machines = list_accessible_machines_for_user(&p.state, &p.owner).await;
    assert!(
        entry(&machines, p.m6, &pid)
            .other_hosts
            .iter()
            .all(|host| host.hostname != "zoo-009"),
        "nor is its machine named to the project's own"
    );

    // A cluster member sees the fields for a project it holds a role in.
    let member = cluster_member(&p, &[p.m5, p.m6]).await;
    let machines = list_accessible_machines_for_user(&p.state, &member).await;
    expected6(&entry(&machines, p.m6, &pid));
    // Without the role it is not shown the project at all, and an entry
    // filled for it carries nothing, whatever it held.
    p.state
        .db
        .remove_project_member(&p.owner.to_string(), &pid, &member.to_string())
        .await
        .unwrap();
    let machines = list_accessible_machines_for_user(&p.state, &member).await;
    assert!(machines.iter().all(|machine| machine
        .projects
        .iter()
        .all(|project| project.project_id != pid)));
    let mut filled = lying(false);
    p.state
        .sessions
        .fill_project_open_state(&member, &p.m6, &mut filled);
    assert!(no_open_state(&filled));
    let mut filled = lying(false);
    p.state
        .sessions
        .fill_project_open_state(&p.owner, &p.m6, &mut filled);
    expected6(&filled);
}

/// Session lists and the mobile bootstrap carry the recorded state on each
/// entry, and nothing on the entry of a project nobody has recorded.
#[tokio::test]
async fn session_entries_carry_the_recorded_state() {
    let p = project().await;
    let other = Uuid::new_v4();
    p.state
        .db
        .authorize_project_registration(&other.to_string(), &p.owner.to_string())
        .await
        .unwrap();
    let session = |id: Uuid, project_id: Option<Uuid>| -> shared::SessionInfo {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "project_id": project_id,
            "cli_client_id": null,
            "working_dir": null,
            "hostname": null,
            "status": "active",
            "created_at": null,
            // Nothing a stored row or a client put there is kept.
            "open_state": "closed",
        }))
        .unwrap()
    };
    let mut sessions = vec![
        session(p.session, Some(p.session)),
        // A session row from before sessions carried a project id.
        session(p.session, None),
        session(Uuid::new_v4(), Some(other)),
    ];
    crate::open_state::fill_session_open_states(&p.state, sessions.iter_mut()).await;
    assert!(sessions.iter().all(|session| session.open_state.is_none()));

    set_recorded(&p, ProjectOpenState::Open, p.owner, None).await;
    crate::open_state::fill_session_open_states(&p.state, sessions.iter_mut()).await;
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.open_state)
            .collect::<Vec<_>>(),
        vec![
            Some(ProjectOpenState::Open),
            Some(ProjectOpenState::Open),
            None
        ]
    );
    let entry = serde_json::to_value(&sessions[0]).unwrap();
    assert_eq!(entry["open_state"], serde_json::json!("open"));
    assert!(serde_json::to_value(&sessions[2])
        .unwrap()
        .get("open_state")
        .is_none());
}

/// A control, a close and typed input for a tab whose machine is not running
/// the project are refused with one of two sentences (design Decision 11):
/// while that machine's daemon is connected, opening the project again
/// starts it there; while it is not, the machine is offline. The same for a
/// tab tagged for the machine and for one only attributed to its host.
#[tokio::test]
async fn a_tab_on_a_machine_that_is_not_running_is_refused_by_what_would_start_it() {
    let p = project().await;
    let mut a = p.instance(p.m5, HOST_A);
    let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
    tabs(
        &p,
        vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
            pane(5, None, Some(HOST_B)),
        ],
    );
    web.drain();

    let offline = "zoo-006 is not running this project because it is offline.";
    let connected =
        "zoo-006 is not running this project. Open the project again to start it there.";
    assert_eq!(not_running(HOST_B), offline);
    assert_eq!(not_following(HOST_B), connected);

    async fn refused(p: &Project, web: &mut Endpoint<ServerToWeb>, expected: &str) {
        for pane_id in [4, 5] {
            // A control.
            assert_eq!(
                resolve_pane_session(&p.state, &web.id, Some(p.session), None, pane_id).await,
                None
            );
            assert_eq!(errors(&web.drain()), vec![expected.to_string()]);
            // A close.
            handle_remove_pane(&p.state, &web.id, Some(p.session), None, pane_id, None).await;
            assert_eq!(errors(&web.drain()), vec![expected.to_string()]);
            // Typed input, also under its id.
            typed(p, &web.id, Some(pane_id), Some("msg")).await;
            let delivered = web.drain();
            assert_eq!(errors(&delivered), vec![expected.to_string()]);
            assert_eq!(
                input_refusals(&delivered),
                vec![("msg".to_string(), Some(pane_id), Some(expected.to_string()))]
            );
        }
        assert_eq!(p.pane_ids(), vec![3, 4, 5], "no tab was closed");
    }

    // zoo-006's daemon has not registered since the server started.
    refused(&p, &mut web, offline).await;
    // Connected, and not running the project.
    let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    refused(&p, &mut web, connected).await;
    // Gone again.
    p.state.sessions.unregister_daemon(&p.m6);
    refused(&p, &mut web, offline).await;
    assert!(a.drain().is_empty(), "nothing reached the running instance");
}

/// The server sends a project start to a daemon from an Open, a tab request,
/// a task launch, a creation and a registration, and from nothing else. This
/// reads the sources: every place outside tests that names the start frame
/// must be one of the functions below, and the one function that sends a
/// start on the server's own account must be called only by an Open, by the
/// follow step at a registration, and by a start naming one machine of an
/// open project. A new sender fails this until it is added here on purpose.
#[test]
fn a_project_start_is_sent_only_from_an_open_a_tab_a_task_a_creation_and_a_registration() {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    /// The name of the function a line declares, if it declares one.
    fn declared_fn(line: &str) -> Option<String> {
        let trimmed = line.trim_start();
        let at = trimmed.find("fn ")?;
        let before = trimmed[..at].trim();
        if !before.split_whitespace().all(|word| {
            matches!(
                word,
                "pub" | "pub(crate)" | "pub(super)" | "async" | "const"
            )
        }) {
            return None;
        }
        let name: String = trimmed[at + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    }
    /// Every line of `text` outside tests that mentions `needle`, with the
    /// function it is in. Skips comments, and every inline module declared
    /// under `#[cfg(test)]`, whose closing brace sits at its own indentation.
    fn mentions(text: &str, needle: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut skip_until: Option<String> = None;
        let mut test_item_next = false;
        let mut current = String::from("(no function)");
        for line in text.lines() {
            if let Some(close) = &skip_until {
                if line == close {
                    skip_until = None;
                }
                continue;
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.starts_with("#[cfg(test)]") {
                test_item_next = true;
                continue;
            }
            if test_item_next && !trimmed.starts_with("#[") {
                test_item_next = false;
                if trimmed.contains("mod ") && trimmed.trim_end().ends_with('{') {
                    let indent = &line[..line.len() - trimmed.len()];
                    skip_until = Some(format!("{indent}}}"));
                    continue;
                }
            }
            if let Some(name) = declared_fn(line) {
                current = name;
            }
            if line.contains(needle) {
                found.push(current.clone());
            }
        }
        found
    }

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let frame = concat!("ServerToDaemon::Start", "ProjectCli");
    let sender = concat!("send_recorded", "_start(");
    let mut senders = BTreeSet::new();
    let mut callers = BTreeSet::new();
    for path in files {
        let relative = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .to_string();
        // Test-only files: modules declared under `#[cfg(test)]`.
        if relative
            .split(['/', '.'])
            .any(|part| part == "tests" || part.ends_with("_tests"))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for function in mentions(&text, frame) {
            senders.insert(format!("{relative}: {function}"));
        }
        for function in mentions(&text, sender) {
            callers.insert(format!("{relative}: {function}"));
        }
    }
    let expect = |names: &[&str]| -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    };
    assert_eq!(
        senders,
        expect(&[
            // An Open, the follow step at a registration, and a start naming
            // one machine of an open project.
            "open_state.rs: send_recorded_start",
            // An Open that changes the record, for a machine that reads as
            // running: a stop sent before may still be taking effect there.
            "open_state.rs: send_start_to_running_machine",
            // A task launch: beside a running instance, and as the
            // project's first start.
            "routes/mobile.rs: launch_task",
            "routes/mobile.rs: task_launch_instance",
            // A creation: shared provisioning's finalize.
            "routes/ws_daemon.rs: apply_registered_daemon_message",
            // A tab request on a machine that is not running the project.
            "routes/ws_web.rs: start_instance_for_pane",
            // A start naming one machine: of an open project that already
            // runs there, and of a project nobody has recorded.
            "routes/ws_web.rs: start_one_machine_of_recorded_project",
            "routes/ws_web.rs: start_unrecorded_machine_project",
        ])
    );
    assert_eq!(
        callers,
        expect(&[
            "open_state.rs: open_project_locked",
            "open_state.rs: send_recorded_start",
            "open_state/follow.rs: follow_project_locked",
            "routes/ws_web.rs: start_one_machine_of_recorded_project",
        ])
    );
}

/// A machine an open project's start did not bring up is reported with the
/// reason whether or not it owns a tab, and also while the server holds no
/// roster for the project: the first Open of a project that has never run
/// names its machine, and a refusal, a failure or a start nothing answered
/// would otherwise show an open project with no machine and no reason.
#[tokio::test]
async fn a_machine_an_open_could_not_start_is_reported_while_the_tab_owners_are_unknown() {
    // Refused by a server check: the daemon cannot take the project policy.
    let p = project().await;
    let mut d5 = p.daemon(p.m5, HOST_A, &[PROJECT_INSTANCES_CAPABILITY]);
    let mut web = p.viewer(p.owner, KNOWS);
    assert_eq!(
        project_tab_owners(&p.state, &p.project_id(), None).await,
        TabOwners::Unknown
    );
    open(&p, &web, p.owner, p.m5).await;
    assert_eq!(recorded(&p).await, Some(ProjectOpenState::Open));
    assert!(nothing_sent(&mut d5));
    let refused = vec![row(
        HOST_A,
        MachinePart::NotStarted,
        Some(crate::open_state::POLICY_INCOMPATIBLE),
        false,
        true,
    )];
    assert_eq!(told(&p), refused);
    assert_eq!(
        last_report(web.drain()),
        (Some(ProjectOpenState::Open), refused)
    );

    // Never answered, then refused by the daemon.
    let q = project().await;
    // Long enough for the part to be read while the start is within it.
    q.state.sessions.set_pane_launch_timing(
        crate::session::pane_launches::PANE_ID_RESERVATION_TTL,
        Duration::from_secs(2),
    );
    let mut q5 = q.daemon(q.m5, HOST_A, CAPABLE_DAEMON);
    let mut q6 = q.daemon(q.m6, HOST_B, CAPABLE_DAEMON);
    let web = q.viewer(q.owner, KNOWS);
    open(&q, &web, q.owner, q.m5).await;
    let request_id = one_correlated_start(&mut q5);
    assert_eq!(
        told(&q),
        vec![row(HOST_A, MachinePart::Starting, None, false, true)]
    );
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let unanswered = start_unanswered_reason(HOST_A);
    assert_eq!(
        told(&q),
        vec![row(
            HOST_A,
            MachinePart::NotStarted,
            Some(&unanswered),
            false,
            true
        )]
    );
    assert!(q.state.sessions.machine_start_answered(
        &q.m5,
        &request_id,
        &q.project_id(),
        false,
        Some("Another host holds this project's claim".to_string()),
    ));
    assert_eq!(
        told(&q),
        vec![row(
            HOST_A,
            MachinePart::NotStarted,
            Some("Another host holds this project's claim"),
            false,
            true
        )]
    );
    assert!(nothing_sent(&mut q6), "no other machine is involved");

    // Once it runs, it reads as running; once the project is closed, a
    // machine that owns no tab and does not run it is not listed.
    reports_running(&q, q.m5, HOST_A);
    assert_eq!(
        told(&q),
        vec![row(HOST_A, MachinePart::Running, None, false, true)]
    );
    q.state
        .sessions
        .update_daemon_projects(&q.m5, vec![listed(&q, HOST_A, false)]);
    q.state
        .sessions
        .record_machine_not_started(&q.project_id(), q.m5, "it stopped");
    close(&q, &web, q.owner, q.m5).await;
    assert_eq!(told(&q), Vec::new());

    // Another account's checkout with the same id is never listed, whatever
    // is held for its machine.
    let stranger = Uuid::new_v4();
    q.state.db.create_user(&user(stranger)).await.unwrap();
    let theirs = Uuid::new_v4();
    let _df = foreign_daemon(
        &q,
        stranger,
        theirs,
        "zoo-009",
        vec![listed(&q, "zoo-009", false)],
    );
    set_recorded(&q, ProjectOpenState::Open, q.owner, Some(q.m5)).await;
    refresh_basis(&q.state, &q.project_id()).await;
    q.state
        .sessions
        .record_machine_not_started(&q.project_id(), theirs, "not theirs to start");
    assert!(told(&q).iter().all(|machine| machine.0 != "zoo-009"));
}

/// A report a viewer's full queue did not take is not counted as told: the
/// next report sends the complete frame again, although nothing changed in
/// between. A view that was delivered is not repeated.
#[tokio::test]
async fn a_report_a_full_queue_dropped_is_sent_again_by_the_next_report() {
    let p = project().await;
    let _d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::channel(1);
    p.state.sessions.register_web(id, tx.clone());
    p.state.sessions.set_web_user(id, p.owner);
    p.state.sessions.set_web_capabilities(
        id,
        KNOWS
            .iter()
            .map(|capability| capability.to_string())
            .collect(),
    );
    assert!(p.state.sessions.attach_web_to_session(&p.session, id, None));
    tabs(&p, vec![pane(3, Some(p.m5), Some(HOST_A))]);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    refresh_basis(&p.state, &p.project_id()).await;
    let drain = |rx: &mut mpsc::Receiver<ServerToWeb>| {
        let mut frames = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            frames.push(frame);
        }
        frames
    };
    drain(&mut rx);

    // The viewer's queue is full when the machine's part changes.
    tx.try_send(ServerToWeb::Error {
        message: "unread".to_string(),
    })
    .unwrap();
    p.state
        .sessions
        .record_machine_not_started(&p.project_id(), p.m5, "zoo-005 refused");
    assert!(
        open_state_frames(drain(&mut rx)).is_empty(),
        "the report was dropped"
    );

    // Nothing has changed since, and the next report still sends it.
    p.state.sessions.report_project_open_state(&p.project_id());
    assert_eq!(
        last_report(drain(&mut rx)),
        (
            Some(ProjectOpenState::Open),
            vec![row(
                HOST_A,
                MachinePart::NotStarted,
                Some("zoo-005 refused"),
                true,
                true
            )]
        )
    );

    // Told now: the same view is not sent again.
    p.state.sessions.report_project_open_state(&p.project_id());
    assert!(open_state_frames(drain(&mut rx)).is_empty());
}

/// Two changes of one project reported side by side leave every viewer on
/// the newest view. A report is derived, compared and queued as one step:
/// a second report that begins while the first is between deriving its view
/// and queuing it waits, so the first's older view is never queued after the
/// second's newer one, where nothing would correct it.
#[tokio::test]
async fn reports_made_side_by_side_leave_viewers_on_the_newest_view() {
    let p = project().await;
    let _d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
    let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
    let mut web = p.viewer(p.owner, KNOWS);
    two_machine_roster(&p);
    set_recorded(&p, ProjectOpenState::Open, p.owner, Some(p.m5)).await;
    refresh_basis(&p.state, &p.project_id()).await;
    web.drain();

    // While zoo-005's change is being reported, zoo-006's is reported from
    // another thread. Unordered, it would finish within the wait below.
    let second: std::sync::Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>> =
        Default::default();
    {
        let sessions = p.state.sessions.clone();
        let (project_id, m6) = (p.project_id(), p.m6);
        let second = second.clone();
        p.state.sessions.after_next_view_derived(move || {
            let (done, finished) = std::sync::mpsc::channel();
            *second.lock().unwrap() = Some(std::thread::spawn(move || {
                sessions.record_machine_not_started(&project_id, m6, "zoo-006 refused");
                let _ = done.send(());
            }));
            let _ = finished.recv_timeout(Duration::from_millis(300));
        });
    }
    p.state
        .sessions
        .record_machine_not_started(&p.project_id(), p.m5, "zoo-005 refused");
    second
        .lock()
        .unwrap()
        .take()
        .expect("the second report was begun")
        .join()
        .unwrap();

    let newest = vec![
        row(
            HOST_A,
            MachinePart::NotStarted,
            Some("zoo-005 refused"),
            true,
            true,
        ),
        row(
            HOST_B,
            MachinePart::NotStarted,
            Some("zoo-006 refused"),
            true,
            true,
        ),
    ];
    assert_eq!(told(&p), newest);
    assert_eq!(
        last_report(web.drain()),
        (Some(ProjectOpenState::Open), newest),
        "the last report a viewer holds is the newest view"
    );
}
