//! A project's saved autopilot prompts: the wire contract.
//!
//! Anyone who may start autopilot on a project's tabs can save the start
//! dialog's prompt to the project under a name, replace or delete a saved
//! prompt, and make one the project's default. They are kept in the
//! project's `.apas` file, as `autopilot_prompts` and
//! `default_autopilot_prompt`, which only the CLI writes, through the locked
//! merge. The design record is
//! `openspec/changes/add-autopilot-prompt-library/`.
//!
//! Saving a prompt sends nothing to any tab and never starts, pauses or stops
//! an autopilot.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::autopilot::{validate_autopilot_prompt, AutopilotRefusal};

/// Negotiated twice, as autopilot's markers are.
///
/// A project instance (CLI) advertises it when it can keep saved prompts in
/// `.apas`; the server echoes it in `ServerToCli::Registered`'s
/// `negotiated_capabilities` and sends `ServerToCli::UpdateAutopilotPrompts`
/// only to instances that advertised it, because an older CLI drops an
/// unknown frame without an answer. A CLI that does not see it echoed sends
/// no `CliToServer::ProjectAutopilotPrompts`.
///
/// A viewer advertises it in `Authenticate.capabilities` to receive
/// `ServerToWeb::ProjectAutopilotPrompts` and `AutopilotPromptsReceipt`; the
/// server echoes it in `Authenticated.negotiated_capabilities`. A viewer
/// that does not see it echoed sends no `WebToServer::UpdateAutopilotPrompts`
/// and offers no saved prompts. Older native builds drop their socket on a
/// frame type their bundled schema lacks, which is why the gate exists.
pub const PROJECT_AUTOPILOT_PROMPTS_CAPABILITY: &str = "project_autopilot_prompts_v1";

/// Longest name of a saved prompt, in characters after trimming.
pub const AUTOPILOT_PROMPT_NAME_MAX_CHARS: usize = 80;
/// Most saved prompts one project keeps. At 16 KB each that bounds the list
/// frame to about 800 KB, sent only on change and on attach.
pub const PROJECT_AUTOPILOT_PROMPTS_MAX: usize = 50;

/// The web's built-in templates, in the order the Template control lists
/// them (`packages/web/src/lib/autopilotTemplates.ts`; a test keeps the two
/// in step). A saved prompt may not take one of these names, ignoring case,
/// so that the control never shows two entries under one name.
pub const AUTOPILOT_BUILTIN_TEMPLATE_NAMES: [&str; 4] = [
    "APAS worker",
    "Implement from a design document",
    "Work through TODO.md",
    "Keep the tests green",
];

/// Why every change is refused while the project's `.apas` holds a list that
/// does not parse. The CLI never rewrites such a list; the person fixes or
/// removes it by hand.
pub const AUTOPILOT_PROMPTS_UNREADABLE_REASON: &str =
    "The saved prompts in .apas cannot be read. Fix or remove autopilot_prompts there.";

/// Why a change is refused when the session's primary instance runs a CLI
/// without [`PROJECT_AUTOPILOT_PROMPTS_CAPABILITY`].
pub fn autopilot_prompts_instance_update_required(host: &str) -> String {
    format!("{host} runs an older APAS that cannot save prompts. Reboot {host} to update it.")
}

/// One prompt saved to a project, as `.apas` keeps it and the wire carries
/// it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct SavedAutopilotPrompt {
    /// Trimmed, 1 to [`AUTOPILOT_PROMPT_NAME_MAX_CHARS`] characters, unique in
    /// the project ignoring case.
    pub name: String,
    /// Meets the bounds of a start's prompt
    /// ([`crate::validate_autopilot_prompt`]).
    pub prompt: String,
    /// RFC 3339 time of the last save, set by the CLI that wrote it.
    pub updated_at: String,
}

/// One change to a project's saved prompts
/// (`WebToServer::UpdateAutopilotPrompts.change`). Validated by the server
/// against its cached list and again by each CLI against its `.apas`, with
/// [`validate_autopilot_prompt_change`].
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutopilotPromptChange {
    /// Save `prompt` under `name`. A name that is already saved, ignoring
    /// case, is refused unless `replace` is true, which the dialog sends only
    /// after telling the person the save replaces that prompt. A replacement
    /// takes the entry's place in the list.
    Save {
        name: String,
        prompt: String,
        #[serde(default)]
        replace: bool,
    },
    /// Delete a saved prompt. Deleting the default also clears the default.
    Delete { name: String },
    /// Make a saved prompt the project's default, or clear the default with
    /// no name.
    SetDefault {
        #[serde(default)]
        name: Option<String>,
    },
}

/// The position in `existing` of the saved prompt `name` refers to: names are
/// compared trimmed and ignoring case, as uniqueness is decided. The CLI uses
/// it to apply a change to the same entry the validation accepted.
pub fn find_saved_autopilot_prompt(existing: &[SavedAutopilotPrompt], name: &str) -> Option<usize> {
    let wanted = fold_name(name);
    existing
        .iter()
        .position(|saved| fold_name(&saved.name) == wanted)
}

fn fold_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn validate_new_name(name: &str) -> Result<(), AutopilotRefusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the saved prompt a name.".into());
    }
    if name.chars().count() > AUTOPILOT_PROMPT_NAME_MAX_CHARS {
        return Err(format!(
            "A saved prompt's name is at most {AUTOPILOT_PROMPT_NAME_MAX_CHARS} characters."
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("A saved prompt's name cannot contain control characters.".into());
    }
    let folded = fold_name(name);
    if let Some(builtin) = AUTOPILOT_BUILTIN_TEMPLATE_NAMES
        .iter()
        .find(|builtin| fold_name(builtin) == folded)
    {
        return Err(format!(
            "\"{builtin}\" is the name of a built-in template. Choose another name."
        ));
    }
    Ok(())
}

fn require_existing(
    existing: &[SavedAutopilotPrompt],
    name: &str,
) -> Result<usize, AutopilotRefusal> {
    find_saved_autopilot_prompt(existing, name)
        .ok_or_else(|| format!("No saved prompt is named \"{}\".", name.trim()))
}

/// Whether `change` may be applied to a project whose saved prompts are
/// `existing`, or why it is refused. The text is shown to the person who
/// asked. The server checks it against its cached list and each CLI again
/// against its `.apas`; a CLI whose list is unreadable refuses every change
/// with [`AUTOPILOT_PROMPTS_UNREADABLE_REASON`] before calling this.
///
/// Names are compared trimmed; the CLI stores a saved name trimmed, because
/// this function cannot change the request. `default` is the project's
/// current default. No rule reads it today: deleting the default clears it
/// and setting it again is harmless. It is taken so that every caller
/// passes the whole state the change applies to.
pub fn validate_autopilot_prompt_change(
    change: &AutopilotPromptChange,
    existing: &[SavedAutopilotPrompt],
    default: Option<&str>,
) -> Result<(), AutopilotRefusal> {
    let _ = default;
    match change {
        AutopilotPromptChange::Save {
            name,
            prompt,
            replace,
        } => {
            validate_new_name(name)?;
            match find_saved_autopilot_prompt(existing, name) {
                Some(index) if !replace => {
                    return Err(format!(
                        "A saved prompt named \"{}\" already exists.",
                        existing[index].name.trim()
                    ));
                }
                Some(_) => {}
                None if existing.len() >= PROJECT_AUTOPILOT_PROMPTS_MAX => {
                    return Err(format!(
                        "A project keeps at most {PROJECT_AUTOPILOT_PROMPTS_MAX} saved prompts. Delete one first."
                    ));
                }
                None => {}
            }
            validate_autopilot_prompt(prompt)
        }
        AutopilotPromptChange::Delete { name } => require_existing(existing, name).map(|_| ()),
        AutopilotPromptChange::SetDefault { name: Some(name) } => {
            require_existing(existing, name).map(|_| ())
        }
        AutopilotPromptChange::SetDefault { name: None } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AUTOPILOT_PROMPT_MAX_BYTES;
    use serde_json::json;

    fn saved(name: &str) -> SavedAutopilotPrompt {
        SavedAutopilotPrompt {
            name: name.into(),
            prompt: format!("Do what {name} says."),
            updated_at: "2026-11-14T09:00:00Z".into(),
        }
    }

    fn save(name: &str, replace: bool) -> AutopilotPromptChange {
        AutopilotPromptChange::Save {
            name: name.into(),
            prompt: "Review the open pull requests.".into(),
            replace,
        }
    }

    fn refusal(change: &AutopilotPromptChange, existing: &[SavedAutopilotPrompt]) -> String {
        validate_autopilot_prompt_change(change, existing, None).unwrap_err()
    }

    #[test]
    fn a_new_name_within_the_bounds_is_saved() {
        let existing = vec![saved("Nightly review")];
        assert_eq!(
            validate_autopilot_prompt_change(&save("  Release check  ", false), &existing, None),
            Ok(())
        );
        let longest = "n".repeat(AUTOPILOT_PROMPT_NAME_MAX_CHARS);
        assert_eq!(
            validate_autopilot_prompt_change(&save(&longest, false), &[], None),
            Ok(())
        );
        // Characters, not bytes.
        let accented = "é".repeat(AUTOPILOT_PROMPT_NAME_MAX_CHARS);
        assert_eq!(
            validate_autopilot_prompt_change(&save(&accented, false), &[], None),
            Ok(())
        );
    }

    #[test]
    fn each_name_bound_refuses_with_a_reason() {
        assert_eq!(
            refusal(&save("   ", false), &[]),
            "Give the saved prompt a name."
        );
        assert_eq!(
            refusal(
                &save(&"n".repeat(AUTOPILOT_PROMPT_NAME_MAX_CHARS + 1), false),
                &[]
            ),
            "A saved prompt's name is at most 80 characters."
        );
        for name in [
            "Two\nlines",
            "Tab\there",
            "Bell\u{7}",
            "C1\u{85}name",
            "Del\u{7f}",
        ] {
            assert_eq!(
                refusal(&save(name, false), &[]),
                "A saved prompt's name cannot contain control characters.",
                "{name:?}"
            );
        }
    }

    #[test]
    fn a_built_in_template_name_is_refused_in_any_case() {
        for name in ["APAS worker", "apas WORKER", "  keep the tests green "] {
            let reason = refusal(&save(name, true), &[]);
            assert!(reason.ends_with("is the name of a built-in template. Choose another name."));
        }
        assert_eq!(
            refusal(&save("work through todo.md", false), &[]),
            "\"Work through TODO.md\" is the name of a built-in template. Choose another name."
        );
    }

    #[test]
    fn an_existing_name_is_replaced_only_when_asked() {
        let existing = vec![saved("Nightly review")];
        assert_eq!(
            refusal(&save("nightly REVIEW ", false), &existing),
            "A saved prompt named \"Nightly review\" already exists."
        );
        assert_eq!(
            validate_autopilot_prompt_change(&save("nightly REVIEW ", true), &existing, None),
            Ok(())
        );
        // Asking to replace a name nobody saved is an ordinary save.
        assert_eq!(
            validate_autopilot_prompt_change(&save("Release check", true), &existing, None),
            Ok(())
        );
    }

    #[test]
    fn a_full_list_refuses_a_new_name_but_not_a_replacement() {
        let full: Vec<_> = (0..PROJECT_AUTOPILOT_PROMPTS_MAX)
            .map(|index| saved(&format!("Prompt {index}")))
            .collect();
        assert_eq!(
            refusal(&save("One more", false), &full),
            "A project keeps at most 50 saved prompts. Delete one first."
        );
        assert_eq!(
            validate_autopilot_prompt_change(&save("Prompt 7", true), &full, None),
            Ok(())
        );
    }

    #[test]
    fn a_saved_prompt_meets_the_bounds_of_a_start() {
        let with_prompt = |prompt: String| AutopilotPromptChange::Save {
            name: "Nightly review".into(),
            prompt,
            replace: false,
        };
        assert_eq!(
            refusal(&with_prompt(" \n\t ".into()), &[]),
            "Autopilot needs a prompt."
        );
        assert_eq!(
            refusal(
                &with_prompt("x".repeat(AUTOPILOT_PROMPT_MAX_BYTES + 1)),
                &[]
            ),
            "The prompt is longer than 16000 bytes."
        );
        assert_eq!(
            refusal(&with_prompt("Escape\u{1b}[2J".into()), &[]),
            "The prompt contains control characters other than newline and tab."
        );
        assert_eq!(
            validate_autopilot_prompt_change(
                &with_prompt("Line one\n\tLine two".into()),
                &[],
                None
            ),
            Ok(())
        );
    }

    #[test]
    fn delete_and_set_default_need_a_saved_name() {
        let existing = vec![saved("Nightly review"), saved("Release check")];
        let delete = |name: &str| AutopilotPromptChange::Delete { name: name.into() };
        let make_default = |name: &str| AutopilotPromptChange::SetDefault {
            name: Some(name.into()),
        };
        assert_eq!(
            validate_autopilot_prompt_change(&delete(" release CHECK"), &existing, None),
            Ok(())
        );
        assert_eq!(
            validate_autopilot_prompt_change(
                &make_default("Nightly review"),
                &existing,
                Some("Release check")
            ),
            Ok(())
        );
        assert_eq!(
            refusal(&delete("Weekly"), &existing),
            "No saved prompt is named \"Weekly\"."
        );
        assert_eq!(
            refusal(&make_default(" Weekly "), &existing),
            "No saved prompt is named \"Weekly\"."
        );
        // Clearing the default needs nothing, also when there is none.
        assert_eq!(
            validate_autopilot_prompt_change(
                &AutopilotPromptChange::SetDefault { name: None },
                &[],
                None
            ),
            Ok(())
        );
    }

    #[test]
    fn lookup_compares_trimmed_names_ignoring_case() {
        let existing = vec![saved("Nightly review"), saved("Ärger melden")];
        assert_eq!(
            find_saved_autopilot_prompt(&existing, "  NIGHTLY review "),
            Some(0)
        );
        assert_eq!(
            find_saved_autopilot_prompt(&existing, "ärger MELDEN"),
            Some(1)
        );
        assert_eq!(find_saved_autopilot_prompt(&existing, "Nightly"), None);
    }

    #[test]
    fn a_change_keeps_its_wire_shape() {
        let parsed: AutopilotPromptChange =
            serde_json::from_value(json!({ "kind": "save", "name": "N", "prompt": "P" })).unwrap();
        assert_eq!(
            parsed,
            AutopilotPromptChange::Save {
                name: "N".into(),
                prompt: "P".into(),
                replace: false,
            }
        );
        let parsed: AutopilotPromptChange =
            serde_json::from_value(json!({ "kind": "set_default" })).unwrap();
        assert_eq!(parsed, AutopilotPromptChange::SetDefault { name: None });
        let parsed: AutopilotPromptChange =
            serde_json::from_value(json!({ "kind": "set_default", "name": null })).unwrap();
        assert_eq!(parsed, AutopilotPromptChange::SetDefault { name: None });
        assert_eq!(
            serde_json::to_value(AutopilotPromptChange::Delete { name: "N".into() }).unwrap(),
            json!({ "kind": "delete", "name": "N" })
        );
        assert!(
            serde_json::from_value::<AutopilotPromptChange>(json!({ "kind": "rename" })).is_err()
        );
    }

    #[test]
    fn the_older_instance_refusal_names_the_host() {
        assert_eq!(
            autopilot_prompts_instance_update_required("zoo-006"),
            "zoo-006 runs an older APAS that cannot save prompts. Reboot zoo-006 to update it."
        );
    }

    /// The web's template catalog, so a template renamed or added there
    /// cannot leave a saved prompt able to take its name.
    const WEB_AUTOPILOT_TEMPLATES_TS: &str =
        include_str!("../../../packages/web/src/lib/autopilotTemplates.ts");

    #[test]
    fn rust_and_web_built_in_template_names_agree() {
        let marker = "export const AUTOPILOT_TEMPLATES";
        let start = WEB_AUTOPILOT_TEMPLATES_TS
            .find(marker)
            .expect("autopilotTemplates.ts declares AUTOPILOT_TEMPLATES");
        let catalog = &WEB_AUTOPILOT_TEMPLATES_TS[start..];
        let end = catalog.find("\n];").expect("the catalog array ends");
        let names: Vec<&str> = catalog[..end]
            .split("\n    name: \"")
            .skip(1)
            .map(|rest| &rest[..rest.find('"').expect("closing quote")])
            .collect();
        assert_eq!(names, AUTOPILOT_BUILTIN_TEMPLATE_NAMES);
    }
}
