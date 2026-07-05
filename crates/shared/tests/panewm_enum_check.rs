use shared::messages::WebToServer;

// Regression test for the mobile "refresh doesn't fetch latest" bug.
//
// The web sends catchup watermarks as a JSON object keyed by pane id:
// `pane_watermarks: {"2": "<ts>", "97": "<ts>"}`. JSON object keys are
// always strings. `WebToServer` is an internally-tagged enum, and serde's
// tagged-enum path buffers into a Content value that does NOT coerce string
// map keys to integers — so a `HashMap<u32, _>` field rejected every one of
// these with `invalid type: string "2", expected u32`, silently dropping
// the catchup (fully broke reconnect catchup; desktop masked it via live
// streaming, mobile surfaced it as a frozen pane). The field is now
// `HashMap<String, String>` and the server parses keys to u32.
#[test]
fn pane_watermarks_string_keys_deserialize_in_webtoserver() {
    let json = r#"{"type":"get_session_messages","session_id":"4366dc38-9145-458a-a0eb-713ffbeb8438","pane_watermarks":{"2":"2026-07-05T00:00:00Z","97":"2026-07-05T00:01:00Z"}}"#;
    let parsed: WebToServer = serde_json::from_str(json)
        .expect("string-keyed pane_watermarks must deserialize inside the tagged enum");
    match parsed {
        WebToServer::GetSessionMessages {
            pane_watermarks: Some(wm),
            ..
        } => {
            assert_eq!(wm.get("2").map(String::as_str), Some("2026-07-05T00:00:00Z"));
            assert_eq!(wm.get("97").map(String::as_str), Some("2026-07-05T00:01:00Z"));
        }
        other => panic!("expected GetSessionMessages with pane_watermarks, got {other:?}"),
    }
}
