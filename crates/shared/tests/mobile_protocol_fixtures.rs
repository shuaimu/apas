use shared::{
    AddPaneStatus, CodeEvent, JobApiError, JobEventsResponse, JobMutationReceipt, MutationKind,
    ServerToWeb, WebToServer,
};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/protocol/fixtures")
}

#[test]
fn golden_mobile_fixtures_match_rust_wire_types() {
    for entry in std::fs::read_dir(fixtures_dir()).expect("read protocol fixtures") {
        let path = entry.expect("fixture entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy();
        let json = std::fs::read_to_string(&path).expect("read fixture");
        if name.starts_with("web-") {
            serde_json::from_str::<WebToServer>(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else if name.starts_with("server-") {
            serde_json::from_str::<ServerToWeb>(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else if name == "code-event.json" {
            serde_json::from_str::<CodeEvent>(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else if name == "job-receipt.json" {
            serde_json::from_str::<JobMutationReceipt>(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else if name == "job-events.json" {
            serde_json::from_str::<JobEventsResponse>(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else if name == "job-error.json" {
            serde_json::from_str::<JobApiError>(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else {
            panic!("unclassified protocol fixture: {}", path.display());
        }
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join(name)).expect("read fixture")
}

fn web(name: &str) -> WebToServer {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn server(name: &str) -> ServerToWeb {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// Both sides ignore unknown keys, so a misspelled optional field would still
/// pass the generic check above. Pin that the multi-machine fixtures really
/// carry the fields they exist to exercise.
#[test]
fn multi_machine_fixtures_populate_the_new_fields() {
    match web("web-add-pane-targeted.json") {
        WebToServer::AddPane {
            machine_id,
            request_id,
            ..
        } => {
            assert!(machine_id.is_some());
            assert_eq!(request_id.as_deref(), Some("add-pane-1"));
        }
        other => panic!("unexpected {other:?}"),
    }
    match web("web-cli-lifecycle-request-targeted.json") {
        WebToServer::CliLifecycleRequest {
            machine_id,
            operation,
            ..
        } => {
            assert!(machine_id.is_some());
            assert!(operation.is_some());
        }
        other => panic!("unexpected {other:?}"),
    }
    match web("web-create-project-instance-join.json") {
        WebToServer::CreateProjectInstance {
            join_project_id, ..
        } => assert!(join_project_id.is_some()),
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        web("web-forget-machine-panes.json"),
        WebToServer::ForgetMachinePanes { .. }
    ));

    match server("server-add-pane-starting.json") {
        ServerToWeb::AddPaneResult {
            status,
            machine_id,
            pane_id,
            error,
            ..
        } => {
            assert_eq!(status, AddPaneStatus::Starting);
            assert!(machine_id.is_some());
            assert_eq!((pane_id, error), (None, None));
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-add-pane-failed.json") {
        ServerToWeb::AddPaneResult { status, error, .. } => {
            assert_eq!(status, AddPaneStatus::Failed);
            assert!(error.is_some());
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-session-instances.json") {
        ServerToWeb::SessionInstances { instances, .. } => {
            assert_eq!(instances.len(), 2);
            assert_eq!(
                instances.iter().filter(|instance| instance.primary).count(),
                1
            );
            assert!(instances
                .iter()
                .all(|instance| instance.machine_id.is_some()));
            assert_eq!(
                instances
                    .iter()
                    .map(|instance| instance.runtime_access)
                    .collect::<Vec<_>>(),
                vec![Some(true), Some(false)]
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-session-instances-legacy.json") {
        ServerToWeb::SessionInstances { instances, .. } => {
            assert_eq!(instances.len(), 1);
            assert!(instances[0].primary);
            assert_eq!(instances[0].machine_id, None);
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-cli-lifecycle-inventory-instance.json") {
        ServerToWeb::CliLifecycleInventory {
            machine_id,
            inventory,
            ..
        } => {
            assert!(machine_id.is_some());
            assert_eq!(inventory.panes.len(), 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-cli-lifecycle-status-instance.json") {
        ServerToWeb::CliLifecycleStatus { machine_id, .. } => assert!(machine_id.is_some()),
        other => panic!("unexpected {other:?}"),
    }
    for name in [
        "web-start-machine-project-cli-all.json",
        "web-stop-machine-project-cli-all.json",
    ] {
        match web(name) {
            WebToServer::StartMachineProjectCli { all_instances, .. }
            | WebToServer::StopMachineProjectCli { all_instances, .. } => {
                assert!(all_instances, "{name}")
            }
            other => panic!("{name}: unexpected {other:?}"),
        }
    }
    match server("server-pane-list-instances.json") {
        ServerToWeb::PaneList { panes, .. } => {
            assert_eq!(panes.len(), 2);
            assert!(panes
                .iter()
                .all(|pane| pane.machine_id.is_some() && pane.hostname.is_some()));
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-sessions-instances.json") {
        ServerToWeb::Sessions { sessions } => {
            assert_eq!(sessions[0].instances.len(), 2);
            assert_eq!(sessions[0].instances[0].runtime_access, Some(true));
            assert!(sessions[0]
                .panes
                .iter()
                .all(|pane| pane.machine_id.is_some() && pane.hostname.is_some()));
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-machines-instances.json") {
        ServerToWeb::Machines { machines } => {
            assert!(machines[0].project_instances_available);
            assert!(machines[0].daemon_connected);
            assert!(machines[0].projects[0].git_remote.is_some());
        }
        other => panic!("unexpected {other:?}"),
    }
    match server("server-mutation-ack-input.json") {
        ServerToWeb::MutationAck {
            request_id,
            mutation,
            accepted,
            error,
            question_pending,
            ..
        } => {
            assert_eq!(mutation, MutationKind::Input);
            assert_eq!(request_id, "msg-1");
            assert!(!accepted);
            assert!(error.is_some());
            assert_eq!(question_pending, None);
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// The autopilot fixtures carry what they exist to exercise: a start with its
/// limits, a state with its condition, and a pane summary without a prompt.
#[test]
fn autopilot_fixtures_populate_the_new_fields() {
    use shared::{
        AutopilotAction, AutopilotLimit, AutopilotReceiptOutcome, AutopilotRepeat,
        AutopilotSupport,
    };

    match web("web-control-pane-autopilot-start.json") {
        WebToServer::ControlPaneAutopilot {
            pane_id,
            request_id,
            action,
            ..
        } => {
            assert_eq!(pane_id, 4);
            assert_eq!(request_id, "autopilot-1");
            let shared::AutopilotCommand::Start { spec } = action.validate().unwrap() else {
                panic!("expected a start");
            };
            assert_eq!(spec.repeat, AutopilotRepeat::Every { seconds: 900 });
            assert_eq!(spec.iteration_limit, AutopilotLimit::AtMost { value: 20 });
            assert_eq!(spec.time_limit, AutopilotLimit::None);
        }
        other => panic!("expected ControlPaneAutopilot, got {other:?}"),
    }
    assert!(matches!(
        web("web-control-pane-autopilot-stop.json"),
        WebToServer::ControlPaneAutopilot {
            action: AutopilotAction::Stop {
                autopilot_id: Some(_),
                now: true
            },
            ..
        }
    ));
    assert!(matches!(
        web("web-control-pane-autopilot-resume.json"),
        WebToServer::ControlPaneAutopilot {
            action: AutopilotAction::Resume { .. },
            ..
        }
    ));

    match server("server-pane-autopilot-receipt.json") {
        ServerToWeb::PaneAutopilotReceipt {
            outcome, reason, ..
        } => {
            assert_eq!(outcome, AutopilotReceiptOutcome::Refused);
            assert!(reason.is_some());
        }
        other => panic!("expected PaneAutopilotReceipt, got {other:?}"),
    }
    match server("server-pane-autopilot-state.json") {
        ServerToWeb::PaneAutopilotState {
            support,
            state,
            reported_at,
            ..
        } => {
            assert_eq!(support, AutopilotSupport::Supported);
            assert!(reported_at.is_some());
            let state = state.expect("state");
            let autopilot = state.autopilot.as_ref().expect("autopilot");
            assert_eq!(autopilot.iterations, 2);
            assert_eq!(autopilot.condition.kind, "waiting");
            assert!(autopilot.next_iteration_at.is_some());
            assert!(autopilot.deadline.is_none());
            let goal = state.omp_modes.as_ref().and_then(|modes| modes.goal.as_ref());
            assert_eq!(goal.map(|goal| goal.status.as_str()), Some("paused"));
        }
        other => panic!("expected PaneAutopilotState, got {other:?}"),
    }
    match server("server-pane-autopilot-state-ended.json") {
        ServerToWeb::PaneAutopilotState { state, .. } => {
            let ended = state.and_then(|state| state.last_ended).expect("ended");
            assert_eq!(ended.reason, "conversation_changed");
            assert_eq!(ended.spec.iteration_limit, AutopilotLimit::None);
        }
        other => panic!("expected PaneAutopilotState, got {other:?}"),
    }
    assert!(matches!(
        server("server-pane-autopilot-state-unavailable.json"),
        ServerToWeb::PaneAutopilotState {
            support: AutopilotSupport::InstanceUpdateRequired,
            state: None,
            ..
        }
    ));

    let ServerToWeb::Sessions { sessions } = server("server-sessions-autopilot.json") else {
        panic!("expected Sessions");
    };
    let panes = &sessions[0].panes;
    let summary = panes[0].autopilot.as_ref().expect("autopilot summary");
    assert_eq!(summary.support, AutopilotSupport::Supported);
    assert_eq!(summary.condition.as_deref(), Some("waiting"));
    assert_eq!(summary.iteration_limit, Some(20));
    assert_eq!(
        panes[1].autopilot.as_ref().map(|summary| summary.support),
        Some(AutopilotSupport::NeedsAgentReboot)
    );
    assert!(!fixture("server-sessions-autopilot.json").contains("prompt"));
}
