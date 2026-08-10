use shared::{CodeEvent, ServerToWeb, WebToServer};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/protocol/fixtures")
}

#[test]
fn golden_mobile_fixtures_match_rust_wire_types() {
    let mut seen = 0;
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
        } else {
            panic!("unclassified protocol fixture: {}", path.display());
        }
        seen += 1;
    }
    assert!(seen >= 10, "expected representative mobile fixtures");
}
