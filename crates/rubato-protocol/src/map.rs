//! The system map: contexts, contracts, externals and the seams between them (ADR 0007).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// `system-map.yml`. Node and seam ids are the only identifiers pins and plans may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemMap {
    /// Map format version. v0 is `1`.
    pub version: u32,
    /// Team or system name.
    pub name: String,
    /// Prose rule for who owns a seam. Informational.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_rule: Option<String>,
    /// Repositories behind the nodes, with per-repo overrides (ADR 0009).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<RepoRef>,
    pub nodes: Vec<MapNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seams: Vec<Seam>,
    /// Named journeys through the system, as ordered steps over declared flows (ADR 0024).
    /// Optional; a map without scenarios is still version 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<Scenario>,
}

/// A named journey: what crosses which seam, in order, when someone uses the system (ADR 0024).
/// Its id shares the namespace of node and seam ids but is not one a pin or plan may name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Scenario {
    pub id: String,
    /// Short title shown in the scenario picker.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// In order. Steps need not be contiguous: a journey may jump to another part.
    pub steps: Vec<ScenarioStep>,
}

/// One step: a flow declared on `seam`, picked by its kind and direction, and by `carries` when
/// the seam declares more than one flow with that kind and direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScenarioStep {
    pub seam: String,
    pub from: String,
    pub to: String,
    pub kind: FlowKind,
    /// Must equal the flow's `carries`; required only to tell two such flows apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carries: Option<String>,
    /// What happens at this step, in words, for the caption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub says: Option<String>,
}

/// A repository behind one or more nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RepoRef {
    /// `owner/name`.
    pub name: String,
    /// Plan path template with `{issue}` and `{slug}` placeholders. Default `docs/plans/{issue}.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_path: Option<String>,
}

impl RepoRef {
    pub const DEFAULT_PLAN_PATH: &'static str = "docs/plans/{issue}.md";

    /// Resolve the plan path for an issue number and slug.
    pub fn plan_path_for(&self, issue: u64, slug: &str) -> String {
        self.plan_path
            .as_deref()
            .unwrap_or(Self::DEFAULT_PLAN_PATH)
            .replace("{issue}", &issue.to_string())
            .replace("{slug}", slug)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Owned code with behaviour.
    Context,
    /// A format or schema others agree on.
    Contract,
    /// Something we stitch, not build.
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MapNode {
    pub id: String,
    pub kind: NodeKind,
    /// `owner/name` of the repository holding the code, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Path inside that repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A seam between two nodes. The owner side is woken when it breaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Seam {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carries: Option<String>,
    /// Node id of the owner side; must be `from` or `to`.
    pub owner: String,
    /// The check that guards this seam.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    /// What this seam is for, in a sentence a newcomer can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// What crosses this seam, one entry per thing and direction (ADR 0017). Optional: a seam
    /// without flows is drawn as a plain dependency from `from` to `to`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
}

/// Data moves; control tells the other side to do something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    Data,
    Control,
}

/// One thing crossing a seam in one direction. `from` and `to` are the seam's two nodes, in
/// either order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Flow {
    pub kind: FlowKind,
    pub from: String,
    pub to: String,
    /// What it carries: a type, a message, a command.
    pub carries: String,
}

impl Seam {
    /// Whether a flow from `from` to `to` runs along this seam: its two nodes, either way.
    pub fn joins(&self, from: &str, to: &str) -> bool {
        (from == self.from && to == self.to) || (from == self.to && to == self.from)
    }

    /// The declared flows a scenario step could mean: same kind and direction, and same
    /// `carries` when the step names one.
    pub fn flows_matching<'a>(&'a self, step: &'a ScenarioStep) -> impl Iterator<Item = &'a Flow> {
        self.flows.iter().filter(move |f| {
            f.kind == step.kind
                && f.from == step.from
                && f.to == step.to
                && step.carries.as_ref().is_none_or(|c| c == &f.carries)
        })
    }
}

impl FlowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowKind::Data => "data",
            FlowKind::Control => "control",
        }
    }
}

fn carrying(carries: &Option<String>) -> String {
    carries
        .as_ref()
        .map(|c| format!(" carrying {c:?}"))
        .unwrap_or_default()
}

/// A reference the map does not declare.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    #[error("duplicate id {0:?}")]
    DuplicateId(String),
    #[error("seam {seam:?} references unknown node {node:?}")]
    UnknownNode { seam: String, node: String },
    #[error("seam {seam:?} owner {owner:?} is neither its from nor its to side")]
    OwnerNotASide { seam: String, owner: String },
    #[error("seam {seam:?} has a flow from {from:?} to {to:?}, which are not its two nodes")]
    FlowOffSeam {
        seam: String,
        from: String,
        to: String,
    },
    #[error("scenario {0:?} has no steps")]
    EmptyScenario(String),
    #[error("scenario {scenario:?} step {step} names unknown seam {seam:?}")]
    ScenarioUnknownSeam {
        scenario: String,
        step: usize,
        seam: String,
    },
    #[error(
        "scenario {scenario:?} step {step} on seam {seam:?}: no declared {} flow from {from:?} to {to:?}{}",
        kind.as_str(),
        carrying(carries)
    )]
    ScenarioStepNoFlow {
        scenario: String,
        /// 1-based.
        step: usize,
        seam: String,
        kind: FlowKind,
        from: String,
        to: String,
        carries: Option<String>,
    },
    #[error(
        "scenario {scenario:?} step {step} on seam {seam:?}: {count} declared {} flows from {from:?} to {to:?}; name one with carries",
        kind.as_str()
    )]
    ScenarioStepAmbiguous {
        scenario: String,
        step: usize,
        seam: String,
        kind: FlowKind,
        from: String,
        to: String,
        count: usize,
    },
}

impl SystemMap {
    pub fn node(&self, id: &str) -> Option<&MapNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn seam(&self, id: &str) -> Option<&Seam> {
        self.seams.iter().find(|s| s.id == id)
    }

    pub fn scenario(&self, id: &str) -> Option<&Scenario> {
        self.scenarios.iter().find(|s| s.id == id)
    }

    /// The one declared flow a scenario step means, if it means exactly one.
    pub fn flow_for<'a>(&'a self, step: &'a ScenarioStep) -> Option<&'a Flow> {
        let mut found = self.seam(&step.seam)?.flows_matching(step);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Node ids only.
    pub fn node_ids(&self) -> BTreeSet<&str> {
        self.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    /// Seam ids only.
    pub fn seam_ids(&self) -> BTreeSet<&str> {
        self.seams.iter().map(|s| s.id.as_str()).collect()
    }

    /// Every repository the map names: `repos[]` and each node's `repo` (ADR 0019).
    pub fn repos_named(&self) -> BTreeSet<&str> {
        self.repos
            .iter()
            .map(|r| r.name.as_str())
            .chain(self.nodes.iter().filter_map(|n| n.repo.as_deref()))
            .collect()
    }

    /// Every id a plan or pin may reference: nodes and seams.
    pub fn ids(&self) -> BTreeSet<&str> {
        self.nodes
            .iter()
            .map(|n| n.id.as_str())
            .chain(self.seams.iter().map(|s| s.id.as_str()))
            .collect()
    }

    /// Structural checks beyond the schema: unique ids, seams between declared nodes, owner is a
    /// side, flows along their seam, every scenario step is exactly one declared flow.
    pub fn validate(&self) -> Result<(), Vec<MapError>> {
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();
        for id in self
            .nodes
            .iter()
            .map(|n| &n.id)
            .chain(self.seams.iter().map(|s| &s.id))
            .chain(self.scenarios.iter().map(|s| &s.id))
        {
            if !seen.insert(id.as_str()) {
                errors.push(MapError::DuplicateId(id.clone()));
            }
        }
        for s in &self.seams {
            for node in [&s.from, &s.to] {
                if self.node(node).is_none() {
                    errors.push(MapError::UnknownNode {
                        seam: s.id.clone(),
                        node: node.clone(),
                    });
                }
            }
            if s.owner != s.from && s.owner != s.to {
                errors.push(MapError::OwnerNotASide {
                    seam: s.id.clone(),
                    owner: s.owner.clone(),
                });
            }
            for f in &s.flows {
                if !s.joins(&f.from, &f.to) {
                    errors.push(MapError::FlowOffSeam {
                        seam: s.id.clone(),
                        from: f.from.clone(),
                        to: f.to.clone(),
                    });
                }
            }
        }
        for sc in &self.scenarios {
            if sc.steps.is_empty() {
                errors.push(MapError::EmptyScenario(sc.id.clone()));
            }
            for (i, step) in sc.steps.iter().enumerate() {
                let Some(seam) = self.seam(&step.seam) else {
                    errors.push(MapError::ScenarioUnknownSeam {
                        scenario: sc.id.clone(),
                        step: i + 1,
                        seam: step.seam.clone(),
                    });
                    continue;
                };
                match seam.flows_matching(step).count() {
                    1 => {}
                    0 => errors.push(MapError::ScenarioStepNoFlow {
                        scenario: sc.id.clone(),
                        step: i + 1,
                        seam: step.seam.clone(),
                        kind: step.kind,
                        from: step.from.clone(),
                        to: step.to.clone(),
                        carries: step.carries.clone(),
                    }),
                    count => errors.push(MapError::ScenarioStepAmbiguous {
                        scenario: sc.id.clone(),
                        step: i + 1,
                        seam: step.seam.clone(),
                        kind: step.kind,
                        from: step.from.clone(),
                        to: step.to.clone(),
                        count,
                    }),
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Plan path for an issue in a repo, falling back to the default when the repo is not listed.
    pub fn plan_path_for(&self, repo: &str, issue: u64, slug: &str) -> String {
        self.repos
            .iter()
            .find(|r| r.name == repo)
            .cloned()
            .unwrap_or(RepoRef {
                name: repo.to_owned(),
                plan_path: None,
            })
            .plan_path_for(issue, slug)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MapResponse {
    pub map: SystemMap,
    /// Git sha the map was loaded from.
    pub version: String,
    pub loaded_at: DateTime<Utc>,
}

/// Route paths for the map.
pub mod routes {
    pub const MAP: &str = "/api/map";
    pub const MAP_RELOAD: &str = "/api/map/reload";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_named_joins_the_repo_list_and_the_nodes() {
        let mut a = node("a");
        a.repo = Some("o/from-node".into());
        let m = SystemMap {
            version: 1,
            name: "t".into(),
            owner_rule: None,
            repos: vec![RepoRef {
                name: "o/listed".into(),
                plan_path: None,
            }],
            nodes: vec![a, node("b")],
            seams: vec![Seam {
                id: "a-b".into(),
                from: "a".into(),
                to: "b".into(),
                carries: None,
                owner: "a".into(),
                guard: None,
                description: None,
                flows: vec![],
            }],
            scenarios: vec![],
        };
        assert_eq!(
            m.repos_named().into_iter().collect::<Vec<_>>(),
            ["o/from-node", "o/listed"]
        );
        assert!(m.node_ids().contains("a") && !m.node_ids().contains("a-b"));
        assert!(m.seam_ids().contains("a-b") && !m.seam_ids().contains("a"));
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

    fn map(seams: Vec<Seam>) -> SystemMap {
        SystemMap {
            version: 1,
            name: "t".into(),
            owner_rule: None,
            repos: vec![RepoRef {
                name: "o/r".into(),
                plan_path: Some("features/{issue}-{slug}/plan.md".into()),
            }],
            nodes: vec![node("a"), node("b")],
            seams,
            scenarios: vec![],
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

    fn flow(kind: FlowKind, from: &str, to: &str) -> Flow {
        Flow {
            kind,
            from: from.into(),
            to: to.into(),
            carries: "x".into(),
        }
    }

    #[test]
    fn flow_ends_must_be_the_seams_nodes() {
        let mut s = seam("a-b", "a", "b", "b");
        s.flows = vec![
            flow(FlowKind::Data, "a", "b"),
            flow(FlowKind::Control, "b", "a"),
        ];
        assert_eq!(map(vec![s.clone()]).validate(), Ok(()), "either direction");
        s.flows = vec![
            flow(FlowKind::Data, "a", "b"),
            flow(FlowKind::Data, "a", "c"),
            flow(FlowKind::Data, "a", "a"),
        ];
        let errors = map(vec![s]).validate().unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors
                .iter()
                .all(|e| matches!(e, MapError::FlowOffSeam { seam, .. } if seam == "a-b"))
        );
        assert!(errors[0].to_string().contains("\"a-b\""), "{}", errors[0]);
    }

    #[test]
    fn map_without_flows_still_validates() {
        // A map file written before flows existed: no `flows`, no seam `description`.
        let yaml = "version: 1\nname: old\nnodes:\n  - {id: a, kind: context}\n  - {id: b, kind: context}\nseams:\n  - {id: a-b, from: a, to: b, owner: a}\n";
        let m: SystemMap = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(m.validate(), Ok(()));
        assert!(m.seams[0].flows.is_empty() && m.seams[0].description.is_none());
        // And it is written back without the new keys.
        let out = serde_json::to_string(&m).unwrap();
        assert!(
            !out.contains("flows") && !out.contains("description"),
            "{out}"
        );
        // The team's own map.
        let own: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../docs/design/rubato-map.yml")).unwrap();
        assert_eq!(own.validate(), Ok(()));
    }

    /// The scenario fixtures the TypeScript side reads too (plan-check "scenario fixtures").
    #[test]
    fn scenario_fixtures() {
        let good: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../schema/fixtures/maps/scenarios.yml"))
                .unwrap();
        assert_eq!(good.validate(), Ok(()));
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../schema/fixtures/maps/scenarios.json"))
                .unwrap();
        assert_eq!(serde_json::to_value(&good).unwrap(), expected);
        // Scenario ids are not ids a pin or plan may use.
        assert!(!good.ids().contains("pin-an-issue"));
        assert_eq!(
            good.scenario("pin-an-issue").map(|s| s.steps.len()),
            Some(4)
        );
        let bad: SystemMap = serde_yaml_ng::from_str(include_str!(
            "../../../schema/fixtures/maps/scenarios-bad.yml"
        ))
        .unwrap();
        let said: Vec<String> = bad
            .validate()
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            said,
            [
                "duplicate id \"web\"",
                "scenario \"off-flow\" step 1 on seam \"web-server\": no declared control flow from \"web\" to \"server\"",
                "scenario \"ambiguous\" step 1 on seam \"web-server\": 2 declared control flows from \"server\" to \"web\"; name one with carries",
                "scenario \"ambiguous\" step 2 on seam \"web-server\": no declared control flow from \"server\" to \"web\" carrying \"Nope\"",
                "scenario \"lost\" step 1 names unknown seam \"web-store\"",
                "scenario \"empty\" has no steps",
            ]
        );
        println!(
            "checked 2 scenario fixtures, {} scenarios, {} problems",
            good.scenarios.len() + bad.scenarios.len(),
            said.len()
        );
    }

    #[test]
    fn a_step_resolves_to_its_one_flow() {
        let good: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../schema/fixtures/maps/scenarios.yml"))
                .unwrap();
        let s = good.scenario("pin-an-issue").unwrap();
        let carried: Vec<&str> = s
            .steps
            .iter()
            .map(|step| good.flow_for(step).unwrap().carries.as_str())
            .collect();
        assert_eq!(
            carried,
            ["PinRequest", "rows", "WebEvent Changed", "Overview"]
        );
    }

    /// The fixtures the TypeScript side reads too (plan-check "map fixtures").
    #[test]
    fn map_fixtures() {
        let good: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../schema/fixtures/maps/flows.yml"))
                .unwrap();
        assert_eq!(good.validate(), Ok(()));
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../schema/fixtures/maps/flows.json")).unwrap();
        assert_eq!(serde_json::to_value(&good).unwrap(), expected);
        let bad: SystemMap = serde_yaml_ng::from_str(include_str!(
            "../../../schema/fixtures/maps/flows-off-seam.yml"
        ))
        .unwrap();
        let errors = bad.validate().unwrap_err();
        let said: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "seam \"web-server\" has a flow from \"server\" to \"store\", which are not its two nodes",
                "seam \"web-server\" has a flow from \"web\" to \"web\", which are not its two nodes",
            ]
        );
        println!(
            "checked 2 map fixtures, {} flows",
            good.seams[0].flows.len() + 3
        );
    }

    #[test]
    fn valid_map_passes() {
        assert_eq!(map(vec![seam("a-b", "a", "b", "b")]).validate(), Ok(()));
    }

    #[test]
    fn reports_every_structural_error() {
        let m = map(vec![
            seam("a-b", "a", "b", "zzz"),
            seam("a", "a", "nope", "a"),
        ]);
        let errs = m.validate().unwrap_err();
        assert_eq!(errs.len(), 3, "{errs:?}");
        assert!(errs.contains(&MapError::DuplicateId("a".into())));
        assert!(errs.contains(&MapError::UnknownNode {
            seam: "a".into(),
            node: "nope".into()
        }));
        assert!(errs.contains(&MapError::OwnerNotASide {
            seam: "a-b".into(),
            owner: "zzz".into()
        }));
    }

    #[test]
    fn plan_path_override_and_default() {
        let m = map(vec![]);
        assert_eq!(
            m.plan_path_for("o/r", 4, "companion"),
            "features/4-companion/plan.md"
        );
        assert_eq!(m.plan_path_for("o/other", 4, "x"), "docs/plans/4.md");
    }
}
