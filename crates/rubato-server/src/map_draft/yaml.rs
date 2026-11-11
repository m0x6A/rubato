//! The proposed map file: the map model written as YAML, and the check that rewriting the
//! file from the model would not drop anything the file has but the model does not know.

use anyhow::Result;
use rubato_protocol::SystemMap;
use serde_yaml_ng::Value;

const HEADER: &str = "# The team's system map. Every id here is one that plans, pins and the plan check may use.\n# This file was written by Rubato from a map draft: comments are not kept across a proposal.\n";

/// The map as the file a proposal commits.
pub fn render(map: &SystemMap) -> Result<String> {
    Ok(format!("{HEADER}{}", serde_yaml_ng::to_string(map)?))
}

const TOP: &[&str] = &["version", "name", "owner_rule", "repos", "nodes", "seams"];
const REPO: &[&str] = &["name", "plan_path"];
const NODE: &[&str] = &["id", "kind", "repo", "path", "description"];
const SEAM: &[&str] = &[
    "id",
    "from",
    "to",
    "carries",
    "owner",
    "guard",
    "description",
    "flows",
];
const FLOW: &[&str] = &["kind", "from", "to", "carries"];

fn extra(value: &Value, known: &[&str], at: &str, out: &mut Vec<String>) {
    if let Value::Mapping(m) = value {
        for key in m.keys().filter_map(Value::as_str) {
            if !known.contains(&key) {
                out.push(if at.is_empty() {
                    key.to_owned()
                } else {
                    format!("{at}.{key}")
                });
            }
        }
    }
}

fn each<'a>(parent: &'a Value, key: &str) -> impl Iterator<Item = (usize, &'a Value)> {
    parent
        .get(key)
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .enumerate()
}

/// Keys in the map file that the model does not have, as paths (`nodes[2].team`). A rewrite
/// from the model would silently drop them, so a proposal is refused while any exist.
pub fn unknown_keys(text: &str) -> Result<Vec<String>> {
    let doc: Value = serde_yaml_ng::from_str(text)?;
    let mut out = Vec::new();
    extra(&doc, TOP, "", &mut out);
    for (i, r) in each(&doc, "repos") {
        extra(r, REPO, &format!("repos[{i}]"), &mut out);
    }
    for (i, n) in each(&doc, "nodes") {
        extra(n, NODE, &format!("nodes[{i}]"), &mut out);
    }
    for (i, s) in each(&doc, "seams") {
        extra(s, SEAM, &format!("seams[{i}]"), &mut out);
        for (j, f) in each(s, "flows") {
            extra(f, FLOW, &format!("seams[{i}].flows[{j}]"), &mut out);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUBATO: &str = include_str!("../../../../docs/design/rubato-map.yml");
    const TEAM: &str = include_str!("../../../../e2e/fixtures/team-map.yml");
    const FLOWS: &str = include_str!("../../../../schema/fixtures/maps/flows.yml");

    #[test]
    fn round_trip_keeps_the_committed_maps() {
        for (name, text) in [
            ("rubato-map.yml", RUBATO),
            ("team-map.yml", TEAM),
            ("flows.yml", FLOWS),
        ] {
            let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
            let written = render(&map).unwrap();
            let again: SystemMap = serde_yaml_ng::from_str(&written).unwrap();
            assert_eq!(again, map, "{name}: parse, write, parse gives the same map");
            assert_eq!(again.validate(), Ok(()));
            // Writing it a second time changes nothing: the proposed file is stable.
            assert_eq!(render(&again).unwrap(), written, "{name}");
            // Nothing the committed file has is unknown to the model, and nothing the model
            // writes is unknown to the check.
            assert_eq!(unknown_keys(text).unwrap(), Vec::<String>::new(), "{name}");
            assert_eq!(
                unknown_keys(&written).unwrap(),
                Vec::<String>::new(),
                "{name}"
            );
        }
        println!("checked 3 map files");
    }

    #[test]
    fn keys_the_model_lacks_are_named_by_path() {
        let text = "version: 1\nname: x\nteam: payments\nnodes:\n  - {id: a, kind: context}\n  - {id: b, kind: context, on_call: alice}\nseams:\n  - id: a-b\n    from: a\n    to: b\n    owner: a\n    slo: 99.9\n    flows:\n      - {kind: data, from: a, to: b, carries: x, format: json}\nrepos:\n  - {name: o/r, default_branch: main}\n";
        assert_eq!(
            unknown_keys(text).unwrap(),
            [
                "team",
                "repos[0].default_branch",
                "nodes[1].on_call",
                "seams[0].slo",
                "seams[0].flows[0].format"
            ]
        );
        // The file still loads: unknown keys are ignored by the model, which is why a rewrite
        // would lose them.
        let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
        assert!(!render(&map).unwrap().contains("on_call"));
    }
}
