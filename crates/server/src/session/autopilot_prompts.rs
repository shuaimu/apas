//! A project's saved autopilot prompts, as the session manager holds them:
//! the list the primary instance last reported, which viewers are told, and
//! the changes waiting for the primary's receipt.
//!
//! The prompts live in the project's `.apas`, which only CLIs write. The
//! server keeps nothing durable: the cache is rebuilt from what the primary
//! instance reports after it registers. The wire contract is
//! `shared::autopilot_prompts`.
//!
//! Every capable instance's last report is kept, by its connection, and the
//! list viewers are shown is the primary's. When the primary leaves, the next
//! primary's last report is shown at once: on separate checkouts it is a
//! different list, and its CLI sends it again only when it changes. A next
//! primary that has reported nothing (an older CLI, or one still
//! registering) leaves the departed primary's list on show rather than none:
//! on a shared home it reads the same file, and a list that disappeared from
//! viewers until the file next changed would be worse than one that may be a
//! separate checkout's until then.
//!
//! **A change goes to the primary first, and to the other instances only
//! after the primary accepted it.** Every instance checks a change again
//! under the project lock against its own `.apas`. On a shared home they all
//! write one file, so whichever instance writes second finds a saved name
//! already there, or a deleted one gone, and refuses. Sent to all at once,
//! the secondary could win the lock and the primary's receipt, the one the
//! requester is shown, would be a refusal of a change that was saved. In
//! this order the secondaries' refusals on a shared home follow a write that
//! already happened and are dropped like every secondary's receipt, and on
//! separate checkouts a change the primary refused never reaches the others.

use super::autopilot::AutopilotReceipt;
use super::SessionManager;
use shared::{
    AutopilotPromptChange, AutopilotReceiptOutcome, SavedAutopilotPrompt, ServerToCli, ServerToWeb,
    PROJECT_AUTOPILOT_PROMPTS_CAPABILITY,
};
use tokio::sync::oneshot;
use uuid::Uuid;

/// The saved prompts as an instance reported them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AutopilotPromptsView {
    pub prompts: Vec<SavedAutopilotPrompt>,
    pub default: Option<String>,
    pub unreadable: bool,
}

impl AutopilotPromptsView {
    fn frame(self, session_id: Uuid) -> ServerToWeb {
        ServerToWeb::ProjectAutopilotPrompts {
            session_id,
            prompts: self.prompts,
            default: self.default,
            unreadable: self.unreadable,
        }
    }
}

/// A change forwarded to the primary instance whose receipt has not arrived.
pub(super) struct AutopilotPromptsWaiter {
    pub(super) cli_id: Uuid,
    /// Sent on to the other instances once the primary accepted it.
    change: AutopilotPromptChange,
    sender: oneshot::Sender<AutopilotReceipt>,
}

/// Why a checked change did not reach the primary instance.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AutopilotPromptsForwardError {
    /// This same request id is already waiting in this session.
    AlreadyWaiting,
    /// The primary is no longer the instance the change was checked against,
    /// or its connection did not take the frame. Nothing was sent to it.
    Undelivered,
}

impl SessionManager {
    /// The saved prompts viewers are told, from the cached report.
    pub(crate) fn autopilot_prompts(&self, session_id: &Uuid) -> Option<AutopilotPromptsView> {
        self.sessions.get(session_id)?.autopilot_prompts.clone()
    }

    /// The frame that tells a viewer the saved prompts, as they stand when
    /// this is called. The caller sends it only to a connection that
    /// negotiated [`PROJECT_AUTOPILOT_PROMPTS_CAPABILITY`].
    pub(crate) fn autopilot_prompts_frame(&self, session_id: &Uuid) -> Option<ServerToWeb> {
        self.autopilot_prompts(session_id)
            .map(|view| view.frame(*session_id))
    }

    /// Keep what the instance `cli_id` reported and, when it is the primary,
    /// show it: tell capable viewers when its content differs from what they
    /// were told. A secondary's report is kept for a handover.
    ///
    /// A report from a connection that did not advertise the capability, or
    /// that runs no instance of the session, is ignored. Attachment is
    /// checked here under the session's lock, so a connection replaced
    /// meanwhile cannot write over the one that took its place.
    pub(crate) fn report_autopilot_prompts(
        &self,
        session_id: &Uuid,
        cli_id: Uuid,
        view: AutopilotPromptsView,
    ) {
        if !self.cli_supports_capability(&cli_id, PROJECT_AUTOPILOT_PROMPTS_CAPABILITY) {
            tracing::debug!(%session_id, %cli_id, "ignoring saved prompts from a CLI that did not advertise them");
            return;
        }
        let changed = {
            let Some(mut session) = self.sessions.get_mut(session_id) else {
                return;
            };
            let Some(index) = session
                .instances
                .iter()
                .position(|instance| instance.cli_id == cli_id)
            else {
                tracing::debug!(%session_id, %cli_id, "ignoring saved prompts from a connection that runs no instance");
                return;
            };
            session
                .autopilot_prompts_reports
                .insert(cli_id, view.clone());
            if index != 0 {
                return;
            }
            let changed = session.autopilot_prompts.as_ref() != Some(&view);
            session.autopilot_prompts = Some(view);
            changed
        };
        if changed {
            self.broadcast_autopilot_prompts(session_id);
        }
    }

    /// The session's viewers that negotiated
    /// [`PROJECT_AUTOPILOT_PROMPTS_CAPABILITY`]. Older native builds drop
    /// their socket on a frame type their schema does not list, so these
    /// frames go to such connections one by one, never through
    /// `route_to_web`.
    fn autopilot_prompts_viewers(&self, session_id: &Uuid) -> Vec<Uuid> {
        let viewers: Vec<Uuid> = self
            .sessions
            .get(session_id)
            .map(|session| session.web_connection_ids.clone())
            .unwrap_or_default();
        viewers
            .into_iter()
            .filter(|connection_id| {
                self.web_supports_capability(connection_id, PROJECT_AUTOPILOT_PROMPTS_CAPABILITY)
            })
            .collect()
    }

    /// Tell capable viewers the saved prompts as the cache holds them.
    pub(crate) fn broadcast_autopilot_prompts(&self, session_id: &Uuid) {
        let viewers = self.autopilot_prompts_viewers(session_id);
        if viewers.is_empty() {
            return;
        }
        let Some(frame) = self.autopilot_prompts_frame(session_id) else {
            return;
        };
        for connection_id in viewers {
            let sender = self
                .web_senders
                .get(&connection_id)
                .map(|sender| sender.clone());
            if let Some(sender) = sender {
                let _ = sender.try_send(frame.clone());
            }
        }
    }

    /// Send a checked change to `primary_cli`, the primary instance it was
    /// checked against, and wait for its receipt. The other instances are
    /// sent it only once the primary accepted it
    /// ([`Self::finish_autopilot_prompts_request`]); the module comment says
    /// why.
    ///
    /// The checks awaited the database, so the primary is read once more
    /// here: a change never goes out when another instance has become
    /// primary meanwhile. The frame is queued without waiting, so a stalled
    /// instance cannot hold up the requester's other frames.
    pub(crate) fn forward_autopilot_prompts_change(
        &self,
        session_id: Uuid,
        primary_cli: Uuid,
        request_id: &str,
        change: &AutopilotPromptChange,
    ) -> Result<oneshot::Receiver<AutopilotReceipt>, AutopilotPromptsForwardError> {
        // The sender is cloned outside the session's lock, so the sender map
        // is never locked inside it.
        let primary_sender = self
            .cli_senders
            .get(&primary_cli)
            .map(|sender| sender.clone())
            .ok_or(AutopilotPromptsForwardError::Undelivered)?;
        {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or(AutopilotPromptsForwardError::Undelivered)?;
            if session.instances.first().map(|primary| primary.cli_id) != Some(primary_cli) {
                return Err(AutopilotPromptsForwardError::Undelivered);
            }
        }
        let receiver = {
            use dashmap::mapref::entry::Entry;
            let Entry::Vacant(entry) = self
                .autopilot_prompts_waiters
                .entry((session_id, request_id.to_string()))
            else {
                return Err(AutopilotPromptsForwardError::AlreadyWaiting);
            };
            let (sender, receiver) = oneshot::channel();
            entry.insert(AutopilotPromptsWaiter {
                cli_id: primary_cli,
                change: change.clone(),
                sender,
            });
            receiver
        };
        let frame = ServerToCli::UpdateAutopilotPrompts {
            session_id,
            request_id: request_id.to_string(),
            change: change.clone(),
        };
        if primary_sender.try_send(frame).is_err() {
            drop(receiver);
            self.drop_abandoned_autopilot_prompts_request(session_id, request_id);
            return Err(AutopilotPromptsForwardError::Undelivered);
        }
        Ok(receiver)
    }

    /// Hand the primary's receipt to the change waiting for it. Only the
    /// connection the change was checked against can answer it.
    ///
    /// An accepted change is then sent to every other attached instance that
    /// can keep saved prompts, whether or not the requester still waits: the
    /// primary has written it. Their receipts are not the answer and are
    /// dropped. An instance without the capability is sent nothing, because
    /// an older CLI drops the frame without an answer. A change refused, or
    /// not answered before the requester stopped waiting, goes nowhere else.
    pub(crate) fn finish_autopilot_prompts_request(
        &self,
        session_id: Uuid,
        cli_id: Uuid,
        request_id: &str,
        receipt: AutopilotReceipt,
    ) -> bool {
        let Some((_, waiter)) = self
            .autopilot_prompts_waiters
            .remove_if(&(session_id, request_id.to_string()), |_, waiter| {
                waiter.cli_id == cli_id
            })
        else {
            return false;
        };
        if receipt.outcome == AutopilotReceiptOutcome::Accepted {
            self.forward_accepted_autopilot_prompts_change(
                session_id,
                cli_id,
                request_id,
                &waiter.change,
            );
        }
        waiter.sender.send(receipt).is_ok()
    }

    /// Send a change the primary `primary_cli` accepted to the session's
    /// other capable instances, without waiting.
    fn forward_accepted_autopilot_prompts_change(
        &self,
        session_id: Uuid,
        primary_cli: Uuid,
        request_id: &str,
        change: &AutopilotPromptChange,
    ) {
        // Read under the session's lock, sent outside it.
        let others: Vec<Uuid> = self
            .sessions
            .get(&session_id)
            .map(|session| {
                session
                    .instances
                    .iter()
                    .map(|instance| instance.cli_id)
                    .filter(|instance| *instance != primary_cli)
                    .collect()
            })
            .unwrap_or_default();
        let frame = ServerToCli::UpdateAutopilotPrompts {
            session_id,
            request_id: request_id.to_string(),
            change: change.clone(),
        };
        for cli_id in others {
            if !self.cli_supports_capability(&cli_id, PROJECT_AUTOPILOT_PROMPTS_CAPABILITY) {
                continue;
            }
            let sender = self.cli_senders.get(&cli_id).map(|sender| sender.clone());
            match sender {
                Some(sender) if sender.try_send(frame.clone()).is_ok() => {}
                _ => {
                    tracing::debug!(%session_id, %cli_id, "a secondary instance did not take a saved-prompts change")
                }
            }
        }
    }

    /// Drop the waiter of a change nobody waits for any more. One that is
    /// still being waited on is left alone.
    pub(crate) fn drop_abandoned_autopilot_prompts_request(
        &self,
        session_id: Uuid,
        request_id: &str,
    ) {
        self.autopilot_prompts_waiters
            .remove_if(&(session_id, request_id.to_string()), |_, waiter| {
                waiter.sender.is_closed()
            });
    }
}
