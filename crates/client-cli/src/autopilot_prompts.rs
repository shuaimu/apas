//! A project's saved autopilot prompts, as this CLI keeps them in `.apas`
//! and reports them to the server.
//!
//! The list is two optional top-level fields of `.apas`,
//! `autopilot_prompts` and `default_autopilot_prompt`, written only through
//! the locked merge (`project_instances::update_project_settings`), so every
//! other field and every other machine's panes stay as the file has them.
//! The wire contract is `shared::autopilot_prompts`; the design record is
//! `openspec/changes/add-autopilot-prompt-library/`.
//!
//! **A list that does not parse must not break `.apas`.** A typed
//! `Vec<SavedAutopilotPrompt>` field would make the whole file fail to parse,
//! and with it every pane save of every machine. Both fields are therefore
//! [`Lenient`]: a value of the wrong shape is kept as raw JSON, reported as
//! unreadable, and written back unchanged, so the file is never rewritten
//! because of it.
//!
//! Saving a prompt sends nothing to any tab and never starts, pauses or stops
//! an autopilot: nothing here touches a pane or its runtime directory.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use shared::{
    find_saved_autopilot_prompt, validate_autopilot_prompt_change, AutopilotPromptChange,
    AutopilotReceiptOutcome, CliToServer, SavedAutopilotPrompt,
    AUTOPILOT_PROMPTS_UNREADABLE_REASON, PROJECT_AUTOPILOT_PROMPTS_CAPABILITY,
};
use std::sync::Mutex;
use uuid::Uuid;

/// One optional `.apas` field that must never stop the file from parsing.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Lenient<T> {
    /// The field is not there, or is `null`.
    #[default]
    Absent,
    Valid(T),
    /// The field is there with a value of the wrong shape. Kept as read, and
    /// serialized back as it was, so an unchanged write leaves it alone.
    Malformed(serde_json::Value),
}

impl<T> Lenient<T> {
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

impl<T: Serialize> Serialize for Lenient<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Absent => serializer.serialize_none(),
            Self::Valid(value) => value.serialize(serializer),
            Self::Malformed(raw) => raw.serialize(serializer),
        }
    }
}

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        if raw.is_null() {
            return Ok(Self::Absent);
        }
        Ok(match serde_json::from_value::<T>(raw.clone()) {
            Ok(value) => Self::Valid(value),
            Err(_) => Self::Malformed(raw),
        })
    }
}

/// The saved prompts as the server is told them: the payload of
/// `CliToServer::ProjectAutopilotPrompts`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptsView {
    pub prompts: Vec<SavedAutopilotPrompt>,
    pub default: Option<String>,
    pub unreadable: bool,
}

impl PromptsView {
    /// What `.apas` holds. Either field malformed makes the whole list
    /// unreadable: a default that cannot be read would otherwise be dropped
    /// by the next save.
    pub fn of(prompts: &Lenient<Vec<SavedAutopilotPrompt>>, default: &Lenient<String>) -> Self {
        let unreadable =
            matches!(prompts, Lenient::Malformed(_)) || matches!(default, Lenient::Malformed(_));
        if unreadable {
            return Self {
                prompts: Vec::new(),
                default: None,
                unreadable: true,
            };
        }
        Self {
            prompts: match prompts {
                Lenient::Valid(prompts) => prompts.clone(),
                _ => Vec::new(),
            },
            default: match default {
                Lenient::Valid(name) => Some(name.clone()),
                _ => None,
            },
            unreadable: false,
        }
    }

    pub fn of_metadata(metadata: &crate::project::ProjectMetadata) -> Self {
        Self::of(
            &metadata.autopilot_prompts,
            &metadata.default_autopilot_prompt,
        )
    }

    pub fn frame(&self, session_id: Uuid) -> CliToServer {
        CliToServer::ProjectAutopilotPrompts {
            session_id,
            prompts: self.prompts.clone(),
            default: self.default.clone(),
            unreadable: self.unreadable,
        }
    }
}

/// The time a save records, as RFC 3339 in UTC.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Apply `change` to the project-level fields `metadata` holds, or say why it
/// is refused, in which case nothing in `metadata` changed. Runs under the
/// project lock on a fresh read, so the check is against the file as it is,
/// not as the server last heard it.
///
/// A saved name is stored trimmed. A replacement keeps the entry's place in
/// the list. Deleting the default clears the default, and the default is
/// stored under the saved entry's own spelling.
pub fn apply_change(
    metadata: &mut crate::project::ProjectMetadata,
    change: &AutopilotPromptChange,
    now: &str,
) -> Result<(), String> {
    let view = PromptsView::of_metadata(metadata);
    if view.unreadable {
        return Err(AUTOPILOT_PROMPTS_UNREADABLE_REASON.to_string());
    }
    validate_autopilot_prompt_change(change, &view.prompts, view.default.as_deref())?;
    let mut prompts = view.prompts;
    let mut default = view.default;
    let same = |a: &str, b: &str| a.trim().to_lowercase() == b.trim().to_lowercase();
    match change {
        AutopilotPromptChange::Save { name, prompt, .. } => {
            let saved = SavedAutopilotPrompt {
                name: name.trim().to_string(),
                prompt: prompt.clone(),
                updated_at: now.to_string(),
            };
            match find_saved_autopilot_prompt(&prompts, name) {
                Some(index) => {
                    // The default follows a replacement that changes the
                    // name's case, so it never names an entry that is gone.
                    if default
                        .as_deref()
                        .is_some_and(|current| same(current, &prompts[index].name))
                    {
                        default = Some(saved.name.clone());
                    }
                    prompts[index] = saved;
                }
                None => prompts.push(saved),
            }
        }
        AutopilotPromptChange::Delete { name } => {
            if let Some(index) = find_saved_autopilot_prompt(&prompts, name) {
                let removed = prompts.remove(index);
                if default
                    .as_deref()
                    .is_some_and(|current| same(current, &removed.name))
                {
                    default = None;
                }
            }
        }
        AutopilotPromptChange::SetDefault { name: Some(name) } => {
            if let Some(index) = find_saved_autopilot_prompt(&prompts, name) {
                default = Some(prompts[index].name.clone());
            }
        }
        AutopilotPromptChange::SetDefault { name: None } => default = None,
    }
    // An empty list and no default leave no trace in the file.
    metadata.autopilot_prompts = if prompts.is_empty() {
        Lenient::Absent
    } else {
        Lenient::Valid(prompts)
    };
    metadata.default_autopilot_prompt = match default {
        Some(name) => Lenient::Valid(name),
        None => Lenient::Absent,
    };
    Ok(())
}

/// The receipt for one request: `Accepted` when the change is written,
/// `Refused` with the reason otherwise. `written` is what the locked write
/// returned: its outer error is a failure to take the lock, read or write
/// the file, in which case `.apas` was not changed.
pub fn receipt(
    session_id: Uuid,
    request_id: String,
    written: anyhow::Result<Result<(), String>>,
) -> CliToServer {
    let (outcome, reason) = match written {
        Ok(Ok(())) => (AutopilotReceiptOutcome::Accepted, None),
        Ok(Err(reason)) => (AutopilotReceiptOutcome::Refused, Some(reason)),
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "could not save the project's autopilot prompts");
            (
                AutopilotReceiptOutcome::Refused,
                Some(format!(
                    "The prompts could not be saved to .apas: {error:#}"
                )),
            )
        }
    };
    CliToServer::AutopilotPromptsReceipt {
        session_id,
        request_id,
        outcome,
        reason,
    }
}

/// What this runtime last told the server about the saved prompts, and
/// whether the server of the current connection takes the report at all.
///
/// Every list frame is queued under this one lock, onto one queue, but the
/// file is read before the lock is taken, and reads race: the 5-second poll
/// can read the list just before a change writes it, and then report that
/// older copy after the change reported the new one. The server would show
/// it, and check changes against it, until the next poll. So each read takes
/// a [`ReadTicket`] before it reads, and a report whose ticket is older than
/// one already queued is dropped. A change's report, and the one after a
/// registration, take a fresh ticket ([`Self::fresh_ticket`]); the poll's
/// report is also dropped while a fresh read that began after its own is on
/// its way, since that one reports by itself. With that, the last frame the
/// server receives is the newest read, which is the one recorded here.
#[derive(Debug, Default)]
pub struct PromptsReporter {
    state: Mutex<ReporterState>,
}

/// When a read of the saved prompts began, relative to the others: taken
/// before the read, handed to [`PromptsReporter::report`] with what it read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReadTicket(u64);

#[derive(Debug, Default)]
struct ReporterState {
    /// The server of the current registration echoed
    /// [`PROJECT_AUTOPILOT_PROMPTS_CAPABILITY`]. An older one cannot parse
    /// the frame and would log every copy.
    negotiated: bool,
    /// The list last queued for the server since the current registration.
    last_sent: Option<PromptsView>,
    /// The newest ticket a fresh read was given. Never goes back, also not
    /// across registrations, so a read begun before one cannot pass for a
    /// later one.
    generation: u64,
    /// The ticket of the read last queued.
    sent: u64,
}

impl PromptsReporter {
    pub fn new() -> Self {
        Self::default()
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ReporterState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A registration completed, with `negotiated` echoed. Nothing has been
    /// sent on the new connection yet.
    pub fn registered(&self, negotiated: &[String]) {
        let mut state = self.locked();
        state.negotiated = negotiated
            .iter()
            .any(|capability| capability == PROJECT_AUTOPILOT_PROMPTS_CAPABILITY);
        state.last_sent = None;
        // A periodic read begun on the old connection is not reported on
        // this one; the registration's own report follows.
        state.generation += 1;
    }

    /// The connection is gone. Nothing is reported until the next
    /// registration.
    pub fn disconnected(&self) {
        let mut state = self.locked();
        state.negotiated = false;
        state.last_sent = None;
    }

    /// The ticket of a periodic read, taken before it reads the file.
    pub fn ticket(&self) -> ReadTicket {
        ReadTicket(self.locked().generation)
    }

    /// The ticket of a read whose report must go out, after a change or a
    /// registration, taken before it reads the file. A periodic read that
    /// began earlier is no longer reported.
    pub fn fresh_ticket(&self) -> ReadTicket {
        let mut state = self.locked();
        state.generation += 1;
        ReadTicket(state.generation)
    }

    /// Queue `view`, read under `ticket`, with `queue` when the server takes
    /// it. Never when a read that began later was already queued, and,
    /// unless `always`, neither while a fresh read that began later is on
    /// its way nor when it equals what was last queued. A queue that is full
    /// or closed takes nothing, and the next read tries again.
    pub fn report(
        &self,
        session_id: Uuid,
        view: PromptsView,
        ticket: ReadTicket,
        always: bool,
        queue: impl FnOnce(CliToServer) -> bool,
    ) -> bool {
        let mut state = self.locked();
        if !state.negotiated || ticket.0 < state.sent {
            return false;
        }
        if !always && (ticket.0 < state.generation || state.last_sent.as_ref() == Some(&view)) {
            return false;
        }
        if queue(view.frame(session_id)) {
            state.last_sent = Some(view);
            state.sent = ticket.0;
            true
        } else {
            state.last_sent = None;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectMetadata;
    use serde_json::json;

    const NOW: &str = "2026-11-14T10:00:00Z";

    fn saved(name: &str, prompt: &str) -> SavedAutopilotPrompt {
        SavedAutopilotPrompt {
            name: name.into(),
            prompt: prompt.into(),
            updated_at: "2026-11-13T09:00:00Z".into(),
        }
    }

    fn with(prompts: Vec<SavedAutopilotPrompt>, default: Option<&str>) -> ProjectMetadata {
        let mut metadata = ProjectMetadata::new();
        metadata.autopilot_prompts = Lenient::Valid(prompts);
        metadata.default_autopilot_prompt = match default {
            Some(name) => Lenient::Valid(name.to_string()),
            None => Lenient::Absent,
        };
        metadata
    }

    fn save(name: &str, prompt: &str, replace: bool) -> AutopilotPromptChange {
        AutopilotPromptChange::Save {
            name: name.into(),
            prompt: prompt.into(),
            replace,
        }
    }

    #[test]
    fn a_metadata_file_with_a_malformed_list_still_parses_and_keeps_it() {
        let raw = json!({
            "id": Uuid::nil(),
            "name": "p",
            "created_at": "2026-01-01T00:00:00Z",
            "autopilot_prompts": {"oops": true},
            "default_autopilot_prompt": 7
        });
        let metadata: ProjectMetadata = serde_json::from_value(raw).unwrap();
        assert_eq!(
            metadata.autopilot_prompts,
            Lenient::Malformed(json!({"oops": true}))
        );
        assert!(PromptsView::of_metadata(&metadata).unreadable);
        let back = serde_json::to_value(&metadata).unwrap();
        assert_eq!(back["autopilot_prompts"], json!({"oops": true}));
        assert_eq!(back["default_autopilot_prompt"], json!(7));

        // An entry without its time is as malformed as a list that is not one.
        let raw = json!({
            "id": Uuid::nil(), "name": "p", "created_at": "x",
            "autopilot_prompts": [{"name": "a", "prompt": "b"}]
        });
        let metadata: ProjectMetadata = serde_json::from_value(raw).unwrap();
        assert!(PromptsView::of_metadata(&metadata).unreadable);
    }

    #[test]
    fn absent_and_null_fields_are_no_prompts_and_are_not_written() {
        let raw = json!({"id": Uuid::nil(), "name": "p", "created_at": "x",
            "autopilot_prompts": null});
        let metadata: ProjectMetadata = serde_json::from_value(raw).unwrap();
        assert_eq!(PromptsView::of_metadata(&metadata), PromptsView::default());
        let back = serde_json::to_value(&metadata).unwrap();
        assert!(back.get("autopilot_prompts").is_none());
        assert!(back.get("default_autopilot_prompt").is_none());
    }

    #[test]
    fn a_save_adds_a_trimmed_name_with_the_time_of_the_save() {
        let mut metadata = with(vec![saved("Nightly", "Old.")], None);
        apply_change(
            &mut metadata,
            &save("  Release check ", "Check.", false),
            NOW,
        )
        .unwrap();
        let view = PromptsView::of_metadata(&metadata);
        assert_eq!(
            view.prompts,
            vec![
                saved("Nightly", "Old."),
                SavedAutopilotPrompt {
                    name: "Release check".into(),
                    prompt: "Check.".into(),
                    updated_at: NOW.into(),
                }
            ]
        );
    }

    #[test]
    fn an_existing_name_is_replaced_in_place_only_when_asked() {
        let mut metadata = with(
            vec![saved("A", "a"), saved("Nightly", "Old."), saved("B", "b")],
            Some("Nightly"),
        );
        let before = metadata.clone();
        assert_eq!(
            apply_change(&mut metadata, &save("nightly", "New.", false), NOW),
            Err("A saved prompt named \"Nightly\" already exists.".to_string())
        );
        assert_eq!(
            PromptsView::of_metadata(&metadata),
            PromptsView::of_metadata(&before)
        );
        apply_change(&mut metadata, &save("NIGHTLY", "New.", true), NOW).unwrap();
        let view = PromptsView::of_metadata(&metadata);
        assert_eq!(view.prompts[1].name, "NIGHTLY");
        assert_eq!(view.prompts[1].prompt, "New.");
        assert_eq!(view.prompts[1].updated_at, NOW);
        assert_eq!(view.prompts.len(), 3);
        // The default follows the entry under its new spelling.
        assert_eq!(view.default.as_deref(), Some("NIGHTLY"));
    }

    #[test]
    fn deleting_the_default_clears_it_and_another_delete_keeps_it() {
        let mut metadata = with(vec![saved("A", "a"), saved("B", "b")], Some("B"));
        apply_change(
            &mut metadata,
            &AutopilotPromptChange::Delete { name: "a".into() },
            NOW,
        )
        .unwrap();
        assert_eq!(
            PromptsView::of_metadata(&metadata).default.as_deref(),
            Some("B")
        );
        apply_change(
            &mut metadata,
            &AutopilotPromptChange::Delete { name: " b ".into() },
            NOW,
        )
        .unwrap();
        assert_eq!(PromptsView::of_metadata(&metadata), PromptsView::default());
        assert!(metadata.autopilot_prompts.is_absent());
        assert!(metadata.default_autopilot_prompt.is_absent());
    }

    #[test]
    fn the_default_is_set_under_the_saved_spelling_and_cleared() {
        let mut metadata = with(vec![saved("Nightly review", "a")], None);
        apply_change(
            &mut metadata,
            &AutopilotPromptChange::SetDefault {
                name: Some("nightly REVIEW".into()),
            },
            NOW,
        )
        .unwrap();
        assert_eq!(
            PromptsView::of_metadata(&metadata).default.as_deref(),
            Some("Nightly review")
        );
        assert_eq!(
            apply_change(
                &mut metadata,
                &AutopilotPromptChange::SetDefault {
                    name: Some("Weekly".into())
                },
                NOW,
            ),
            Err("No saved prompt is named \"Weekly\".".to_string())
        );
        apply_change(
            &mut metadata,
            &AutopilotPromptChange::SetDefault { name: None },
            NOW,
        )
        .unwrap();
        assert_eq!(PromptsView::of_metadata(&metadata).default, None);
    }

    #[test]
    fn an_unreadable_list_refuses_every_change_and_changes_nothing() {
        let mut metadata = ProjectMetadata::new();
        metadata.autopilot_prompts = Lenient::Malformed(json!("not a list"));
        let before = serde_json::to_value(&metadata).unwrap();
        for change in [
            save("New", "x", false),
            AutopilotPromptChange::Delete { name: "x".into() },
            AutopilotPromptChange::SetDefault { name: None },
        ] {
            assert_eq!(
                apply_change(&mut metadata, &change, NOW),
                Err(AUTOPILOT_PROMPTS_UNREADABLE_REASON.to_string())
            );
        }
        assert_eq!(serde_json::to_value(&metadata).unwrap(), before);
    }

    #[test]
    fn the_receipt_names_what_happened() {
        let session = Uuid::new_v4();
        let outcome = |written| match receipt(session, "r".into(), written) {
            CliToServer::AutopilotPromptsReceipt {
                outcome, reason, ..
            } => (outcome, reason),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            outcome(Ok(Ok(()))),
            (AutopilotReceiptOutcome::Accepted, None)
        );
        assert_eq!(
            outcome(Ok(Err("No.".into()))),
            (AutopilotReceiptOutcome::Refused, Some("No.".into()))
        );
        let (refused, reason) = outcome(Err(anyhow::anyhow!("lock timed out")));
        assert_eq!(refused, AutopilotReceiptOutcome::Refused);
        assert!(reason.unwrap().contains("lock timed out"));
    }

    #[test]
    fn the_list_is_reported_after_registration_and_then_only_on_a_change() {
        let session = Uuid::new_v4();
        let reporter = PromptsReporter::new();
        let sent = std::cell::RefCell::new(Vec::new());
        let one = PromptsView {
            prompts: vec![saved("A", "a")],
            default: None,
            unreadable: false,
        };
        let mut queue = |frame: CliToServer| {
            sent.borrow_mut().push(frame);
            true
        };

        // Not registered, or registered with a server that did not echo the
        // capability: nothing at all.
        assert!(!reporter.report(
            session,
            one.clone(),
            reporter.fresh_ticket(),
            true,
            &mut queue
        ));
        reporter.registered(&[shared::OMP_TAB_AUTOPILOT_CAPABILITY.to_string()]);
        assert!(!reporter.report(
            session,
            one.clone(),
            reporter.fresh_ticket(),
            true,
            &mut queue
        ));
        assert!(sent.borrow().is_empty());

        reporter.registered(&[PROJECT_AUTOPILOT_PROMPTS_CAPABILITY.to_string()]);
        // After registration, the first report goes out.
        assert!(reporter.report(
            session,
            one.clone(),
            reporter.fresh_ticket(),
            true,
            &mut queue
        ));
        // The periodic read of the same content sends nothing.
        for _ in 0..3 {
            assert!(!reporter.report(session, one.clone(), reporter.ticket(), false, &mut queue));
        }
        let two = PromptsView {
            default: Some("A".into()),
            ..one.clone()
        };
        assert!(reporter.report(session, two.clone(), reporter.ticket(), false, &mut queue));
        assert!(!reporter.report(session, two.clone(), reporter.ticket(), false, &mut queue));
        // After an applied change the list follows the receipt even when the
        // poller already sent it.
        assert!(reporter.report(
            session,
            two.clone(),
            reporter.fresh_ticket(),
            true,
            &mut queue
        ));
        assert_eq!(sent.borrow().len(), 3);

        // A reconnect sends it again, once.
        reporter.disconnected();
        assert!(!reporter.report(session, two.clone(), reporter.ticket(), false, &mut queue));
        reporter.registered(&[PROJECT_AUTOPILOT_PROMPTS_CAPABILITY.to_string()]);
        assert!(reporter.report(session, two.clone(), reporter.ticket(), false, &mut queue));
        assert!(!reporter.report(session, two.clone(), reporter.ticket(), false, &mut queue));
        assert_eq!(sent.borrow().len(), 4);
        assert!(matches!(
            &sent.borrow()[3],
            CliToServer::ProjectAutopilotPrompts { default: Some(name), unreadable: false, .. }
                if name == "A"
        ));

        // A full queue takes nothing, and the next read tries again.
        let mut full = |_frame: CliToServer| false;
        reporter.registered(&[PROJECT_AUTOPILOT_PROMPTS_CAPABILITY.to_string()]);
        assert!(!reporter.report(session, one.clone(), reporter.ticket(), false, &mut full));
        assert!(reporter.report(session, one.clone(), reporter.ticket(), false, &mut queue));
    }

    /// A periodic read that began before a change wrote the file is never
    /// reported after the change's own report, and neither is a change's
    /// read that began before another change's that was already reported.
    #[test]
    fn an_older_read_is_never_reported_after_a_newer_one() {
        let session = Uuid::new_v4();
        let reporter = PromptsReporter::new();
        let sent = std::cell::RefCell::new(Vec::new());
        let mut queue = |frame: CliToServer| {
            sent.borrow_mut().push(frame);
            true
        };
        let view = |names: &[&str]| PromptsView {
            prompts: names.iter().map(|name| saved(name, "x")).collect(),
            default: None,
            unreadable: false,
        };
        reporter.registered(&[PROJECT_AUTOPILOT_PROMPTS_CAPABILITY.to_string()]);
        assert!(reporter.report(
            session,
            view(&[]),
            reporter.fresh_ticket(),
            true,
            &mut queue
        ));

        // The poll reads the file without "Nightly"; a change then writes it,
        // reads it back and reports it; the poll's report comes last.
        let poll = reporter.ticket();
        let change = reporter.fresh_ticket();
        assert!(reporter.report(session, view(&["Nightly"]), change, true, &mut queue));
        assert!(!reporter.report(session, view(&[]), poll, false, &mut queue));
        // The poll reports at all only while no fresher read is on its way.
        let poll = reporter.ticket();
        let in_flight = reporter.fresh_ticket();
        assert!(!reporter.report(
            session,
            view(&["Nightly", "Other"]),
            poll,
            false,
            &mut queue
        ));
        assert!(reporter.report(
            session,
            view(&["Nightly", "Other"]),
            in_flight,
            true,
            &mut queue
        ));

        // Two changes whose reports cross: the older read is dropped.
        let first = reporter.fresh_ticket();
        let second = reporter.fresh_ticket();
        assert!(reporter.report(
            session,
            view(&["Nightly", "Other", "Third"]),
            second,
            true,
            &mut queue
        ));
        assert!(!reporter.report(
            session,
            view(&["Nightly", "Other"]),
            first,
            true,
            &mut queue
        ));

        // A read that began before a reconnect is not reported after it.
        let stale = reporter.ticket();
        reporter.disconnected();
        reporter.registered(&[PROJECT_AUTOPILOT_PROMPTS_CAPABILITY.to_string()]);
        let registration = reporter.fresh_ticket();
        assert!(!reporter.report(session, view(&["Old"]), stale, false, &mut queue));
        assert!(reporter.report(session, view(&["New"]), registration, true, &mut queue));

        let last = |frame: &CliToServer| match frame {
            CliToServer::ProjectAutopilotPrompts { prompts, .. } => prompts
                .iter()
                .map(|saved| saved.name.clone())
                .collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };
        let sent = sent.borrow();
        assert_eq!(sent.len(), 5);
        assert_eq!(last(&sent[1]), vec!["Nightly".to_string()]);
        assert_eq!(last(&sent[3]).len(), 3);
        assert_eq!(last(&sent[4]), vec!["New".to_string()]);
    }
}
