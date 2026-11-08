//! Every file under `fixtures/` is a protocol envelope that must deserialize and round-trip.
//! The companion and server suites replay the same files (seam `companion-server`).

use rubato_protocol::{CompanionEnvelope, ServerEnvelope};
use std::path::PathBuf;

fn fixtures() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn round_trip<T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug>(
    name: &str,
    text: &str,
) {
    let value: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: not JSON: {e}"));
    let typed: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|e| panic!("{name}: does not deserialize: {e}"));
    let back = serde_json::to_value(&typed).unwrap();
    assert_eq!(back, value, "{name}: round trip changed the document");
}

#[test]
fn all_fixtures_round_trip() {
    let files = fixtures();
    assert!(!files.is_empty(), "no fixtures found");
    for (name, text) in &files {
        if name.starts_with("companion_") {
            round_trip::<CompanionEnvelope>(name, text);
        } else if name.starts_with("server_") {
            round_trip::<ServerEnvelope>(name, text);
        } else {
            panic!("{name}: fixture must start with companion_ or server_");
        }
    }
    println!("checked {} fixtures", files.len());
}

#[test]
fn a_fixture_that_must_fail() {
    // A seam check needs a test that makes it fail on purpose: wrong protocol shape is refused.
    let bad =
        r#"{"seq":1,"sent_at":"2026-11-08T12:00:00Z","message":{"type":"hook","session_id":"x"}}"#;
    assert!(serde_json::from_str::<CompanionEnvelope>(bad).is_err());
}
