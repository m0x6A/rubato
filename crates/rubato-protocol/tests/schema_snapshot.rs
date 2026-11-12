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

/// ADR 0030 and 0004: a person's GitHub token never leaves the server. No wire type has a
/// property named like a token, except the companion's pairing token (ADR 0011). Fails on
/// purpose for `{"github_token": ...}`.
#[test]
fn no_wire_type_carries_a_token() {
    fn names(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(serde_json::Value::Object(props)) = m.get("properties") {
                    out.extend(props.keys().cloned());
                }
                m.values().for_each(|x| names(x, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| names(x, out)),
            _ => {}
        }
    }
    let leaks = |schema: &serde_json::Value, allowed: &[&str]| {
        let mut n = Vec::new();
        names(schema, &mut n);
        n.into_iter()
            .filter(|p| p.to_ascii_lowercase().contains("token") && !allowed.contains(&p.as_str()))
            .collect::<Vec<_>>()
    };
    let all = rubato_protocol::schema::all();
    let mut checked = 0;
    for (name, schema) in &all {
        let schema = serde_json::to_value(schema).unwrap();
        let allowed: &[&str] = if *name == "pair_poll_response" {
            &["token"]
        } else {
            &[]
        };
        assert_eq!(leaks(&schema, allowed), Vec::<String>::new(), "{name}");
        checked += 1;
    }
    let bad = serde_json::json!({"properties": {"github_token": {"type": "string"}}});
    assert_eq!(
        leaks(&bad, &[]),
        ["github_token"],
        "the check fails on purpose"
    );
    println!("checked {checked} schemas for token properties");
}
