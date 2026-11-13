//! Autopilot for OMP terminal tabs, as the session manager holds it: the last
//! state each tab's instance reported, what viewers are told about a tab, and
//! the requests waiting for an instance's receipt.
//!
//! Nothing here records which autopilots were forwarded. An autopilot lives
//! in its tab's OMP session and outlives a server restart there; the cache is
//! rebuilt from what instances report after they attach.

use super::instances::{resolve_pane_owner, PaneOwner};
use super::{SessionManager, SessionState};
use shared::{
    AutopilotCommand, AutopilotReceiptOutcome, AutopilotSupport, PaneAutopilotState, PaneConfig,
    PaneMode, Provider, ServerToCli, ServerToWeb, OMP_TAB_AUTOPILOT_CAPABILITY,
    TAB_AUTOPILOT_CAPABILITY,
};
use tokio::sync::oneshot;
use uuid::Uuid;

/// How many of an account's most recent request ids are remembered.
const MAX_TRACKED_REQUEST_IDS: usize = 128;

/// Whether autopilot applies to `pane`: an OMP terminal tab that is not
/// read-only legacy bot history.
pub(crate) fn is_autopilot_tab(pane: &PaneConfig) -> bool {
    pane.kind.is_terminal() && pane.provider == Provider::Omp && pane.mode != PaneMode::Deadloop
}

/// The instance that runs the OMP terminal tab `pane`, as autopilot sees it:
/// who may report its state and who is sent its requests.
///
/// This is the owner every pane control resolves, with one difference. A tab
/// that names neither a machine nor a host goes to the primary as a fallback,
/// which is harmless for a keystroke. With several machines attached, each
/// tags the tabs it runs, so such a tab is one none of them runs: an
/// autopilot request for it is refused, never handed to the primary, and
/// nothing the primary says about it is shown.
fn autopilot_tab_owner(session: &SessionState, pane: &PaneConfig) -> PaneOwner {
    let attributed = pane.machine_id.is_some()
        || pane
            .hostname
            .as_deref()
            .is_some_and(|host| !host.trim().is_empty());
    if !attributed && session.instances.len() > 1 {
        return PaneOwner::NotRunning {
            machine_id: None,
            hostname: None,
        };
    }
    resolve_pane_owner(session, pane.pane_id)
}

/// The last `CliToServer::PaneAutopilotState` accepted for a tab.
#[derive(Debug, Clone)]
pub struct PaneAutopilotReport {
    /// The connection that sent it. It speaks for the tab only while it is
    /// the attached instance that owns the tab: a connection that replaced it
    /// reports every tab again after its own registration.
    pub reporter: Uuid,
    pub support: AutopilotSupport,
    pub state: Option<Box<PaneAutopilotState>>,
    pub reported_at: Option<String>,
}

/// Autopilot on one tab as viewers are told: the payload of
/// `ServerToWeb::PaneAutopilotState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneAutopilotAvailability {
    pub support: AutopilotSupport,
    pub state: Option<Box<PaneAutopilotState>>,
    pub reported_at: Option<String>,
}

impl PaneAutopilotAvailability {
    fn bare(support: AutopilotSupport) -> Self {
        Self {
            support,
            state: None,
            reported_at: None,
        }
    }

    fn frame(self, session_id: Uuid, pane_id: u32) -> ServerToWeb {
        ServerToWeb::PaneAutopilotState {
            session_id,
            pane_id,
            support: self.support,
            state: self.state,
            reported_at: self.reported_at,
        }
    }
}

/// How a tab's instance answered one forwarded request.
#[derive(Debug)]
pub(crate) struct AutopilotReceipt {
    pub outcome: AutopilotReceiptOutcome,
    pub reason: Option<String>,
}

/// A request forwarded to `cli_id` whose receipt has not arrived.
pub(super) struct AutopilotWaiter {
    pub(super) cli_id: Uuid,
    sender: oneshot::Sender<AutopilotReceipt>,
}

/// What an autopilot request for one pane is aimed at.
#[derive(Debug)]
pub(crate) enum AutopilotTarget {
    /// The roster does not list the pane.
    Unlisted,
    /// Read-only legacy bot history.
    LegacyBotHistory,
    /// A structured pane, or a terminal tab of another provider.
    NotOmpTerminal,
    /// An OMP terminal tab, and the instance that owns it.
    Tab(PaneOwner),
}

/// Why a checked request did not reach its tab's instance.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AutopilotForwardError {
    /// This same request id is already waiting for this tab.
    AlreadyWaiting,
    /// The tab's instance is no longer the one the request was checked
    /// against, or its connection did not take the frame. Nothing was sent.
    Undelivered,
}

impl SessionManager {
    /// Remember a viewer's request id. False for one this account sent
    /// before: a repeated copy of a request has no further effect.
    pub(crate) fn claim_autopilot_request(&self, user_id: Uuid, request_id: &str) -> bool {
        let mut seen = self.autopilot_request_ids.entry(user_id).or_default();
        if seen.iter().any(|id| id == request_id) {
            return false;
        }
        seen.push_back(request_id.to_string());
        while seen.len() > MAX_TRACKED_REQUEST_IDS {
            seen.pop_front();
        }
        true
    }

    /// Wait for the receipt of a request about to be forwarded to `cli_id`.
    /// Waiters are kept by request id, never one per tab, so a stop is not
    /// turned away because another request for its tab is still being
    /// answered. `None` only when this same id is already waiting for this
    /// tab.
    pub(crate) fn begin_autopilot_request(
        &self,
        session_id: Uuid,
        pane_id: u32,
        cli_id: Uuid,
        request_id: &str,
    ) -> Option<oneshot::Receiver<AutopilotReceipt>> {
        use dashmap::mapref::entry::Entry;
        let Entry::Vacant(entry) =
            self.autopilot_waiters
                .entry((session_id, pane_id, request_id.to_string()))
        else {
            return None;
        };
        let (sender, receiver) = oneshot::channel();
        entry.insert(AutopilotWaiter { cli_id, sender });
        Some(receiver)
    }

    /// Hand an instance's receipt to the request waiting for it. Only the
    /// connection the request was forwarded to can answer it.
    pub(crate) fn finish_autopilot_request(
        &self,
        session_id: Uuid,
        pane_id: u32,
        cli_id: Uuid,
        request_id: &str,
        receipt: AutopilotReceipt,
    ) -> bool {
        self.autopilot_waiters
            .remove_if(
                &(session_id, pane_id, request_id.to_string()),
                |_, waiter| waiter.cli_id == cli_id,
            )
            .is_some_and(|(_, waiter)| waiter.sender.send(receipt).is_ok())
    }

    /// Drop the waiter of a request nobody waits for any more: its requester
    /// gave up, or the request was never delivered. A waiter that is still
    /// being waited on is left alone.
    pub(crate) fn drop_abandoned_autopilot_request(
        &self,
        session_id: Uuid,
        pane_id: u32,
        request_id: &str,
    ) {
        self.autopilot_waiters.remove_if(
            &(session_id, pane_id, request_id.to_string()),
            |_, waiter| waiter.sender.is_closed(),
        );
    }

    /// What a request for `pane_id` is aimed at: the tab and its owner, read
    /// together under the session's lock. Read apart, a tab closed in between
    /// would resolve as an unlisted pane, which routing gives to the primary.
    pub(crate) fn autopilot_target(&self, session_id: &Uuid, pane_id: u32) -> AutopilotTarget {
        let Some(session) = self.sessions.get(session_id) else {
            return AutopilotTarget::Unlisted;
        };
        match session.panes.iter().find(|pane| pane.pane_id == pane_id) {
            None => AutopilotTarget::Unlisted,
            Some(pane) if pane.mode == PaneMode::Deadloop => AutopilotTarget::LegacyBotHistory,
            Some(pane) if !is_autopilot_tab(pane) => AutopilotTarget::NotOmpTerminal,
            Some(pane) => AutopilotTarget::Tab(autopilot_tab_owner(&session, pane)),
        }
    }

    /// Send a checked request to `cli_id`, the instance it was checked
    /// against, and wait for that instance's receipt.
    ///
    /// The checks awaited the database, so the tab's owner is read once more
    /// here, with the send under the same lock: a request never reaches a
    /// connection that was replaced or whose tab was closed meanwhile. The
    /// frame is queued without waiting. The caller is the requester's read
    /// loop, which a stalled instance must not hold up, and a request that
    /// cannot be handed over now is refused, not kept for later; queuing in
    /// place also keeps one connection's requests in the order it sent them.
    pub(crate) fn forward_autopilot_request(
        &self,
        session_id: Uuid,
        pane_id: u32,
        cli_id: Uuid,
        request_id: &str,
        command: AutopilotCommand,
    ) -> Result<oneshot::Receiver<AutopilotReceipt>, AutopilotForwardError> {
        // Cloned before the session is locked, so the sender map is never
        // locked inside a session's lock.
        let sender = self
            .cli_senders
            .get(&cli_id)
            .map(|sender| sender.clone())
            .ok_or(AutopilotForwardError::Undelivered)?;
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(AutopilotForwardError::Undelivered)?;
        let still_owned = session
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id && is_autopilot_tab(pane))
            .is_some_and(|pane| {
                matches!(
                    autopilot_tab_owner(&session, pane),
                    PaneOwner::Instance(owner) if owner.cli_id == cli_id
                )
            });
        if !still_owned {
            return Err(AutopilotForwardError::Undelivered);
        }
        let receiver = self
            .begin_autopilot_request(session_id, pane_id, cli_id, request_id)
            .ok_or(AutopilotForwardError::AlreadyWaiting)?;
        let frame = ServerToCli::ControlPaneAutopilot {
            session_id,
            pane_id,
            request_id: request_id.to_string(),
            command,
        };
        if sender.try_send(frame).is_err() {
            drop(receiver);
            self.drop_abandoned_autopilot_request(session_id, pane_id, request_id);
            return Err(AutopilotForwardError::Undelivered);
        }
        Ok(receiver)
    }

    /// Cache what `cli_id` reported for one tab and tell capable viewers.
    ///
    /// A report from a connection that did not advertise autopilot or runs no
    /// instance of the session is ignored. So is one for a pane the roster
    /// lists as something other than an OMP terminal tab, or as a tab another
    /// instance owns: the inbound check lets any instance report an untagged
    /// pane, and a report from the wrong one would replace what the tab's own
    /// instance said. One for a pane the roster does not list yet is kept: a
    /// new tab's first frames can arrive before the roster report that lists
    /// it, and viewers are told when that report adds the tab.
    pub(crate) fn report_pane_autopilot(
        &self,
        session_id: &Uuid,
        cli_id: Uuid,
        pane_id: u32,
        support: AutopilotSupport,
        state: Option<Box<PaneAutopilotState>>,
        reported_at: Option<String>,
    ) {
        if !self.cli_supports_capability(&cli_id, OMP_TAB_AUTOPILOT_CAPABILITY) {
            tracing::debug!(%session_id, %cli_id, pane_id, "ignoring an autopilot state from a CLI that did not advertise autopilot");
            return;
        }
        // Only the server says that an instance needs an update. A CLI that
        // sent the value could otherwise disable the control in its own name.
        let support = match support {
            AutopilotSupport::InstanceUpdateRequired => AutopilotSupport::NotReported,
            support => support,
        };
        {
            let Some(mut session) = self.sessions.get_mut(session_id) else {
                return;
            };
            // A replaced connection must not write over what the one that
            // took its place reported.
            if !session
                .instances
                .iter()
                .any(|instance| instance.cli_id == cli_id)
            {
                return;
            }
            if let Some(pane) = session.panes.iter().find(|pane| pane.pane_id == pane_id) {
                if !is_autopilot_tab(pane) {
                    tracing::debug!(%session_id, %cli_id, pane_id, "ignoring an autopilot state for a pane that is not an OMP terminal tab");
                    return;
                }
                let owned = matches!(
                    autopilot_tab_owner(&session, pane),
                    PaneOwner::Instance(owner) if owner.cli_id == cli_id
                );
                if !owned {
                    tracing::debug!(%session_id, %cli_id, pane_id, "ignoring an autopilot state from an instance that does not own the tab");
                    return;
                }
            }
            session.pane_autopilot.insert(
                pane_id,
                PaneAutopilotReport {
                    reporter: cli_id,
                    support,
                    state,
                    reported_at,
                },
            );
        }
        self.broadcast_pane_autopilot(session_id, Some(pane_id));
    }

    /// What viewers are told about autopilot on one tab, or `None` for a pane
    /// that is not an OMP terminal tab of the roster.
    pub(crate) fn pane_autopilot_availability(
        &self,
        session_id: &Uuid,
        pane_id: u32,
    ) -> Option<PaneAutopilotAvailability> {
        let session = self.sessions.get(session_id)?;
        self.availability_in(&session, pane_id)
    }

    /// [`Self::pane_autopilot_availability`] for every OMP terminal tab of
    /// the roster, in roster order.
    pub(crate) fn pane_autopilot_availabilities(
        &self,
        session_id: &Uuid,
    ) -> Vec<(u32, PaneAutopilotAvailability)> {
        let Some(session) = self.sessions.get(session_id) else {
            return Vec::new();
        };
        session
            .panes
            .iter()
            .filter_map(|pane| {
                self.availability_in(&session, pane.pane_id)
                    .map(|availability| (pane.pane_id, availability))
            })
            .collect()
    }

    /// Viewers are not told an instance's capabilities, so two values are the
    /// server's own: instance update required, when the tab's instance is
    /// attached without autopilot, and not reported, when that instance has
    /// sent nothing or is not attached. Otherwise this is what the instance
    /// last reported, with the time it last saw the tab refresh its state.
    fn availability_in(
        &self,
        session: &SessionState,
        pane_id: u32,
    ) -> Option<PaneAutopilotAvailability> {
        let pane = session.panes.iter().find(|pane| pane.pane_id == pane_id)?;
        if !is_autopilot_tab(pane) {
            return None;
        }
        let owner = match autopilot_tab_owner(session, pane) {
            PaneOwner::Instance(owner) => owner,
            PaneOwner::NotRunning { .. } | PaneOwner::NoInstance => {
                return Some(PaneAutopilotAvailability::bare(
                    AutopilotSupport::NotReported,
                ));
            }
        };
        if !self.cli_supports_capability(&owner.cli_id, OMP_TAB_AUTOPILOT_CAPABILITY) {
            return Some(PaneAutopilotAvailability::bare(
                AutopilotSupport::InstanceUpdateRequired,
            ));
        }
        Some(
            match session
                .pane_autopilot
                .get(&pane_id)
                .filter(|report| report.reporter == owner.cli_id)
            {
                Some(report) => PaneAutopilotAvailability {
                    support: report.support,
                    state: report.state.clone(),
                    reported_at: report.reported_at.clone(),
                },
                None => PaneAutopilotAvailability::bare(AutopilotSupport::NotReported),
            },
        )
    }

    /// The frame that tells a viewer about autopilot on one tab, as it stands
    /// when this is called; `None` for a pane that is not an OMP terminal tab.
    /// The caller sends it only to a connection that negotiated
    /// [`TAB_AUTOPILOT_CAPABILITY`].
    pub(crate) fn pane_autopilot_frame(
        &self,
        session_id: &Uuid,
        pane_id: u32,
    ) -> Option<ServerToWeb> {
        self.pane_autopilot_availability(session_id, pane_id)
            .map(|availability| availability.frame(*session_id, pane_id))
    }

    /// The session's viewers that negotiated [`TAB_AUTOPILOT_CAPABILITY`].
    /// Older native builds drop their socket on a frame type their schema
    /// does not list, so the autopilot frames go to these connections one by
    /// one and never through `route_to_web`.
    fn autopilot_viewers(&self, session_id: &Uuid) -> Vec<Uuid> {
        let viewers: Vec<Uuid> = self
            .sessions
            .get(session_id)
            .map(|session| session.web_connection_ids.clone())
            .unwrap_or_default();
        viewers
            .into_iter()
            .filter(|connection_id| {
                self.web_supports_capability(connection_id, TAB_AUTOPILOT_CAPABILITY)
            })
            .collect()
    }

    fn send_autopilot_frames(&self, viewers: &[Uuid], frames: &[ServerToWeb]) {
        for connection_id in viewers {
            let sender = self
                .web_senders
                .get(connection_id)
                .map(|sender| sender.clone());
            let Some(sender) = sender else { continue };
            for frame in frames {
                let _ = sender.try_send(frame.clone());
            }
        }
    }

    /// Tell capable viewers the autopilot availability of `pane_id`, or of
    /// every OMP terminal tab of the session for `None`.
    pub(crate) fn broadcast_pane_autopilot(&self, session_id: &Uuid, pane_id: Option<u32>) {
        let viewers = self.autopilot_viewers(session_id);
        if viewers.is_empty() {
            return;
        }
        let frames: Vec<ServerToWeb> = match pane_id {
            Some(pane_id) => self
                .pane_autopilot_frame(session_id, pane_id)
                .into_iter()
                .collect(),
            None => self
                .pane_autopilot_availabilities(session_id)
                .into_iter()
                .map(|(pane_id, availability)| availability.frame(*session_id, pane_id))
                .collect(),
        };
        self.send_autopilot_frames(&viewers, &frames);
    }

    /// Tell capable viewers about each OMP terminal tab whose availability is
    /// not what `before` holds for it: a tab a roster report added, or one the
    /// report gave to another instance. `before` is
    /// [`Self::pane_autopilot_availabilities`] read ahead of the report.
    pub(crate) fn broadcast_pane_autopilot_changes(
        &self,
        session_id: &Uuid,
        before: &[(u32, PaneAutopilotAvailability)],
    ) {
        let viewers = self.autopilot_viewers(session_id);
        if viewers.is_empty() {
            return;
        }
        let frames: Vec<ServerToWeb> = self
            .pane_autopilot_availabilities(session_id)
            .into_iter()
            .filter(|current| !before.contains(current))
            .map(|(pane_id, availability)| availability.frame(*session_id, pane_id))
            .collect();
        self.send_autopilot_frames(&viewers, &frames);
    }

    /// The cached report for a tab, whoever sent it.
    #[cfg(test)]
    pub(crate) fn cached_pane_autopilot(
        &self,
        session_id: &Uuid,
        pane_id: u32,
    ) -> Option<PaneAutopilotReport> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.pane_autopilot.get(&pane_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::AttachOutcome;
    use shared::{
        autopilot_condition, AutopilotCondition, AutopilotLimit, AutopilotRepeat, AutopilotSpec,
        AutopilotView, PaneKind, PaneType, ServerToCli, PROJECT_INSTANCES_CAPABILITY,
    };
    use tokio::sync::mpsc;

    const HOST_A: &str = "zoo-005";
    const HOST_B: &str = "zoo-006";

    struct Fixture {
        manager: SessionManager,
        user: Uuid,
        session: Uuid,
        m5: Uuid,
        m6: Uuid,
    }

    fn fixture() -> Fixture {
        Fixture {
            manager: SessionManager::new(),
            user: Uuid::new_v4(),
            session: Uuid::new_v4(),
            m5: Uuid::new_v4(),
            m6: Uuid::new_v4(),
        }
    }

    struct Cli {
        id: Uuid,
        _rx: mpsc::Receiver<ServerToCli>,
    }

    impl Fixture {
        /// A registered CLI that can share the project with other machines.
        /// `autopilot` says whether it also advertised autopilot: one that
        /// did not is the CLI of a machine not yet rebooted to update.
        fn cli(&self, autopilot: bool) -> Cli {
            let id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(16);
            self.manager.register_cli(id, self.user, tx, None);
            let mut capabilities = vec![PROJECT_INSTANCES_CAPABILITY.to_string()];
            if autopilot {
                capabilities.push(OMP_TAB_AUTOPILOT_CAPABILITY.to_string());
            }
            self.manager.set_cli_capabilities(id, capabilities);
            Cli { id, _rx: rx }
        }

        fn attach(&self, cli: &Cli, machine_id: Uuid, host: &str) -> Vec<Uuid> {
            match self.manager.attach_cli_instance(
                self.session,
                cli.id,
                Some(machine_id),
                Some(format!("/work/{host}")),
                Some(host.to_string()),
            ) {
                AttachOutcome::Attached { replaced, .. } => replaced,
                AttachOutcome::Rejected { reason } => panic!("expected an attach, got {reason}"),
            }
        }

        /// An attached instance of the project on `machine_id`.
        fn instance(&self, machine_id: Uuid, host: &str, autopilot: bool) -> Cli {
            let cli = self.cli(autopilot);
            self.attach(&cli, machine_id, host);
            cli
        }

        /// A web connection viewing the session that negotiated
        /// `capabilities`.
        fn viewer(&self, capabilities: &[&str]) -> mpsc::Receiver<ServerToWeb> {
            let id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(256);
            self.manager.register_web(id, tx);
            self.manager.set_web_user(id, self.user);
            self.manager.set_web_capabilities(
                id,
                capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            assert!(self.manager.attach_web_to_session(&self.session, id, None));
            rx
        }

        /// Set the project's roster. Attaching is what creates a session in
        /// memory, so a connection nothing else knows attaches first.
        fn roster(&self, panes: Vec<PaneConfig>) {
            self.manager
                .attach_web_to_session(&self.session, Uuid::new_v4(), None);
            self.manager.set_session_panes(&self.session, panes);
        }

        fn report(&self, cli: &Cli, pane_id: u32, support: AutopilotSupport, iterations: u64) {
            self.manager.report_pane_autopilot(
                &self.session,
                cli.id,
                pane_id,
                support,
                Some(on_autopilot(iterations)),
                Some(format!("2026-11-13T14:{iterations:02}:00Z")),
            );
        }

        fn support(&self, pane_id: u32) -> Option<AutopilotSupport> {
            self.manager
                .pane_autopilot_availability(&self.session, pane_id)
                .map(|availability| availability.support)
        }

        fn cached(&self, pane_id: u32) -> bool {
            self.manager
                .cached_pane_autopilot(&self.session, pane_id)
                .is_some()
        }
    }

    fn tab(pane_id: u32, provider: Provider, machine_id: Uuid, host: &str) -> PaneConfig {
        let mut pane = PaneConfig::defaults().remove(0);
        pane.pane_id = pane_id;
        pane.kind = PaneKind::Terminal;
        pane.provider = provider;
        pane.label = None;
        pane.machine_id = Some(machine_id);
        pane.hostname = Some(host.to_string());
        pane
    }

    fn omp_tab(pane_id: u32, machine_id: Uuid, host: &str) -> PaneConfig {
        tab(pane_id, Provider::Omp, machine_id, host)
    }

    /// A tab waiting for its next iteration after `iterations` of them.
    fn on_autopilot(iterations: u64) -> Box<PaneAutopilotState> {
        Box::new(PaneAutopilotState {
            autopilot: Some(AutopilotView {
                autopilot_id: Uuid::nil(),
                spec: AutopilotSpec {
                    prompt: "Check the build and fix what fails.".into(),
                    repeat: AutopilotRepeat::Every { seconds: 900 },
                    iteration_limit: AutopilotLimit::None,
                    time_limit: AutopilotLimit::None,
                    end_without_tool_call: false,
                },
                started_at: "2026-11-13T14:00:00Z".into(),
                iterations,
                deadline: None,
                next_iteration_at: Some("2026-11-13T14:30:00Z".into()),
                condition: AutopilotCondition {
                    kind: autopilot_condition::WAITING.into(),
                    detail: Some(autopilot_condition::WAIT_NEXT_ITERATION.into()),
                    since: None,
                },
            }),
            ..PaneAutopilotState::default()
        })
    }

    /// The autopilot frames a viewer was sent since it was last read: the
    /// pane, its availability, the iterations of a reported autopilot, and
    /// the time the state was last reported.
    fn autopilot_frames(
        rx: &mut mpsc::Receiver<ServerToWeb>,
    ) -> Vec<(u32, AutopilotSupport, Option<u64>, Option<String>)> {
        let mut frames = Vec::new();
        while let Ok(message) = rx.try_recv() {
            match message {
                ServerToWeb::PaneAutopilotState {
                    pane_id,
                    support,
                    state,
                    reported_at,
                    ..
                } => frames.push((
                    pane_id,
                    support,
                    state
                        .and_then(|state| state.autopilot)
                        .map(|autopilot| autopilot.iterations),
                    reported_at,
                )),
                ServerToWeb::PaneAutopilotReceipt { .. } => {
                    panic!("a receipt is never broadcast")
                }
                _ => {}
            }
        }
        frames
    }

    #[test]
    fn a_repeated_request_id_is_claimed_once_per_account() {
        let f = fixture();
        let other = Uuid::new_v4();
        assert!(f.manager.claim_autopilot_request(f.user, "request-1"));
        assert!(!f.manager.claim_autopilot_request(f.user, "request-1"));
        assert!(f.manager.claim_autopilot_request(other, "request-1"));
        assert!(f.manager.claim_autopilot_request(f.user, "request-2"));

        // Only the most recent ids are remembered.
        for index in 0..MAX_TRACKED_REQUEST_IDS {
            assert!(f
                .manager
                .claim_autopilot_request(f.user, &format!("filler-{index}")));
        }
        assert!(f.manager.claim_autopilot_request(f.user, "request-1"));
        assert!(!f
            .manager
            .claim_autopilot_request(f.user, &format!("filler-{}", MAX_TRACKED_REQUEST_IDS - 1)));
        assert!(
            f.manager.autopilot_request_ids.get(&f.user).unwrap().len() <= MAX_TRACKED_REQUEST_IDS
        );
    }

    /// Two requests for one tab wait side by side, each answered by the
    /// connection it was forwarded to and by no other.
    #[tokio::test]
    async fn requests_wait_by_id_and_never_one_per_tab() {
        let f = fixture();
        let a = f.instance(f.m5, HOST_A, true);
        let b = f.instance(f.m6, HOST_B, true);
        let accepted = || AutopilotReceipt {
            outcome: AutopilotReceiptOutcome::Accepted,
            reason: None,
        };

        let start = f
            .manager
            .begin_autopilot_request(f.session, 7, a.id, "start-1")
            .expect("the start waits");
        let stop = f
            .manager
            .begin_autopilot_request(f.session, 7, a.id, "stop-1")
            .expect("a stop waits beside the start for the same tab");
        assert!(
            f.manager
                .begin_autopilot_request(f.session, 7, a.id, "stop-1")
                .is_none(),
            "the same id is not forwarded twice"
        );

        // Another instance cannot answer for the tab's own.
        assert!(!f
            .manager
            .finish_autopilot_request(f.session, 7, b.id, "stop-1", accepted()));
        assert!(f
            .manager
            .finish_autopilot_request(f.session, 7, a.id, "stop-1", accepted()));
        assert_eq!(
            stop.await.unwrap().outcome,
            AutopilotReceiptOutcome::Accepted
        );
        assert!(
            !f.manager
                .finish_autopilot_request(f.session, 7, a.id, "stop-1", accepted()),
            "a second receipt for one request finds nothing waiting"
        );

        // A request that is still waited on is not dropped as abandoned; one
        // whose requester gave up is.
        f.manager
            .drop_abandoned_autopilot_request(f.session, 7, "start-1");
        assert_eq!(f.manager.autopilot_waiters.len(), 1);
        drop(start);
        f.manager
            .drop_abandoned_autopilot_request(f.session, 7, "start-1");
        assert!(f.manager.autopilot_waiters.is_empty());

        // A connection that closes takes its unanswered requests with it.
        let pending = f
            .manager
            .begin_autopilot_request(f.session, 7, a.id, "pause-1")
            .unwrap();
        let elsewhere = f
            .manager
            .begin_autopilot_request(f.session, 8, b.id, "pause-2")
            .unwrap();
        f.manager.unregister_cli(&a.id);
        assert!(pending.await.is_err(), "answered as not confirmed");
        assert_eq!(f.manager.autopilot_waiters.len(), 1);
        drop(elsewhere);
    }

    /// Viewers are not told an instance's capabilities, so the server says
    /// for each OMP terminal tab whether its instance can run autopilot and
    /// what it reported, to viewers that negotiated autopilot and to no
    /// other.
    #[test]
    fn capable_viewers_are_told_each_omp_tabs_availability() {
        let f = fixture();
        f.roster(vec![
            omp_tab(3, f.m5, HOST_A),
            omp_tab(4, f.m6, HOST_B),
            tab(5, Provider::Claude, f.m5, HOST_A),
        ]);
        let mut web = f.viewer(&[TAB_AUTOPILOT_CAPABILITY]);
        let mut older_web = f.viewer(&[PROJECT_INSTANCES_CAPABILITY]);

        // Nothing runs the project: not reported, never "off".
        assert_eq!(f.support(3), Some(AutopilotSupport::NotReported));
        assert_eq!(f.support(4), Some(AutopilotSupport::NotReported));
        assert_eq!(f.support(5), None, "not an OMP terminal tab");
        assert_eq!(f.support(9), None, "not in the roster");

        // An instance attaches: every OMP tab's availability is sent.
        let a = f.instance(f.m5, HOST_A, true);
        assert_eq!(
            autopilot_frames(&mut web),
            vec![
                (3, AutopilotSupport::NotReported, None, None),
                (4, AutopilotSupport::NotReported, None, None),
            ]
        );

        // Its report is relayed, cached and replayed with the time it was
        // last reported.
        f.report(&a, 3, AutopilotSupport::Supported, 7);
        let reported = (
            3,
            AutopilotSupport::Supported,
            Some(7),
            Some("2026-11-13T14:07:00Z".to_string()),
        );
        assert_eq!(autopilot_frames(&mut web), vec![reported.clone()]);
        let replayed = f
            .manager
            .pane_autopilot_availability(&f.session, 3)
            .unwrap();
        assert_eq!(replayed.reported_at, reported.3);
        assert_eq!(replayed.state.unwrap().autopilot.unwrap().iterations, 7);

        // A machine not yet rebooted to update attaches: its OMP tab needs an
        // instance update, and the other tab's state is sent again as it is.
        let b = f.instance(f.m6, HOST_B, false);
        assert_eq!(
            autopilot_frames(&mut web),
            vec![
                reported.clone(),
                (4, AutopilotSupport::InstanceUpdateRequired, None, None),
            ]
        );
        // Whatever that connection sends, it cannot report a state.
        f.report(&b, 4, AutopilotSupport::Supported, 1);
        assert_eq!(f.support(4), Some(AutopilotSupport::InstanceUpdateRequired));
        assert!(!f.cached(4));
        assert!(autopilot_frames(&mut web).is_empty());

        // Only the server says an instance needs an update.
        f.report(&a, 3, AutopilotSupport::InstanceUpdateRequired, 8);
        assert_eq!(
            autopilot_frames(&mut web),
            vec![(
                3,
                AutopilotSupport::NotReported,
                Some(8),
                Some("2026-11-13T14:08:00Z".to_string())
            )]
        );

        // A report for a tab of another provider is dropped.
        f.report(&a, 5, AutopilotSupport::Supported, 1);
        assert!(!f.cached(5));
        assert!(autopilot_frames(&mut web).is_empty());

        // The instance leaves: its tab's state is dropped and shown as not
        // reported, while the provider may well still be running.
        f.manager.detach_cli_instance(&f.session, &a.id).unwrap();
        assert!(!f.cached(3));
        assert_eq!(
            autopilot_frames(&mut web),
            vec![
                (3, AutopilotSupport::NotReported, None, None),
                (4, AutopilotSupport::InstanceUpdateRequired, None, None),
            ]
        );

        // A viewer that did not negotiate autopilot was sent none of it.
        assert!(autopilot_frames(&mut older_web).is_empty());
    }

    /// The cached state goes wherever the tab's status goes, and with the
    /// connection that reported it.
    #[test]
    fn the_cached_state_is_pruned_with_the_tab_and_its_reporter() {
        let f = fixture();
        f.roster(vec![
            omp_tab(3, f.m5, HOST_A),
            omp_tab(4, f.m5, HOST_A),
            omp_tab(5, f.m5, HOST_A),
            omp_tab(6, f.m6, HOST_B),
        ]);
        let a = f.instance(f.m5, HOST_A, true);
        let b = f.instance(f.m6, HOST_B, true);
        for pane_id in [3, 4, 5] {
            f.report(&a, pane_id, AutopilotSupport::Supported, 1);
            f.manager.set_pane_status(
                &f.session,
                PaneType::Interactive,
                pane_id,
                Some("Working...".to_string()),
            );
        }
        f.report(&b, 6, AutopilotSupport::Supported, 1);
        let statuses = |f: &Fixture| {
            let mut panes: Vec<u32> = f
                .manager
                .get_pane_statuses(&f.session)
                .into_iter()
                .map(|(_, pane_id, _)| pane_id)
                .collect();
            panes.sort_unstable();
            panes
        };

        // Tab 3 is closed, and tab 4 becomes a Claude tab: neither has an
        // autopilot left to show. Another machine's report changes nothing.
        f.manager
            .merge_instance_roster(
                &f.session,
                &a.id,
                vec![
                    tab(4, Provider::Claude, f.m5, HOST_A),
                    omp_tab(5, f.m5, HOST_A),
                ],
            )
            .unwrap();
        f.manager
            .merge_instance_roster(&f.session, &b.id, vec![omp_tab(6, f.m6, HOST_B)])
            .unwrap();
        assert_eq!(statuses(&f), vec![4, 5], "the closed tab's status went");
        assert!(!f.cached(3) && !f.cached(4));
        assert!(f.cached(5) && f.cached(6));
        assert_eq!(f.support(4), None);

        // The same machine reconnects on a new connection: what the old one
        // reported is not the new one's, which reports again after its own
        // registration.
        let reconnected = f.cli(true);
        assert_eq!(f.attach(&reconnected, f.m5, HOST_A), vec![a.id]);
        assert!(!f.cached(5));
        assert_eq!(f.support(5), Some(AutopilotSupport::NotReported));
        assert!(f.cached(6), "another machine's report is kept");
        f.report(&reconnected, 5, AutopilotSupport::Supported, 2);
        assert_eq!(f.support(5), Some(AutopilotSupport::Supported));
        // A late frame from the replaced connection changes nothing.
        f.report(&a, 5, AutopilotSupport::NeedsAgentReboot, 9);
        assert_eq!(f.support(5), Some(AutopilotSupport::Supported));

        // A machine that left takes its reports with it, and forgetting its
        // tabs leaves nothing behind.
        f.manager.detach_cli_instance(&f.session, &b.id).unwrap();
        assert!(!f.cached(6) && f.cached(5));
        assert_eq!(
            f.manager
                .forget_machine_panes(&f.session, f.m6, Some(HOST_B))
                .unwrap(),
            vec![6]
        );
        assert_eq!(f.support(6), None);

        // Replacing the roster drops what it no longer lists.
        f.manager.set_session_panes(&f.session, Vec::new());
        assert!(!f.cached(5));
        assert!(statuses(&f).is_empty());
    }

    /// A state can arrive before the roster report that lists its tab. It is
    /// kept, and viewers hear of the tab when that report adds it.
    #[test]
    fn a_roster_report_that_adds_an_omp_tab_tells_capable_viewers() {
        let f = fixture();
        f.roster(vec![omp_tab(3, f.m5, HOST_A)]);
        let a = f.instance(f.m5, HOST_A, true);
        let b = f.instance(f.m6, HOST_B, false);
        f.report(&a, 3, AutopilotSupport::Supported, 1);
        let mut web = f.viewer(&[TAB_AUTOPILOT_CAPABILITY]);

        f.report(&a, 8, AutopilotSupport::NeedsAgentReboot, 0);
        assert!(f.cached(8));
        assert!(
            autopilot_frames(&mut web).is_empty(),
            "nothing is said about a tab viewers have not been told of"
        );

        let before = f.manager.pane_autopilot_availabilities(&f.session);
        f.manager
            .merge_instance_roster(
                &f.session,
                &a.id,
                vec![omp_tab(3, f.m5, HOST_A), omp_tab(8, f.m5, HOST_A)],
            )
            .unwrap();
        f.manager
            .broadcast_pane_autopilot_changes(&f.session, &before);
        assert_eq!(
            autopilot_frames(&mut web),
            vec![(
                8,
                AutopilotSupport::NeedsAgentReboot,
                Some(0),
                Some("2026-11-13T14:00:00Z".to_string())
            )],
            "only the added tab, with the state that arrived ahead of it"
        );

        // A tab added on a machine without autopilot is told as needing an
        // instance update; a tab of another provider is not mentioned.
        let before = f.manager.pane_autopilot_availabilities(&f.session);
        f.manager
            .merge_instance_roster(
                &f.session,
                &b.id,
                vec![
                    omp_tab(9, f.m6, HOST_B),
                    tab(10, Provider::Codex, f.m6, HOST_B),
                ],
            )
            .unwrap();
        f.manager
            .broadcast_pane_autopilot_changes(&f.session, &before);
        assert_eq!(
            autopilot_frames(&mut web),
            vec![(9, AutopilotSupport::InstanceUpdateRequired, None, None)]
        );
    }

    /// The inbound check lets any attached instance report a pane that names
    /// no machine. For autopilot only the instance that runs a listed tab
    /// speaks for it: a report from another one would replace what the tab's
    /// own instance said, and viewers would read it as not reported.
    #[test]
    fn a_listed_tabs_state_comes_only_from_the_instance_that_runs_it() {
        let f = fixture();
        // Tab 3 predates tags and is attributed to zoo-005 by host.
        let mut attributed = omp_tab(3, f.m5, HOST_A);
        attributed.machine_id = None;
        f.roster(vec![attributed, omp_tab(4, f.m6, HOST_B)]);
        let a = f.instance(f.m5, HOST_A, true);
        let b = f.instance(f.m6, HOST_B, true);
        let mut web = f.viewer(&[TAB_AUTOPILOT_CAPABILITY]);

        f.report(&a, 3, AutopilotSupport::Supported, 2);
        f.report(&b, 3, AutopilotSupport::NeedsAgentReboot, 9);
        assert_eq!(f.support(3), Some(AutopilotSupport::Supported));
        assert_eq!(
            f.manager
                .cached_pane_autopilot(&f.session, 3)
                .unwrap()
                .reporter,
            a.id
        );
        f.report(&a, 4, AutopilotSupport::Supported, 1);
        assert!(!f.cached(4), "zoo-006's tab is not zoo-005's to report");
        assert_eq!(
            autopilot_frames(&mut web),
            vec![(
                3,
                AutopilotSupport::Supported,
                Some(2),
                Some("2026-11-13T14:02:00Z".to_string())
            )],
            "only the owner's report was relayed"
        );
    }

    /// A tab that names neither a machine nor a host goes to the primary as
    /// a fallback when a pane control is routed. With several machines
    /// attached it is a tab none of them runs, so autopilot treats it as a
    /// stopped machine's: nothing the primary says about it is shown, and a
    /// request for it is not handed to the primary.
    #[test]
    fn a_tab_no_machine_runs_is_not_the_primarys_for_autopilot() {
        let f = fixture();
        let mut unowned = omp_tab(3, f.m5, HOST_A);
        unowned.machine_id = None;
        unowned.hostname = None;
        f.roster(vec![unowned]);
        let stop = || AutopilotCommand::Stop {
            autopilot_id: None,
            now: false,
        };

        // One machine runs the project: it is the only one that can run the
        // tab, as for every other control.
        let a = f.instance(f.m5, HOST_A, true);
        assert!(matches!(
            f.manager.autopilot_target(&f.session, 3),
            AutopilotTarget::Tab(PaneOwner::Instance(owner)) if owner.cli_id == a.id
        ));
        f.report(&a, 3, AutopilotSupport::Supported, 1);
        assert_eq!(f.support(3), Some(AutopilotSupport::Supported));

        let _b = f.instance(f.m6, HOST_B, true);
        assert!(matches!(
            f.manager.autopilot_target(&f.session, 3),
            AutopilotTarget::Tab(PaneOwner::NotRunning {
                machine_id: None,
                hostname: None
            })
        ));
        assert_eq!(f.support(3), Some(AutopilotSupport::NotReported));
        f.report(&a, 3, AutopilotSupport::Supported, 2);
        assert_eq!(f.support(3), Some(AutopilotSupport::NotReported));
        assert_eq!(
            f.manager
                .forward_autopilot_request(f.session, 3, a.id, "stop-1", stop())
                .unwrap_err(),
            AutopilotForwardError::Undelivered
        );
        assert!(f.manager.autopilot_waiters.is_empty());
    }

    /// A checked request is queued for the tab's instance at once or not at
    /// all. Nothing waits on a stalled connection, and nothing is sent to a
    /// connection that stopped being the tab's owner while the request was
    /// being checked, or for a tab that was closed meanwhile.
    #[test]
    fn a_request_is_handed_to_the_tabs_instance_at_once_or_not_at_all() {
        let f = fixture();
        f.roster(vec![omp_tab(3, f.m5, HOST_A), omp_tab(4, f.m6, HOST_B)]);
        // zoo-005's connection takes one frame at a time.
        let a = Uuid::new_v4();
        let (tx, mut a_rx) = mpsc::channel(1);
        f.manager.register_cli(a, f.user, tx, None);
        f.manager.set_cli_capabilities(
            a,
            vec![
                PROJECT_INSTANCES_CAPABILITY.to_string(),
                OMP_TAB_AUTOPILOT_CAPABILITY.to_string(),
            ],
        );
        assert!(matches!(
            f.manager.attach_cli_instance(
                f.session,
                a,
                Some(f.m5),
                Some(format!("/work/{HOST_A}")),
                Some(HOST_A.to_string()),
            ),
            AttachOutcome::Attached { .. }
        ));
        let b = f.instance(f.m6, HOST_B, true);
        let forward = |pane_id: u32, cli_id: Uuid, request_id: &str| {
            f.manager.forward_autopilot_request(
                f.session,
                pane_id,
                cli_id,
                request_id,
                AutopilotCommand::Stop {
                    autopilot_id: None,
                    now: true,
                },
            )
        };
        let sent = |rx: &mut mpsc::Receiver<ServerToCli>| match rx.try_recv() {
            Ok(ServerToCli::ControlPaneAutopilot {
                session_id,
                pane_id,
                request_id,
                command,
            }) => {
                assert_eq!(session_id, f.session);
                assert!(!command.starts_work());
                Some((pane_id, request_id))
            }
            Ok(other) => panic!("unexpected frame {other:?}"),
            Err(_) => None,
        };

        let _first = forward(3, a, "stop-1").expect("queued for the tab's instance");
        // The connection has not drained it: the next request is turned away
        // now, and leaves nothing waiting.
        assert_eq!(
            forward(3, a, "stop-2").unwrap_err(),
            AutopilotForwardError::Undelivered
        );
        assert_eq!(f.manager.autopilot_waiters.len(), 1);
        assert_eq!(sent(&mut a_rx), Some((3, "stop-1".to_string())));
        assert_eq!(sent(&mut a_rx), None);

        // The same id is not sent twice while it waits.
        assert_eq!(
            forward(3, a, "stop-1").unwrap_err(),
            AutopilotForwardError::AlreadyWaiting
        );
        assert_eq!(sent(&mut a_rx), None);

        // Another machine's connection is never sent this tab's request.
        assert_eq!(
            forward(3, b.id, "stop-3").unwrap_err(),
            AutopilotForwardError::Undelivered
        );
        // Nor is a connection that a reconnect of its machine replaced.
        let reconnected = f.cli(true);
        assert_eq!(f.attach(&reconnected, f.m5, HOST_A), vec![a]);
        assert_eq!(
            forward(3, a, "stop-4").unwrap_err(),
            AutopilotForwardError::Undelivered
        );
        assert_eq!(sent(&mut a_rx), None);
        // And nothing is sent for a tab that was closed meanwhile.
        f.manager
            .merge_instance_roster(&f.session, &reconnected.id, Vec::new())
            .unwrap();
        assert_eq!(
            forward(3, reconnected.id, "stop-5").unwrap_err(),
            AutopilotForwardError::Undelivered
        );
        assert_eq!(f.manager.autopilot_waiters.len(), 1, "only stop-1 waits");
    }

    /// Read-only legacy bot history is never an autopilot tab, whatever its
    /// provider and kind say.
    #[test]
    fn legacy_bot_history_is_not_an_autopilot_tab() {
        let f = fixture();
        let mut history = omp_tab(3, f.m5, HOST_A);
        history.mode = PaneMode::Deadloop;
        let mut structured = omp_tab(4, f.m5, HOST_A);
        structured.kind = PaneKind::Agent;
        assert!(!is_autopilot_tab(&history));
        assert!(!is_autopilot_tab(&structured));
        assert!(is_autopilot_tab(&omp_tab(5, f.m5, HOST_A)));

        f.roster(vec![history, structured]);
        let a = f.instance(f.m5, HOST_A, true);
        f.report(&a, 3, AutopilotSupport::Supported, 1);
        assert!(!f.cached(3));
        assert_eq!(f.support(3), None);
        assert_eq!(f.support(4), None);
    }
}
