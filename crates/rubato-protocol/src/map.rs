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
}

impl SystemMap {
    pub fn node(&self, id: &str) -> Option<&MapNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn seam(&self, id: &str) -> Option<&Seam> {
        self.seams.iter().find(|s| s.id == id)
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

    /// Structural checks beyond the schema: unique ids, seams between declared nodes, owner is a side.
    pub fn validate(&self) -> Result<(), Vec<MapError>> {
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();
        for id in self
            .nodes
            .iter()
            .map(|n| &n.id)
            .chain(self.seams.iter().map(|s| &s.id))
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
            }],
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
        }
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
