//! Wire types for a project's open state: whether the project is open or
//! closed as a whole, who set that, and each machine's part in it.
//!
//! The server records the state and computes every value here. Nothing in
//! this module is sent by a CLI or a daemon, and neither negotiates
//! [`PROJECT_OPEN_STATE_CAPABILITY`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A viewer (web or native) understands [`crate::ServerToWeb::ProjectOpenState`]
/// and tells its user what opening and closing a project do.
///
/// A viewer advertises it in `Authenticate.capabilities`; the server echoes
/// it in `Authenticated.negotiated_capabilities`. It gates two things:
///
/// - the server sends `ServerToWeb::ProjectOpenState` only to connections
///   that negotiated it. Older native builds drop their socket on a frame
///   type their bundled schema lacks;
/// - only a request from a connection that negotiated it records the state
///   of a project nobody has recorded yet, or opens the project for a new tab
///   on a machine that is not running it. See
///   [`crate::WebToServer::StartMachineProjectCli`].
///
/// The optional fields that carry the state on session and machine entries
/// (`SessionInfo::open_state`, `MachineProjectInfo::open_state` and its
/// neighbours) need no capability: older schemas accept them and older
/// clients ignore them.
///
/// CLIs and daemons never advertise it, and the server never echoes it to
/// them.
pub const PROJECT_OPEN_STATE_CAPABILITY: &str = "project_open_state_v1";

/// The recorded open state of a project.
///
/// There is a third condition, not recorded, and it is never a value: every
/// field of this type is optional and is absent for a project nobody has
/// opened or closed since the server began recording. The server neither
/// starts nor stops such a project by itself, and a client shows it from
/// what runs, as before the state existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOpenState {
    /// Every machine that owns tabs of the project runs them, or is started
    /// when its daemon next registers.
    Open,
    /// No machine runs the project, or is stopped when its daemon next
    /// registers.
    Closed,
}

/// How a project's open state was last set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpenStateSetKind {
    /// Someone opened or closed the project, or created it with a start.
    Request,
    /// A suspension, an operator's or administrator's "Stop runtime", or the
    /// start of a deletion closed it.
    Administrative,
}

/// One machine's part in a project, as the server derived it from the
/// roster, the machine's inventory, the attached instances, the starts in
/// flight and the outcome of the last start it asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MachinePart {
    /// The machine runs the project and the project is not closed.
    Running,
    /// The machine was asked to start the project and has not answered yet.
    Starting,
    /// The project is open and this machine owns tabs of it, but its daemon
    /// is not connected. It is started when the daemon registers.
    WaitingOffline,
    /// The machine owns tabs of the project and does not run them, and
    /// nothing is about to start it. The reason, when known, is beside this
    /// value. In an open project it is a machine that refused, failed or
    /// never answered. It is also the part of a tab owner that does not run
    /// a closed project, and of one that does not run a project that is not
    /// recorded, offline or not, since nothing starts it when it returns.
    NotStarted,
    /// The project is closed and this machine still runs it. The reason,
    /// when one is known, is beside this value.
    StillRunning,
}

/// One machine's part in a project, in
/// [`crate::ServerToWeb::ProjectOpenState`]. Listed for every machine that
/// owns tabs of the project or runs it, and for the machine an open named
/// while no machine owns a tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectMachinePart {
    /// `None` for a host the server knows only by name: untagged tabs are
    /// attributed to a hostname, and the server knows a machine's id only
    /// from tagged tabs or once its daemon has registered since the server
    /// started. No request can name such a host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<Uuid>,
    /// Display name of the machine; never used for routing.
    pub hostname: String,
    pub part: MachinePart,
    /// Why the machine is [`MachinePart::NotStarted`] or, when known, why it
    /// is [`MachinePart::StillRunning`]: the sentence the refusing check or
    /// the daemon gave. Shown as it is. Absent when no reason is known, as
    /// after a server restart until that machine registers again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The roster records tabs for this machine, attributes untagged tabs to
    /// its host, or, while no machine owns a tab, an open named it.
    pub owns_tabs: bool,
    /// The machine's daemon holds a live connection to the server now.
    pub connected: bool,
}

/// Another machine that owns tabs of a project or runs it, in
/// [`crate::MachineProjectInfo::other_hosts`], so a machine list can name the
/// machines an Open or a Close reaches without attaching to the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectHostSummary {
    /// Display name of the machine; never used for routing.
    pub hostname: String,
    /// The machine's daemon holds a live connection to the server now.
    pub connected: bool,
    /// The machine runs the project now.
    pub running: bool,
    /// The roster records tabs for this machine or attributes untagged tabs
    /// to its host.
    pub owns_tabs: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MachineProjectInfo, ServerToWeb, SessionInfo};
    use serde_json::json;

    #[test]
    fn capability_wire_value_is_stable() {
        assert_eq!(PROJECT_OPEN_STATE_CAPABILITY, "project_open_state_v1");
    }

    #[test]
    fn wire_values_are_stable() {
        for (value, wire) in [
            (ProjectOpenState::Open, "open"),
            (ProjectOpenState::Closed, "closed"),
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<ProjectOpenState>(json!(wire)).unwrap(),
                value
            );
        }
        for (value, wire) in [
            (OpenStateSetKind::Request, "request"),
            (OpenStateSetKind::Administrative, "administrative"),
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<OpenStateSetKind>(json!(wire)).unwrap(),
                value
            );
        }
        for (value, wire) in [
            (MachinePart::Running, "running"),
            (MachinePart::Starting, "starting"),
            (MachinePart::WaitingOffline, "waiting_offline"),
            (MachinePart::NotStarted, "not_started"),
            (MachinePart::StillRunning, "still_running"),
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<MachinePart>(json!(wire)).unwrap(),
                value
            );
        }
        // "Not recorded" is the absence of a value, never a value.
        assert!(serde_json::from_value::<ProjectOpenState>(json!("not_recorded")).is_err());
    }

    #[test]
    fn a_not_recorded_project_is_a_frame_without_a_state() {
        let frame = ServerToWeb::ProjectOpenState {
            project_id: "5f0c7c0e-3b1d-4f6e-9a55-0f5e3a1b2c3d".to_string(),
            state: None,
            set_by: None,
            set_kind: None,
            set_at: None,
            machines: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({
                "type": "project_open_state",
                "project_id": "5f0c7c0e-3b1d-4f6e-9a55-0f5e3a1b2c3d",
                "machines": [],
            })
        );
        let decoded: ServerToWeb = serde_json::from_value(json!({
            "type": "project_open_state",
            "project_id": "p",
            "machines": [],
        }))
        .unwrap();
        assert!(matches!(
            decoded,
            ServerToWeb::ProjectOpenState {
                state: None,
                set_by: None,
                set_kind: None,
                set_at: None,
                ref machines,
                ..
            } if machines.is_empty()
        ));
    }

    #[test]
    fn a_recorded_project_carries_who_set_it_and_each_machine() {
        let machine_id = Uuid::new_v4();
        let frame = ServerToWeb::ProjectOpenState {
            project_id: "p".to_string(),
            state: Some(ProjectOpenState::Open),
            set_by: Some("owner@example.com".to_string()),
            set_kind: Some(OpenStateSetKind::Request),
            set_at: Some("2026-11-16T05:00:00Z".to_string()),
            machines: vec![
                ProjectMachinePart {
                    machine_id: Some(machine_id),
                    hostname: "zoo-005".to_string(),
                    part: MachinePart::Running,
                    reason: None,
                    owns_tabs: true,
                    connected: true,
                },
                ProjectMachinePart {
                    machine_id: None,
                    hostname: "laptop".to_string(),
                    part: MachinePart::NotStarted,
                    reason: Some("laptop refused".to_string()),
                    owns_tabs: true,
                    connected: false,
                },
            ],
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["state"], json!("open"));
        assert_eq!(value["set_kind"], json!("request"));
        assert_eq!(
            value["machines"][0],
            json!({
                "machine_id": machine_id,
                "hostname": "zoo-005",
                "part": "running",
                "owns_tabs": true,
                "connected": true,
            })
        );
        assert_eq!(
            value["machines"][1],
            json!({
                "hostname": "laptop",
                "part": "not_started",
                "reason": "laptop refused",
                "owns_tabs": true,
                "connected": false,
            })
        );
        let ServerToWeb::ProjectOpenState { machines, .. } = serde_json::from_value(value).unwrap()
        else {
            panic!("expected ProjectOpenState");
        };
        assert_eq!(machines[1].machine_id, None);
        assert_eq!(machines[1].part, MachinePart::NotStarted);
    }

    /// A machine's part is a closed vocabulary: a viewer that negotiated the
    /// capability must not be sent a value its schema lacks.
    #[test]
    fn an_invented_part_or_state_is_refused() {
        let frame = |state: &str, part: &str| {
            json!({
                "type": "project_open_state",
                "project_id": "p",
                "state": state,
                "machines": [{
                    "hostname": "zoo-005",
                    "part": part,
                    "owns_tabs": true,
                    "connected": true,
                }],
            })
        };
        assert!(serde_json::from_value::<ServerToWeb>(frame("open", "running")).is_ok());
        assert!(serde_json::from_value::<ServerToWeb>(frame("open", "paused")).is_err());
        assert!(serde_json::from_value::<ServerToWeb>(frame("ajar", "running")).is_err());
    }

    /// Daemons send `MachineProjectInfo` and stored session rows become
    /// `SessionInfo`, so an entry without the new fields must load, and one
    /// that has none set must not gain keys.
    #[test]
    fn entry_fields_are_optional_and_omitted_when_unset() {
        let project: MachineProjectInfo =
            serde_json::from_value(json!({ "project_id": "legacy", "path": "/legacy" })).unwrap();
        assert_eq!(project.open_state, None);
        assert_eq!(project.part, None);
        assert_eq!(project.part_reason, None);
        assert!(project.other_hosts.is_empty());
        let value = serde_json::to_value(&project).unwrap();
        for key in ["open_state", "part", "part_reason", "other_hosts"] {
            assert!(value.get(key).is_none(), "{key}");
        }

        let session: SessionInfo = serde_json::from_value(json!({
            "id": Uuid::new_v4(),
            "cli_client_id": null,
            "working_dir": null,
            "hostname": null,
            "status": "active",
            "created_at": null,
        }))
        .unwrap();
        assert_eq!(session.open_state, None);
        assert!(serde_json::to_value(&session)
            .unwrap()
            .get("open_state")
            .is_none());
    }

    /// The mobile bootstrap's session entry flattens `SessionInfo`, so the
    /// state rides there as the same top-level key and needs no field of
    /// its own.
    #[test]
    fn the_mobile_bootstrap_session_entry_carries_the_state() {
        let entry = json!({
            "id": Uuid::new_v4(),
            "cli_client_id": null,
            "working_dir": null,
            "hostname": null,
            "status": "active",
            "created_at": null,
            "open_state": "closed",
            "project_name": "project",
        });
        let summary: crate::MobileSessionSummary = serde_json::from_value(entry).unwrap();
        assert_eq!(summary.session.open_state, Some(ProjectOpenState::Closed));
        let value = serde_json::to_value(&summary).unwrap();
        assert_eq!(value["open_state"], json!("closed"));
        assert!(value.get("session").is_none());

        let schema =
            serde_json::to_value(schemars::schema_for!(crate::MobileSessionSummary)).unwrap();
        assert!(schema["properties"].get("open_state").is_some());
        assert!(schema.get("additionalProperties").is_none());
        assert!(schema.get("unevaluatedProperties").is_none());
    }

    /// The gate is the frame, not the fields: the frame is a variant of
    /// `ServerToWeb`, and the entries that carry the fields stay open
    /// objects, so a schema from before them accepts them.
    #[test]
    fn the_frame_is_a_tagged_variant_and_the_entries_stay_open() {
        let schema = serde_json::to_value(schemars::schema_for!(ServerToWeb)).unwrap();
        let variants = schema["oneOf"].as_array().expect("variants");
        let tagged = variants
            .iter()
            .filter(|variant| variant["properties"]["type"]["const"] == json!("project_open_state"))
            .count();
        assert_eq!(tagged, 1);
        for name in ["SessionInfo", "MachineProjectInfo", "MachineWithProjects"] {
            let definition = &schema["$defs"][name];
            assert!(definition.is_object(), "{name}");
            assert!(definition.get("additionalProperties").is_none(), "{name}");
            assert!(definition.get("unevaluatedProperties").is_none(), "{name}");
        }
        for name in [
            "ProjectOpenState",
            "OpenStateSetKind",
            "MachinePart",
            "ProjectMachinePart",
            "ProjectHostSummary",
        ] {
            assert!(schema["$defs"][name].is_object(), "{name}");
        }
    }

    #[test]
    fn entry_fields_round_trip_when_set() {
        let mut project: MachineProjectInfo =
            serde_json::from_value(json!({ "project_id": "p", "path": "/p" })).unwrap();
        project.open_state = Some(ProjectOpenState::Open);
        project.part = Some(MachinePart::NotStarted);
        project.part_reason = Some("zoo-006 refused".to_string());
        project.other_hosts = vec![ProjectHostSummary {
            hostname: "zoo-005".to_string(),
            connected: true,
            running: true,
            owns_tabs: true,
        }];
        let value = serde_json::to_value(&project).unwrap();
        assert_eq!(value["open_state"], json!("open"));
        assert_eq!(value["part"], json!("not_started"));
        assert_eq!(value["part_reason"], json!("zoo-006 refused"));
        assert_eq!(
            value["other_hosts"],
            json!([{ "hostname": "zoo-005", "connected": true, "running": true, "owns_tabs": true }])
        );
        let decoded: MachineProjectInfo = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.open_state, Some(ProjectOpenState::Open));
        assert_eq!(decoded.part, Some(MachinePart::NotStarted));
        assert_eq!(decoded.other_hosts, project.other_hosts);

        let mut session: SessionInfo = serde_json::from_value(json!({
            "id": Uuid::new_v4(),
            "cli_client_id": null,
            "working_dir": null,
            "hostname": null,
            "status": "active",
            "created_at": null,
        }))
        .unwrap();
        session.open_state = Some(ProjectOpenState::Closed);
        let value = serde_json::to_value(&session).unwrap();
        assert_eq!(value["open_state"], json!("closed"));
        let decoded: SessionInfo = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.open_state, Some(ProjectOpenState::Closed));
    }
}
