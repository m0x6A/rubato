//! Apply draft operations to a map. Pure: the rules of what an operation may do (design S19).

use rubato_protocol::SystemMap;
use rubato_protocol::map_draft::{DraftOp, MapOp};

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// The map with `op` applied, or why it cannot be applied. Nothing is changed on refusal.
pub fn apply(map: &SystemMap, op: &MapOp) -> Result<SystemMap, String> {
    let mut next = map.clone();
    let taken = |id: &str| map.ids().contains(id) || map.scenario(id).is_some();
    match op {
        MapOp::AddNode { node } => {
            if !valid_id(&node.id) {
                return Err(format!(
                    "{:?} is not an id: use lower-case letters, digits and hyphens",
                    node.id
                ));
            }
            if taken(&node.id) {
                return Err(format!("id {} already exists", node.id));
            }
            next.nodes.push(node.clone());
        }
        MapOp::UpdateNode { node } => {
            let Some(at) = next.nodes.iter().position(|n| n.id == node.id) else {
                return Err(format!("there is no part {} to change", node.id));
            };
            next.nodes[at] = node.clone();
        }
        MapOp::RemoveNode { id } => {
            let Some(at) = next.nodes.iter().position(|n| &n.id == id) else {
                return Err(format!("there is no part {id} to remove"));
            };
            let joined: Vec<&str> = map
                .seams
                .iter()
                .filter(|s| &s.from == id || &s.to == id)
                .map(|s| s.id.as_str())
                .collect();
            if !joined.is_empty() {
                return Err(format!(
                    "part {id} is still an end of {}: remove or change those seams first",
                    joined.join(", ")
                ));
            }
            next.nodes.remove(at);
        }
        MapOp::AddSeam { seam } => {
            if !valid_id(&seam.id) {
                return Err(format!(
                    "{:?} is not an id: use lower-case letters, digits and hyphens",
                    seam.id
                ));
            }
            if taken(&seam.id) {
                return Err(format!("id {} already exists", seam.id));
            }
            next.seams.push(seam.clone());
        }
        MapOp::UpdateSeam { seam } => {
            let Some(at) = next.seams.iter().position(|s| s.id == seam.id) else {
                return Err(format!("there is no seam {} to change", seam.id));
            };
            next.seams[at] = seam.clone();
        }
        MapOp::RemoveSeam { id } => {
            let Some(at) = next.seams.iter().position(|s| &s.id == id) else {
                return Err(format!("there is no seam {id} to remove"));
            };
            next.seams.remove(at);
        }
    }
    // The same structural rules a loaded map must meet: seam ends exist, the owner is an end,
    // every flow runs along its seam.
    next.validate().map_err(|errors| {
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    Ok(next)
}

/// Apply the draft to `base` in order. An operation that no longer applies is marked stale with
/// its reason and skipped, so the rest still show.
pub fn replay(base: &SystemMap, ops: &mut [DraftOp]) -> SystemMap {
    let mut map = base.clone();
    for o in ops.iter_mut() {
        match apply(&map, &o.op) {
            Ok(next) => {
                map = next;
                o.stale = None;
            }
            Err(why) => o.stale = Some(why),
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::{Flow, FlowKind, MapNode, NodeKind, Seam};

    fn base() -> SystemMap {
        serde_yaml_ng::from_str(include_str!("../../../../schema/fixtures/maps/flows.yml")).unwrap()
    }

    fn node(id: &str) -> MapNode {
        MapNode {
            id: id.into(),
            kind: NodeKind::Context,
            repo: None,
            path: None,
            description: None,
        }
    }

    fn seam(id: &str, from: &str, to: &str, owner: &str) -> Seam {
        Seam {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            carries: None,
            owner: owner.into(),
            guard: None,
            description: None,
            flows: vec![],
        }
    }

    #[test]
    fn invalid_op_is_rejected_with_reason() {
        let map = base();
        let mut off_seam = seam("web-billing", "web", "server", "web");
        off_seam.flows = vec![Flow {
            kind: FlowKind::Data,
            from: "web".into(),
            to: "store".into(),
            carries: "x".into(),
        }];
        // One case per rule: the operation and a word its reason must carry.
        let cases: Vec<(MapOp, &str)> = vec![
            (MapOp::AddNode { node: node("web") }, "already exists"),
            (
                MapOp::AddNode {
                    node: node("web-server"),
                },
                "already exists",
            ),
            (
                MapOp::AddNode {
                    node: node("Not An Id"),
                },
                "is not an id",
            ),
            (MapOp::AddNode { node: node("") }, "is not an id"),
            (MapOp::UpdateNode { node: node("nope") }, "no part nope"),
            (MapOp::RemoveNode { id: "nope".into() }, "no part nope"),
            (
                MapOp::RemoveNode {
                    id: "server".into(),
                },
                "still an end of web-server, server-store",
            ),
            (
                MapOp::AddSeam {
                    seam: seam("web-server", "web", "server", "web"),
                },
                "already exists",
            ),
            (
                MapOp::AddSeam {
                    seam: seam("web-nowhere", "web", "nowhere", "web"),
                },
                "unknown node \"nowhere\"",
            ),
            (
                MapOp::AddSeam {
                    seam: seam("web-store", "web", "store", "server"),
                },
                "neither its from nor its to",
            ),
            (MapOp::AddSeam { seam: off_seam }, "not its two nodes"),
            (
                MapOp::UpdateSeam {
                    seam: seam("nope", "web", "server", "web"),
                },
                "no seam nope",
            ),
            (
                MapOp::UpdateSeam {
                    seam: seam("web-server", "web", "gone", "web"),
                },
                "unknown node \"gone\"",
            ),
            (MapOp::RemoveSeam { id: "nope".into() }, "no seam nope"),
        ];
        for (op, word) in &cases {
            let why = apply(&map, op).expect_err(&format!("{op:?} must be refused"));
            assert!(why.contains(word), "{op:?}: {why}");
        }
        println!("checked {} refused operations", cases.len());
    }

    #[test]
    fn a_draft_cannot_pull_a_seam_or_flow_out_from_under_a_scenario() {
        let map: SystemMap = serde_yaml_ng::from_str(include_str!(
            "../../../../schema/fixtures/maps/scenarios.yml"
        ))
        .unwrap();
        // Removing a seam a scenario steps over: refused, naming the scenario.
        let e = apply(
            &map,
            &MapOp::RemoveSeam {
                id: "server-store".into(),
            },
        )
        .unwrap_err();
        assert!(
            e.contains("scenario \"pin-an-issue\" step 2 names unknown seam \"server-store\""),
            "{e}"
        );
        // Dropping the flow a step uses: refused the same way.
        let mut fewer = map.seam("web-server").unwrap().clone();
        fewer.flows.retain(|f| f.carries != "PinRequest");
        let e = apply(&map, &MapOp::UpdateSeam { seam: fewer }).unwrap_err();
        assert!(e.contains("scenario \"pin-an-issue\" step 1"), "{e}");
        // A scenario id is taken: a part cannot reuse it.
        let e = apply(
            &map,
            &MapOp::AddNode {
                node: node("pin-an-issue"),
            },
        )
        .unwrap_err();
        assert!(e.contains("already exists"), "{e}");
        // Ops that leave the scenarios alone still apply, and keep them.
        let next = apply(
            &map,
            &MapOp::AddNode {
                node: node("billing"),
            },
        )
        .unwrap();
        assert_eq!(next.scenarios, map.scenarios);
    }

    #[test]
    fn valid_ops_apply_in_order_and_a_stale_one_is_skipped() {
        let map = base();
        let op = |seq: u64, op: MapOp| DraftOp {
            seq,
            op_id: uuid::Uuid::new_v4(),
            op,
            author: "m0x6A".into(),
            created_at: chrono::Utc::now(),
            stale: None,
        };
        let mut described = node("store");
        described.kind = NodeKind::External;
        described.description = Some("Where rows live.".into());
        let mut ops = vec![
            op(
                1,
                MapOp::AddNode {
                    node: node("billing"),
                },
            ),
            op(
                2,
                MapOp::AddSeam {
                    seam: seam("web-billing", "web", "billing", "billing"),
                },
            ),
            op(
                3,
                MapOp::UpdateNode {
                    node: described.clone(),
                },
            ),
            op(
                4,
                MapOp::RemoveSeam {
                    id: "server-store".into(),
                },
            ),
            op(5, MapOp::RemoveNode { id: "store".into() }),
        ];
        let preview = replay(&map, &mut ops);
        assert!(ops.iter().all(|o| o.stale.is_none()), "{ops:?}");
        assert!(preview.node("billing").is_some());
        assert_eq!(preview.seam("web-billing").unwrap().owner, "billing");
        assert!(preview.node("store").is_none() && preview.seam("server-store").is_none());
        assert_eq!(preview.validate(), Ok(()));

        // Without the operation that added the part, the seam that needs it no longer applies;
        // what does not depend on it still does.
        let mut without_first: Vec<DraftOp> = ops[1..].to_vec();
        let preview = replay(&map, &mut without_first);
        assert!(
            without_first[0]
                .stale
                .as_deref()
                .unwrap()
                .contains("billing")
        );
        assert!(without_first[1..].iter().all(|o| o.stale.is_none()));
        assert!(preview.seam("web-billing").is_none());
        assert!(preview.node("store").is_none());
    }
}
