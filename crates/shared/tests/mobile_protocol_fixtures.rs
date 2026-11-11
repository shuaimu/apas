use shared::{
    AddPaneStatus, CodeEvent, JobApiError, JobEventsResponse, JobMutationReceipt, ServerToWeb,
    WebToServer,
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
}
