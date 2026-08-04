//! One turn of a terminal pane's conversation.
//!
//! An agent pane is *observed*: the CLI parses its stream-json and knows every
//! turn without cooperation. A terminal pane hosts the provider's real TUI on a
//! pty, so there is nothing structured to parse — which is why terminal panes
//! had no history and no usage counters.
//!
//! Turns are read out of the provider's own transcript by [`crate::transcript`]
//! and normalised into [`TurnRecord`], which
//! `dual_pane::conversation_turn_to_stream_messages` then dresses as the stream
//! message an agent pane would have sent. That is what gets a terminal pane's
//! history stored, rendered, and billed without a new wire message, storage
//! path, or renderer.
//!
//! An earlier design had the agent self-report each turn through an MCP tool.
//! It was removed after testing showed that neither claude nor codex acts on
//! the MCP `initialize` instructions asking for it: both connect to the server
//! and will call the tool when told to directly, but an ordinary task recorded
//! nothing at all. Reading the transcript needs no cooperation and cannot be
//! skipped.

use serde::{Deserialize, Serialize};

/// One conversation turn, normalised across providers.
///
/// Field names are chosen for what the CLI needs to rebuild a
/// `ClaudeStreamMessage`, since that is this type's only consumer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    /// Provider-reported timestamp, verbatim.
    pub ts: String,
    /// Stamped by the reader from the pane whose transcript this is.
    pub pane_id: u32,
    /// `user` or `assistant`. Anything else is preserved rather than rejected:
    /// a provider we have not integrated should degrade to "recorded but
    /// rendered plainly", not to a dropped turn.
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

impl TurnRecord {
    pub fn is_assistant(&self) -> bool {
        self.role.eq_ignore_ascii_case("assistant")
    }

    /// Whether this turn carries usage worth billing to the pane.
    pub fn has_usage(&self) -> bool {
        self.input_tokens.unwrap_or(0) > 0 || self.output_tokens.unwrap_or(0) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str) -> TurnRecord {
        TurnRecord {
            ts: "t".into(),
            pane_id: 1,
            role: role.into(),
            text: "x".into(),
            model: None,
            input_tokens: None,
            output_tokens: None,
        }
    }

    #[test]
    fn role_classification_drives_which_stream_message_a_turn_becomes() {
        assert!(turn("assistant").is_assistant());
        assert!(turn("Assistant").is_assistant(), "case-insensitive");
        assert!(!turn("user").is_assistant());
        // An unfamiliar role degrades to a user message rather than being
        // dropped — a provider we have not integrated should still show up.
        assert!(!turn("tool").is_assistant());
    }

    #[test]
    fn usage_is_only_claimed_when_the_transcript_reported_it() {
        // Emitting a Result with zero tokens would bill a turn that cost
        // nothing and pollute the pane's roll-up.
        let mut t = turn("assistant");
        assert!(!t.has_usage());
        t.output_tokens = Some(120);
        assert!(t.has_usage());
    }
}
