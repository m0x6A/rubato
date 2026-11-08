//! The exported JSON Schemas are committed under `schema/protocol/`. This test fails when a
//! type changed and the snapshot did not. Run with `UPDATE_SCHEMAS=1` to rewrite them.

use std::path::PathBuf;

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/protocol")
}

/// The two schemas AGENTS.md names at the top of `schema/`. Same content as their protocol export.
const ALIASES: &[(&str, &str)] = &[
    ("system_map", "../../schema/system-map.schema.json"),
    ("plan", "../../schema/plan.schema.json"),
];

#[test]
fn exported_schemas_match_committed_snapshots() {
    let update = std::env::var_os("UPDATE_SCHEMAS").is_some();
    let dir = schema_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let mut drifted = Vec::new();
    let all = rubato_protocol::schema::all();
    for (name, schema) in &all {
        let path = dir.join(format!("{name}.schema.json"));
        let mut rendered = serde_json::to_string_pretty(schema).unwrap();
        rendered.push('\n');
        if update {
            std::fs::write(&path, &rendered).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(on_disk) if on_disk == rendered => {}
            Ok(_) => drifted.push(format!("{name}: differs from {}", path.display())),
            Err(_) => drifted.push(format!("{name}: missing {}", path.display())),
        }
    }
    for (name, rel) in ALIASES {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
        let schema = all
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s)
            .expect("alias names an exported schema");
        let mut rendered = serde_json::to_string_pretty(schema).unwrap();
        rendered.push('\n');
        if update {
            std::fs::write(&path, &rendered).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(on_disk) if on_disk == rendered => {}
            Ok(_) => drifted.push(format!("{name}: alias differs from {}", path.display())),
            Err(_) => drifted.push(format!("{name}: missing alias {}", path.display())),
        }
    }
    println!(
        "checked {} schemas and {} aliases against {}",
        all.len(),
        ALIASES.len(),
        dir.display()
    );
    assert!(
        drifted.is_empty(),
        "schema snapshots drifted (run with UPDATE_SCHEMAS=1 to rewrite):\n{}",
        drifted.join("\n")
    );
}

#[test]
fn no_stale_snapshot_files() {
    let names: Vec<String> = rubato_protocol::schema::all()
        .iter()
        .map(|(n, _)| format!("{n}.schema.json"))
        .collect();
    let mut stale = Vec::new();
    for entry in std::fs::read_dir(schema_dir()).unwrap() {
        let file = entry.unwrap().file_name().to_string_lossy().into_owned();
        if !names.contains(&file) {
            stale.push(file);
        }
    }
    assert!(
        stale.is_empty(),
        "snapshot files with no type behind them: {stale:?}"
    );
}

#[test]
fn tampered_snapshot_is_detected() {
    // The check must be able to fail: a one-character change in a committed snapshot is a drift.
    let (name, schema) = rubato_protocol::schema::all().into_iter().next().unwrap();
    let path = schema_dir().join(format!("{name}.schema.json"));
    let mut on_disk = std::fs::read_to_string(&path).unwrap();
    let mut rendered = serde_json::to_string_pretty(&schema).unwrap();
    rendered.push('\n');
    assert_eq!(on_disk, rendered, "precondition: snapshot is current");
    on_disk.insert(1, ' ');
    assert_ne!(
        on_disk, rendered,
        "a tampered snapshot must not compare equal"
    );
}
