//! Every map in the repo follows the map id rule (issue #155). Maps are found by walking the
//! folders that hold them and taking each `.yml` with a top-level `nodes:`; a map that is broken
//! on purpose for another reason (a duplicate, an unknown node) still must not break the id rule.
//! Fails on purpose with `schema/fixtures/maps/bad-ids.yml`.

use rubato_protocol::SystemMap;
use rubato_protocol::map::MapError;
use std::path::{Path, PathBuf};

const MUST_FAIL: &str = "schema/fixtures/maps/bad-ids.yml";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn maps_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
            maps_under(&p, out);
        } else if p.extension().is_some_and(|x| x == "yml" || x == "yaml")
            && std::fs::read_to_string(&p).is_ok_and(|t| t.lines().any(|l| l == "nodes:"))
        {
            out.push(p);
        }
    }
}

fn bad_ids(path: &Path) -> (usize, Vec<MapError>) {
    let map: SystemMap = serde_yaml_ng::from_str(&std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let ids = map.nodes.len() + map.seams.len();
    let bad = map
        .validate()
        .err()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| matches!(e, MapError::BadId { .. }))
        .collect();
    (ids, bad)
}

#[test]
fn every_map_in_the_repo_follows_the_id_rule() {
    let root = root();
    let mut maps = Vec::new();
    for dir in [
        "docs/design",
        "e2e/fixtures",
        "web/e2e/fixtures",
        "schema/fixtures",
    ] {
        maps_under(&root.join(dir), &mut maps);
    }
    maps.sort();
    let must_fail = root.join(MUST_FAIL);
    assert!(maps.contains(&must_fail), "the must-fail fixture is found");
    let mut checked_ids = 0;
    let mut offenders = Vec::new();
    for path in maps.iter().filter(|p| **p != must_fail) {
        let (ids, bad) = bad_ids(path);
        checked_ids += ids;
        if !bad.is_empty() {
            offenders.push(format!("{}: {bad:?}", path.display()));
        }
    }
    println!(
        "checked {} maps, {checked_ids} node and seam ids, against the map id rule",
        maps.len() - 1
    );
    assert!(maps.len() >= 8, "found only {} maps", maps.len());
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

#[test]
fn the_must_fail_map_breaks_the_rule_for_each_bad_id() {
    let (_, bad) = bad_ids(&root().join(MUST_FAIL));
    let ids: Vec<String> = bad
        .iter()
        .map(|e| match e {
            MapError::BadId { id, .. } => id.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ids, ["Billing", "a_b", "a'$(touch${IFS}p)'", "-edge"]);
}
