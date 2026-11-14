//! Autopilot for interactive OMP terminal tabs: the wire contract.
//!
//! A tab on autopilot submits one prompt again each time an iteration is due
//! and the tab has settled. The loop, its limits and its state live in the
//! APAS extension inside the tab's OMP process. This module is what the
//! viewer, the server and the CLI say to each other about it. The design
//! record is `openspec/changes/archive/2026-11-13-add-omp-tab-autopilot/`.
//!
//! Autopilot is not a worker and shares none of the worker's types. It is not
//! contained, verified or budgeted.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The project instance (CLI) can relay autopilot requests to its OMP tabs
/// and report their state.
///
/// The server checks it on the instance that owns a tab before it forwards a
/// request, because an older CLI drops an unknown frame without an answer.
/// The server echoes it in `ServerToCli::Registered.negotiated_capabilities`;
/// a CLI that does not see it echoed tells its tabs that the server cannot
/// control autopilot, and they hold.
pub const OMP_TAB_AUTOPILOT_CAPABILITY: &str = "omp_tab_autopilot_v1";
/// A viewer understands the autopilot frames. The server sends
/// `ServerToWeb::PaneAutopilotState` and `PaneAutopilotReceipt` only to
/// connections that negotiated it: older native builds drop their socket on a
/// frame type their bundled schema lacks.
pub const TAB_AUTOPILOT_CAPABILITY: &str = "tab_autopilot_v1";

/// Reserved tool name of the assistant tool-use block that marks where an
/// iteration starts in a tab's conversation. Its input is
/// [`AutopilotIterationMarker`].
pub const AUTOPILOT_ITERATION_TOOL_NAME: &str = "ApasAutopilotIteration";

pub const AUTOPILOT_PROMPT_MAX_BYTES: usize = 16_000;
pub const AUTOPILOT_INTERVAL_MIN_SECS: u64 = 60;
pub const AUTOPILOT_INTERVAL_MAX_SECS: u64 = 24 * 60 * 60;
pub const AUTOPILOT_TIME_LIMIT_MIN_SECS: u64 = 60;
/// Not policy: it refuses a mistyped number. Someone who wants more chooses
/// no limit.
pub const AUTOPILOT_TIME_LIMIT_MAX_SECS: u64 = 365 * 24 * 60 * 60;
/// Not policy either; see [`AUTOPILOT_TIME_LIMIT_MAX_SECS`].
pub const AUTOPILOT_ITERATION_LIMIT_MAX: u64 = 1_000_000;
/// Longest request id a viewer may send.
pub const AUTOPILOT_REQUEST_ID_MAX_BYTES: usize = 128;

/// When the next iteration is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutopilotRepeat {
    /// Five seconds after the tab settles.
    AfterEachTurn,
    /// This long after the previous iteration was submitted, and never
    /// sooner than five seconds after the tab settles.
    Every { seconds: u64 },
}

/// One limit, as an explicit value. "No limit" is a value of its own and is
/// never expressed by leaving the limit out: a field that one version drops
/// must not turn a limited autopilot into an unlimited one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutopilotLimit {
    None,
    /// Iterations for an iteration limit, seconds for a time limit.
    AtMost { value: u64 },
}

impl AutopilotLimit {
    pub fn value(self) -> Option<u64> {
        match self {
            Self::None => None,
            Self::AtMost { value } => Some(value),
        }
    }
}

/// What a viewer asks for in a start. Every field is optional on purpose: on
/// this wire a frame that fails to parse is dropped without an answer, so the
/// request has to parse for the server to refuse it with a reason.
/// [`AutopilotStartRequest::validate`] turns it into the strict
/// [`AutopilotSpec`] or says what is wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutopilotStartRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<AutopilotRepeat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<AutopilotLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_limit: Option<AutopilotLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_without_tool_call: Option<bool>,
}

/// One autopilot as it runs: a prompt, a timing and optional limits. This is
/// the strict form the server forwards and the tab reports back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutopilotSpec {
    /// Submitted as typed text each iteration. Slash commands, skills and
    /// prompt templates in it are not run.
    pub prompt: String,
    pub repeat: AutopilotRepeat,
    /// Most iterations it may submit; the first counts.
    pub iteration_limit: AutopilotLimit,
    /// Seconds from the start after which autopilot ends. Paused, waiting and
    /// holding time counts.
    pub time_limit: AutopilotLimit,
    /// End after an iteration whose turn completes without a tool call.
    pub end_without_tool_call: bool,
}

/// Why a start was refused. The text is shown to the person who sent it.
pub type AutopilotRefusal = String;

fn is_refused_control(character: char) -> bool {
    // U+0000..U+001F except newline and tab, U+007F, and U+0080..U+009F.
    let code = character as u32;
    (code <= 0x1f && character != '\n' && character != '\t') || (0x7f..=0x9f).contains(&code)
}

/// The bounds on a prompt's text alone: what a start checks of its prompt,
/// and what a prompt saved to a project must meet so that it can start one
/// (see [`crate::validate_autopilot_prompt_change`]).
pub fn validate_autopilot_prompt(prompt: &str) -> Result<(), AutopilotRefusal> {
    if prompt.trim().is_empty() {
        return Err("Autopilot needs a prompt.".into());
    }
    if prompt.len() > AUTOPILOT_PROMPT_MAX_BYTES {
        return Err(format!(
            "The prompt is longer than {AUTOPILOT_PROMPT_MAX_BYTES} bytes."
        ));
    }
    if prompt.chars().any(is_refused_control) {
        return Err("The prompt contains control characters other than newline and tab.".into());
    }
    Ok(())
}

impl AutopilotSpec {
    /// The bounds of design decision 2. The server, the CLI and the tab's
    /// extension each check them; this is the one Rust statement of them.
    pub fn validate(&self) -> Result<(), AutopilotRefusal> {
        validate_autopilot_prompt(&self.prompt)?;
        let interval = match self.repeat {
            AutopilotRepeat::AfterEachTurn => None,
            AutopilotRepeat::Every { seconds } => {
                if !(AUTOPILOT_INTERVAL_MIN_SECS..=AUTOPILOT_INTERVAL_MAX_SECS).contains(&seconds)
                {
                    return Err("The repeat interval must be between one minute and 24 hours.".into());
                }
                Some(seconds)
            }
        };
        if let AutopilotLimit::AtMost { value } = self.iteration_limit {
            if !(1..=AUTOPILOT_ITERATION_LIMIT_MAX).contains(&value) {
                return Err(format!(
                    "The iteration limit must be between 1 and {AUTOPILOT_ITERATION_LIMIT_MAX}, or none."
                ));
            }
        }
        if let AutopilotLimit::AtMost { value } = self.time_limit {
            if !(AUTOPILOT_TIME_LIMIT_MIN_SECS..=AUTOPILOT_TIME_LIMIT_MAX_SECS).contains(&value) {
                return Err("The time limit must be between one minute and 365 days, or none.".into());
            }
            if interval.is_some_and(|seconds| value < seconds) {
                return Err("The time limit is shorter than the repeat interval.".into());
            }
        }
        Ok(())
    }
}

impl AutopilotStartRequest {
    /// The strict spec this request asks for, or why it is refused. A limit
    /// that is left out is refused: it must be a number or an explicit "none".
    pub fn validate(&self) -> Result<AutopilotSpec, AutopilotRefusal> {
        let spec = AutopilotSpec {
            prompt: self
                .prompt
                .clone()
                .ok_or_else(|| "Autopilot needs a prompt.".to_string())?,
            repeat: self
                .repeat
                .ok_or_else(|| "Say when autopilot repeats.".to_string())?,
            iteration_limit: self.iteration_limit.ok_or_else(|| {
                "Say whether autopilot has an iteration limit: a number, or none.".to_string()
            })?,
            time_limit: self.time_limit.ok_or_else(|| {
                "Say whether autopilot has a time limit: a duration, or none.".to_string()
            })?,
            end_without_tool_call: self.end_without_tool_call.unwrap_or(false),
        };
        spec.validate()?;
        Ok(spec)
    }
}

impl From<&AutopilotSpec> for AutopilotStartRequest {
    fn from(spec: &AutopilotSpec) -> Self {
        Self {
            prompt: Some(spec.prompt.clone()),
            repeat: Some(spec.repeat),
            iteration_limit: Some(spec.iteration_limit),
            time_limit: Some(spec.time_limit),
            end_without_tool_call: Some(spec.end_without_tool_call),
        }
    }
}

/// What a viewer asks a tab's autopilot to do
/// (`WebToServer::ControlPaneAutopilot.action`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutopilotAction {
    Start {
        spec: AutopilotStartRequest,
    },
    /// Stop further iterations and let a turn in progress continue. The id is
    /// a hint: a pause applies to whatever autopilot is current, because a
    /// pause must not be refused for a stale view.
    Pause {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        autopilot_id: Option<Uuid>,
    },
    /// Continue the named paused autopilot. Refused when that is not the
    /// tab's paused autopilot.
    Resume {
        autopilot_id: Uuid,
    },
    /// End autopilot. With `now`, also interrupt the turn in progress. The id
    /// is a hint, as for pause.
    Stop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        autopilot_id: Option<Uuid>,
        #[serde(default)]
        now: bool,
    },
}

/// The same request as the server forwards it to the tab's instance
/// (`ServerToCli::ControlPaneAutopilot.command`): a start carries the strict
/// spec, and nothing unknown is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutopilotCommand {
    Start {
        spec: AutopilotSpec,
    },
    Pause {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        autopilot_id: Option<Uuid>,
    },
    Resume {
        autopilot_id: Uuid,
    },
    Stop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        autopilot_id: Option<Uuid>,
        #[serde(default)]
        now: bool,
    },
}

impl AutopilotAction {
    /// The strict command for this action, or why it is refused.
    pub fn validate(&self) -> Result<AutopilotCommand, AutopilotRefusal> {
        Ok(match self {
            Self::Start { spec } => AutopilotCommand::Start {
                spec: spec.validate()?,
            },
            Self::Pause { autopilot_id } => AutopilotCommand::Pause {
                autopilot_id: *autopilot_id,
            },
            Self::Resume { autopilot_id } => AutopilotCommand::Resume {
                autopilot_id: *autopilot_id,
            },
            Self::Stop { autopilot_id, now } => AutopilotCommand::Stop {
                autopilot_id: *autopilot_id,
                now: *now,
            },
        })
    }
}

impl AutopilotCommand {
    /// Checked again on the tab's machine, which may be newer or older than
    /// the server that forwarded the command.
    pub fn validate(&self) -> Result<(), AutopilotRefusal> {
        match self {
            Self::Start { spec } => spec.validate(),
            Self::Pause { .. } | Self::Resume { .. } | Self::Stop { .. } => Ok(()),
        }
    }

    /// A start or a resume is never left pending; a pause or a stop stays in
    /// force until the tab applies it.
    pub fn starts_work(&self) -> bool {
        matches!(self, Self::Start { .. } | Self::Resume { .. })
    }
}

/// How a request was answered. `Accepted` means the tab took the request; the
/// displayed state changes only when the tab reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutopilotReceiptOutcome {
    Accepted,
    /// With a reason. Nothing was started or changed.
    Refused,
    /// Not confirmed in time. A pause or a stop stays in force until the tab
    /// applies it. A start or a resume may have been taken; the tab's
    /// reported state shows it. A client never resends by itself.
    Uncertain,
}

/// Whether autopilot can be used on one OMP terminal tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutopilotSupport {
    Supported,
    /// The tab's agent was launched with autopilot support and no fresh state
    /// report exists, or nothing has arrived from the tab's instance. Shown
    /// as not reported, never as off. Stop and Stop now stay available.
    NotReported,
    /// The agent was started by an APAS that had no autopilot. It is never
    /// rebooted automatically.
    NeedsAgentReboot,
    /// The tab's OMP is below the minimum version or lacks a call autopilot
    /// uses.
    UnsupportedOmp,
    UnavailableOnPlatform,
    /// Added by the server for viewers, never sent by a CLI: the instance
    /// that owns the tab is attached without [`OMP_TAB_AUTOPILOT_CAPABILITY`].
    InstanceUpdateRequired,
}

/// Values of [`AutopilotCondition::kind`].
pub mod autopilot_condition {
    pub const STARTING: &str = "starting";
    pub const RUNNING: &str = "running";
    pub const WAITING: &str = "waiting";
    pub const HOLDING: &str = "holding";
    pub const PAUSED: &str = "paused";

    /// `detail` of a waiting autopilot.
    pub const WAIT_NEXT_ITERATION: &str = "next_iteration";
    pub const WAIT_SETTLE: &str = "settle";
    pub const WAIT_BACKGROUND_WORK: &str = "background_work";

    /// `detail` of a holding autopilot.
    pub const HOLD_CONTROLLER: &str = "controller";
    pub const HOLD_SERVER: &str = "server";

    /// `detail` of a paused autopilot: why.
    pub const PAUSE_REQUESTED: &str = "requested";
    pub const PAUSE_FAILURES: &str = "failures";
    pub const PAUSE_INTERRUPTED: &str = "interrupted";
    pub const PAUSE_BACKGROUND_WAIT_ENDED: &str = "background_wait_ended";
    pub const PAUSE_NO_CONTROL: &str = "no_control";
    pub const PAUSE_AGENT_RESTARTED: &str = "agent_restarted";
    /// Reported by the CLI on the tab's behalf, never by the tab.
    pub const PAUSE_AGENT_EXITED: &str = "agent_exited";
    pub const PAUSE_OMP_GOAL_MODE: &str = "omp_goal_mode";
    pub const PAUSE_OMP_LOOP_MODE: &str = "omp_loop_mode";
    pub const PAUSE_OMP_PLAN_MODE: &str = "omp_plan_mode";
    pub const PAUSE_OMP_AUTORESEARCH: &str = "omp_autoresearch";

    /// `reason` of an ended autopilot.
    pub const END_STOPPED: &str = "stopped";
    pub const END_ITERATION_LIMIT: &str = "iteration_limit";
    pub const END_TIME_LIMIT: &str = "time_limit";
    pub const END_TIME_LIMIT_BEFORE_NEXT: &str = "time_limit_before_next";
    pub const END_NO_TOOL_CALL: &str = "no_tool_call";
    pub const END_CONVERSATION_CHANGED: &str = "conversation_changed";
}

/// What a tab on autopilot is doing. `kind` and `detail` are plain strings
/// from [`autopilot_condition`], not enums, because a tab's extension can be
/// newer or older than the CLI, the server and the viewer that read its
/// report. A reader shows a value it does not know as it would an unknown
/// state, and never drops the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutopilotCondition {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// RFC 3339 time the condition began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

impl AutopilotCondition {
    /// A tab is on autopilot while its autopilot is starting, running,
    /// waiting or holding. A paused one is not on.
    pub fn is_on(&self) -> bool {
        self.kind != autopilot_condition::PAUSED
    }
}

/// A tab's current autopilot: on, or paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutopilotView {
    pub autopilot_id: Uuid,
    pub spec: AutopilotSpec,
    /// RFC 3339.
    pub started_at: String,
    /// Iterations submitted so far.
    pub iterations: u64,
    /// RFC 3339 time the time limit ends, when the autopilot has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// RFC 3339 time the next iteration is due, while it waits for that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_iteration_at: Option<String>,
    pub condition: AutopilotCondition,
}

/// A tab's last ended autopilot, kept so its reason can be shown and its
/// values offered for a new start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutopilotEnded {
    pub autopilot_id: Uuid,
    pub spec: AutopilotSpec,
    pub started_at: String,
    pub iterations: u64,
    /// One of the `END_*` values of [`autopilot_condition`].
    pub reason: String,
    pub ended_at: String,
    /// Whether a turn that started after the autopilot ended has run. Lists
    /// stop showing the reason once one has.
    #[serde(default)]
    pub turn_since: bool,
}

/// OMP's built-in goal mode, as OMP reports it. APAS shows it and never
/// switches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OmpGoalModeView {
    /// "active" or "paused".
    pub status: String,
    /// Cut to 500 characters by the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// Known only after OMP's first `goal_updated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u64>,
    /// OMP's own goal status, such as budget-limited; known only after its
    /// first `goal_updated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_status: Option<String>,
}

/// OMP's built-in loop mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OmpLoopModeView {
    /// "active" or "paused".
    pub status: String,
}

/// OMP's own self-continuing modes on a tab, however they were switched on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OmpModesView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<OmpGoalModeView>,
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_mode: Option<OmpLoopModeView>,
}

impl OmpModesView {
    pub fn is_empty(&self) -> bool {
        self.goal.is_none() && self.loop_mode.is_none()
    }
}

/// What one OMP terminal tab reports about autopilot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaneAutopilotState {
    /// The current autopilot, on or paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot: Option<AutopilotView>,
    /// The last ended one, until another starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ended: Option<AutopilotEnded>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omp_modes: Option<OmpModesView>,
    /// True for the one state the CLI reports itself: the pause it shows after
    /// the tab's agent process has exited. No Resume, Stop or Stop now is
    /// offered for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reported_by_cli: bool,
}

/// The autopilot part of a pane summary in session lists. It never carries
/// the prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaneAutopilotSummary {
    pub support: AutopilotSupport,
    /// [`AutopilotCondition::kind`] of the current autopilot, or "ended" for
    /// an ended one whose reason lists still show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// The condition's detail, or the end reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_iteration_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u64>,
    /// Absent when there is no iteration limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration_limit: Option<u64>,
    /// "active" or "paused" for OMP's own goal mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omp_goal: Option<String>,
    /// "active" or "paused" for OMP's own loop mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omp_loop: Option<String>,
}

impl PaneAutopilotSummary {
    pub const CONDITION_ENDED: &'static str = "ended";

    /// The list summary of a tab's reported state.
    pub fn from_state(support: AutopilotSupport, state: Option<&PaneAutopilotState>) -> Self {
        let mut summary = Self {
            support,
            condition: None,
            detail: None,
            next_iteration_at: None,
            iterations: None,
            iteration_limit: None,
            omp_goal: None,
            omp_loop: None,
        };
        let Some(state) = state else {
            return summary;
        };
        if let Some(autopilot) = &state.autopilot {
            summary.condition = Some(autopilot.condition.kind.clone());
            summary.detail = autopilot.condition.detail.clone();
            summary.next_iteration_at = autopilot.next_iteration_at.clone();
            summary.iterations = Some(autopilot.iterations);
            summary.iteration_limit = autopilot.spec.iteration_limit.value();
        } else if let Some(ended) = state.last_ended.as_ref().filter(|ended| !ended.turn_since) {
            summary.condition = Some(Self::CONDITION_ENDED.into());
            summary.detail = Some(ended.reason.clone());
            summary.iterations = Some(ended.iterations);
            summary.iteration_limit = ended.spec.iteration_limit.value();
        }
        if let Some(modes) = &state.omp_modes {
            summary.omp_goal = modes.goal.as_ref().map(|goal| goal.status.clone());
            summary.omp_loop = modes.loop_mode.as_ref().map(|mode| mode.status.clone());
        }
        summary
    }
}

/// Input of the reserved [`AUTOPILOT_ITERATION_TOOL_NAME`] tool-use block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutopilotIterationMarker {
    pub autopilot_id: Uuid,
    pub iteration: u64,
    /// Left out when the autopilot has no iteration limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// On the first marker of an autopilot and on the first after each
    /// resume, so the stored conversation holds the instruction once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> AutopilotSpec {
        AutopilotSpec {
            prompt: "Check the build and fix what fails.".into(),
            repeat: AutopilotRepeat::Every { seconds: 900 },
            iteration_limit: AutopilotLimit::AtMost { value: 20 },
            time_limit: AutopilotLimit::AtMost { value: 8 * 3600 },
            end_without_tool_call: false,
        }
    }

    #[test]
    fn a_spec_within_the_bounds_is_accepted() {
        assert_eq!(spec().validate(), Ok(()));
        let unlimited = AutopilotSpec {
            repeat: AutopilotRepeat::AfterEachTurn,
            iteration_limit: AutopilotLimit::None,
            time_limit: AutopilotLimit::None,
            end_without_tool_call: true,
            ..spec()
        };
        assert_eq!(unlimited.validate(), Ok(()));
        // The old caps are typo guards now, not policy.
        let large = AutopilotSpec {
            iteration_limit: AutopilotLimit::AtMost { value: 5_000 },
            time_limit: AutopilotLimit::AtMost {
                value: 30 * 24 * 3600,
            },
            ..spec()
        };
        assert_eq!(large.validate(), Ok(()));
    }

    #[test]
    fn each_bound_refuses_with_a_reason() {
        let cases = [
            AutopilotSpec {
                prompt: "  \n".into(),
                ..spec()
            },
            AutopilotSpec {
                prompt: "x".repeat(AUTOPILOT_PROMPT_MAX_BYTES + 1),
                ..spec()
            },
            AutopilotSpec {
                prompt: "line\r\n".into(),
                ..spec()
            },
            AutopilotSpec {
                prompt: "bell\u{7}".into(),
                ..spec()
            },
            AutopilotSpec {
                prompt: "c1\u{85}".into(),
                ..spec()
            },
            AutopilotSpec {
                repeat: AutopilotRepeat::Every { seconds: 10 },
                ..spec()
            },
            AutopilotSpec {
                repeat: AutopilotRepeat::Every {
                    seconds: AUTOPILOT_INTERVAL_MAX_SECS + 1,
                },
                time_limit: AutopilotLimit::None,
                ..spec()
            },
            AutopilotSpec {
                iteration_limit: AutopilotLimit::AtMost { value: 0 },
                ..spec()
            },
            AutopilotSpec {
                iteration_limit: AutopilotLimit::AtMost {
                    value: AUTOPILOT_ITERATION_LIMIT_MAX + 1,
                },
                ..spec()
            },
            AutopilotSpec {
                time_limit: AutopilotLimit::AtMost { value: 10 },
                repeat: AutopilotRepeat::AfterEachTurn,
                ..spec()
            },
            AutopilotSpec {
                time_limit: AutopilotLimit::AtMost {
                    value: AUTOPILOT_TIME_LIMIT_MAX_SECS + 1,
                },
                ..spec()
            },
            // Shorter than its fifteen-minute interval.
            AutopilotSpec {
                time_limit: AutopilotLimit::AtMost { value: 600 },
                ..spec()
            },
        ];
        for case in cases {
            let refusal = case.validate().expect_err("must be refused");
            assert!(!refusal.is_empty());
        }
        // Newline and tab are the two control characters a prompt may hold.
        let multiline = AutopilotSpec {
            prompt: "first\n\tsecond".into(),
            ..spec()
        };
        assert_eq!(multiline.validate(), Ok(()));
    }

    #[test]
    fn a_start_must_say_whether_it_is_limited() {
        let complete = AutopilotStartRequest::from(&spec());
        assert_eq!(complete.validate(), Ok(spec()));

        for missing in [
            AutopilotStartRequest {
                iteration_limit: None,
                ..complete.clone()
            },
            AutopilotStartRequest {
                time_limit: None,
                ..complete.clone()
            },
            AutopilotStartRequest {
                prompt: None,
                ..complete.clone()
            },
            AutopilotStartRequest {
                repeat: None,
                ..complete.clone()
            },
        ] {
            assert!(missing.validate().is_err());
        }

        // A request with nothing in it still parses, so the server can answer
        // it instead of dropping the frame.
        let empty: AutopilotStartRequest = serde_json::from_value(json!({})).unwrap();
        assert!(empty.validate().is_err());

        let none: AutopilotStartRequest = serde_json::from_value(json!({
            "prompt": "go",
            "repeat": { "kind": "after_each_turn" },
            "iteration_limit": { "kind": "none" },
            "time_limit": { "kind": "none" },
        }))
        .unwrap();
        let spec = none.validate().unwrap();
        assert_eq!(spec.iteration_limit, AutopilotLimit::None);
        assert_eq!(spec.time_limit, AutopilotLimit::None);
        assert!(!spec.end_without_tool_call);
    }

    #[test]
    fn the_strict_forms_reject_unknown_fields_and_missing_limits() {
        let value = serde_json::to_value(spec()).unwrap();
        assert_eq!(
            value,
            json!({
                "prompt": "Check the build and fix what fails.",
                "repeat": { "kind": "every", "seconds": 900 },
                "iteration_limit": { "kind": "at_most", "value": 20 },
                "time_limit": { "kind": "at_most", "value": 28800 },
                "end_without_tool_call": false,
            })
        );
        assert_eq!(serde_json::from_value::<AutopilotSpec>(value.clone()).unwrap(), spec());

        let mut extra = value.clone();
        extra["token_limit"] = json!(1000);
        assert!(serde_json::from_value::<AutopilotSpec>(extra).is_err());

        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove("time_limit");
        assert!(serde_json::from_value::<AutopilotSpec>(missing).is_err());

        let mut bad_limit = value;
        bad_limit["iteration_limit"] = json!({ "kind": "at_most", "value": 20, "soft": true });
        assert!(serde_json::from_value::<AutopilotSpec>(bad_limit).is_err());

        let command = json!({ "kind": "stop", "now": true, "force": true });
        assert!(serde_json::from_value::<AutopilotCommand>(command).is_err());
    }

    #[test]
    fn an_action_becomes_the_strict_command() {
        let start = AutopilotAction::Start {
            spec: AutopilotStartRequest::from(&spec()),
        };
        assert_eq!(
            start.validate(),
            Ok(AutopilotCommand::Start { spec: spec() })
        );
        let id = Uuid::new_v4();
        let stop: AutopilotAction =
            serde_json::from_value(json!({ "kind": "stop", "now": true })).unwrap();
        assert_eq!(
            stop.validate(),
            Ok(AutopilotCommand::Stop {
                autopilot_id: None,
                now: true
            })
        );
        let resume = AutopilotAction::Resume { autopilot_id: id };
        assert_eq!(
            resume.validate(),
            Ok(AutopilotCommand::Resume { autopilot_id: id })
        );
        assert!(resume.validate().unwrap().starts_work());
        assert!(!stop.validate().unwrap().starts_work());

        let bad = AutopilotAction::Start {
            spec: AutopilotStartRequest::default(),
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn a_state_with_values_this_build_does_not_know_still_parses() {
        let state: PaneAutopilotState = serde_json::from_value(json!({
            "autopilot": {
                "autopilot_id": Uuid::new_v4(),
                "spec": spec(),
                "started_at": "2026-11-13T14:00:00Z",
                "iterations": 3,
                "condition": { "kind": "meditating", "detail": "deeply", "extra": 1 },
                "future_field": true,
            },
            "omp_modes": { "loop": { "status": "paused" } },
            "another_future_field": [1, 2, 3],
        }))
        .unwrap();
        let autopilot = state.autopilot.as_ref().unwrap();
        assert_eq!(autopilot.condition.kind, "meditating");
        assert!(autopilot.condition.is_on());
        assert_eq!(
            state.omp_modes.as_ref().unwrap().loop_mode.as_ref().unwrap().status,
            "paused"
        );
        assert!(!state.reported_by_cli);
        // The mark stays off the wire unless it is set.
        assert!(serde_json::to_value(&state).unwrap().get("reported_by_cli").is_none());
    }

    #[test]
    fn the_summary_never_carries_the_prompt() {
        let id = Uuid::new_v4();
        let state = PaneAutopilotState {
            autopilot: Some(AutopilotView {
                autopilot_id: id,
                spec: spec(),
                started_at: "2026-11-13T14:00:00Z".into(),
                iterations: 7,
                deadline: Some("2026-11-13T22:00:00Z".into()),
                next_iteration_at: Some("2026-11-13T14:30:00Z".into()),
                condition: AutopilotCondition {
                    kind: autopilot_condition::WAITING.into(),
                    detail: Some(autopilot_condition::WAIT_NEXT_ITERATION.into()),
                    since: None,
                },
            }),
            ..PaneAutopilotState::default()
        };
        let summary = PaneAutopilotSummary::from_state(AutopilotSupport::Supported, Some(&state));
        assert_eq!(summary.condition.as_deref(), Some("waiting"));
        assert_eq!(summary.iterations, Some(7));
        assert_eq!(summary.iteration_limit, Some(20));
        let text = serde_json::to_string(&summary).unwrap();
        assert!(!text.contains("Check the build"));

        let ended = PaneAutopilotState {
            last_ended: Some(AutopilotEnded {
                autopilot_id: id,
                spec: AutopilotSpec {
                    iteration_limit: AutopilotLimit::None,
                    ..spec()
                },
                started_at: "2026-11-13T14:00:00Z".into(),
                iterations: 12,
                reason: autopilot_condition::END_TIME_LIMIT.into(),
                ended_at: "2026-11-13T22:00:00Z".into(),
                turn_since: false,
            }),
            ..PaneAutopilotState::default()
        };
        let summary = PaneAutopilotSummary::from_state(AutopilotSupport::Supported, Some(&ended));
        assert_eq!(summary.condition.as_deref(), Some("ended"));
        assert_eq!(summary.detail.as_deref(), Some("time_limit"));
        assert_eq!(summary.iteration_limit, None);

        // Once a later turn has run, lists stop showing the ended reason.
        let mut seen = ended;
        seen.last_ended.as_mut().unwrap().turn_since = true;
        let summary = PaneAutopilotSummary::from_state(AutopilotSupport::Supported, Some(&seen));
        assert_eq!(summary.condition, None);

        let nothing = PaneAutopilotSummary::from_state(AutopilotSupport::NotReported, None);
        assert_eq!(nothing.support, AutopilotSupport::NotReported);
        assert_eq!(nothing.condition, None);
    }

    #[test]
    fn the_marker_leaves_out_what_it_does_not_have() {
        let marker = AutopilotIterationMarker {
            autopilot_id: Uuid::nil(),
            iteration: 3,
            limit: None,
            prompt: None,
        };
        assert_eq!(
            serde_json::to_value(&marker).unwrap(),
            json!({ "autopilot_id": Uuid::nil(), "iteration": 3 })
        );
    }
}
