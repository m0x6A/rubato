//! The plan file's machine-readable parts (ADR 0005, ADR 0009).
//!
//! A plan is Markdown on the issue branch. Its frontmatter carries `issue`, `spend_cap_usd`
//! and `attempt_limit`; the `## Affected parts` and `## O-rows` tables are parsed into the
//! types below and validated against `schema/plan.schema.json`.

use crate::ids::IssueRef;
use crate::map::FlowKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub issue: IssueRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Path of the plan file inside the repository.
    pub path: String,
    /// Git ref the plan was read from (branch name or commit sha).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_cap_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_limit: Option<u32>,
    pub affected_parts: Vec<AffectedPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub o_rows: Vec<ORow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumed: Vec<Assumption>,
    /// What the plan will send across seams, from its `## Flows` table (ADR 0021). Optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<PlanFlow>,
    /// The tables, types, schemas, events and files the change introduces, changes or removes,
    /// from its `## Data models` table (ADR 0031). Optional; empty when absent or "None."
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_models: Vec<DataModel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataModelKind {
    Table,
    Type,
    Schema,
    Event,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelChange {
    New,
    Changed,
    Removed,
}

/// One row of the plan's Data models table (ADR 0031): a model on the part that owns it, and the
/// seam it travels over, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataModel {
    /// The model's name or names, as written, backticks removed.
    pub model: String,
    pub kind: DataModelKind,
    /// Node id of the part that owns the model. Must be in Affected parts.
    pub part: String,
    pub change: ModelChange,
    /// Seam id the model crosses. Must be touched in Affected parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosses: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One row of the plan's Flows table: something the change sends from one part to another over
/// a seam. The seam must join the two parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanFlow {
    pub from: String,
    pub to: String,
    pub seam: String,
    pub kind: FlowKind,
    pub carries: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PartStatus {
    Affected,
    New,
    Quiet,
}

/// One row of the affected-parts table. This is what paints the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AffectedPart {
    /// Node id from the map.
    pub node: String,
    pub status: PartStatus,
    /// What changes, in one line.
    pub change: String,
    /// Seam ids this change touches. Each needs an O-row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seams: Vec<String>,
}

/// The 3 am question for one seam: which signal leads back to it, where, and who is woken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ORow {
    pub seam: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breaks_like: Option<String>,
    /// Span, metric or log line, with the `rubato.*` attributes it carries.
    pub signal: String,
    /// Dashboard panel the signal appears on.
    pub panel: String,
    /// Node id of the side that owns the fix.
    pub owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssumptionScope {
    /// Inside one node: assumed and labelled.
    Node,
    /// On a seam: blocks until a human answers.
    Seam,
}

/// A decision the plan had to make that nothing settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Assumption {
    pub id: String,
    pub decision: String,
    pub chosen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternatives: Option<String>,
    pub scope: AssumptionScope,
}

impl Plan {
    /// Seam ids named in affected parts that have no O-row. Empty means the 3 am question is answered.
    pub fn seams_without_o_row(&self) -> Vec<&str> {
        let mut missing: Vec<&str> = self
            .affected_parts
            .iter()
            .flat_map(|p| p.seams.iter().map(String::as_str))
            .filter(|s| !self.o_rows.iter().any(|o| o.seam == *s))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        missing
    }

    /// Every node and seam id the plan references.
    pub fn referenced_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .affected_parts
            .iter()
            .flat_map(|p| {
                std::iter::once(p.node.as_str()).chain(p.seams.iter().map(String::as_str))
            })
            .chain(
                self.o_rows
                    .iter()
                    .flat_map(|o| [o.seam.as_str(), o.owner.as_str()]),
            )
            .chain(
                self.flows
                    .iter()
                    .flat_map(|f| [f.from.as_str(), f.to.as_str(), f.seam.as_str()]),
            )
            .chain(
                self.data_models
                    .iter()
                    .flat_map(|m| std::iter::once(m.part.as_str()).chain(m.crosses.as_deref())),
            )
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Flows whose seam the map has but which does not join the flow's two parts. A flow that
    /// names an unknown id is reported by `unknown_ids`, not here.
    pub fn flows_off_seam<'a>(&'a self, map: &crate::map::SystemMap) -> Vec<&'a PlanFlow> {
        self.flows
            .iter()
            .filter(|f| map.seam(&f.seam).is_some_and(|s| !s.joins(&f.from, &f.to)))
            .collect()
    }

    /// Data models whose part is not a row of Affected parts (ADR 0031).
    pub fn models_off_part(&self) -> Vec<&DataModel> {
        self.data_models
            .iter()
            .filter(|m| !self.affected_parts.iter().any(|p| p.node == m.part))
            .collect()
    }

    /// Data models that cross a seam no row of Affected parts touches (ADR 0031).
    pub fn models_off_seam(&self) -> Vec<&DataModel> {
        self.data_models
            .iter()
            .filter(|m| {
                m.crosses
                    .as_ref()
                    .is_some_and(|s| !self.affected_parts.iter().any(|p| p.seams.contains(s)))
            })
            .collect()
    }

    /// Ids the plan references that the map does not declare.
    pub fn unknown_ids<'a>(&'a self, map: &crate::map::SystemMap) -> Vec<&'a str> {
        let known = map.ids();
        self.referenced_ids()
            .into_iter()
            .filter(|id| !known.contains(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            issue: IssueRef::new("o/r", 1),
            title: None,
            path: "docs/plans/1.md".into(),
            git_ref: None,
            spend_cap_usd: Some(25.0),
            attempt_limit: Some(3),
            affected_parts: vec![AffectedPart {
                node: "rubato-protocol".into(),
                status: PartStatus::New,
                change: "types".into(),
                seams: vec!["companion-server".into(), "web-server".into()],
            }],
            o_rows: vec![ORow {
                seam: "companion-server".into(),
                breaks_like: None,
                signal: "rubato.events.dropped".into(),
                panel: "Companions".into(),
                owner: "rubato-server".into(),
            }],
            assumed: vec![],
            flows: vec![],
            data_models: vec![],
        }
    }

    #[test]
    fn finds_seam_without_o_row() {
        assert_eq!(plan().seams_without_o_row(), vec!["web-server"]);
    }

    #[test]
    fn referenced_ids_are_sorted_and_unique() {
        assert_eq!(
            plan().referenced_ids(),
            vec![
                "companion-server",
                "rubato-protocol",
                "rubato-server",
                "web-server"
            ]
        );
    }
}
