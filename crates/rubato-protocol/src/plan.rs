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

/// Why a plan does not fit the map (ADR 0029). Each one alone makes `PlanFit::fits` false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FitReason {
    /// The plan file does not parse.
    ParseError { message: String },
    /// Node or seam ids the map does not declare.
    UnknownIds { ids: Vec<String> },
    /// A flow's seam does not join the flow's two parts.
    FlowOffSeam {
        from: String,
        to: String,
        seam: String,
    },
    /// A seam in Affected parts with no O-row.
    MissingORow { seam: String },
    /// A data model on a part that is not a row of Affected parts (ADR 0031).
    ModelOffPart { model: String, part: String },
    /// A data model crossing a seam no row of Affected parts touches (ADR 0031).
    ModelOffSeam { model: String, seam: String },
}

/// Where a plan fits the map, and whether it does (ADR 0029). An approval counts only while
/// `fits` is true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanFit {
    pub fits: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<FitReason>,
}

/// The fit of a plan as read, or of a plan file that did not parse (ADR 0029). Pure: the server
/// and the web read the same answer. `None` when there is neither a plan nor a parse error.
pub fn plan_fit(
    plan: Option<&Plan>,
    parse_error: Option<&str>,
    map: &crate::map::SystemMap,
) -> Option<PlanFit> {
    let mut reasons = Vec::new();
    match (plan, parse_error) {
        (_, Some(message)) => reasons.push(FitReason::ParseError {
            message: message.to_owned(),
        }),
        (Some(plan), None) => {
            let unknown = plan.unknown_ids(map);
            if !unknown.is_empty() {
                reasons.push(FitReason::UnknownIds {
                    ids: unknown.into_iter().map(str::to_owned).collect(),
                });
            }
            reasons.extend(
                plan.flows_off_seam(map)
                    .into_iter()
                    .map(|f| FitReason::FlowOffSeam {
                        from: f.from.clone(),
                        to: f.to.clone(),
                        seam: f.seam.clone(),
                    }),
            );
            reasons.extend(
                plan.seams_without_o_row()
                    .into_iter()
                    .map(|s| FitReason::MissingORow { seam: s.to_owned() }),
            );
            reasons.extend(
                plan.models_off_part()
                    .into_iter()
                    .map(|m| FitReason::ModelOffPart {
                        model: m.model.clone(),
                        part: m.part.clone(),
                    }),
            );
            reasons.extend(
                plan.models_off_seam()
                    .into_iter()
                    .map(|m| FitReason::ModelOffSeam {
                        model: m.model.clone(),
                        seam: m.crosses.clone().unwrap_or_default(),
                    }),
            );
        }
        (None, None) => return None,
    }
    Some(PlanFit {
        fits: reasons.is_empty(),
        reasons,
    })
}

#[cfg(test)]
mod fit_tests {
    use super::*;
    use crate::map::SystemMap;

    fn map() -> SystemMap {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        serde_yaml_ng::from_str(
            &std::fs::read_to_string(root.join("docs/design/rubato-map.yml")).unwrap(),
        )
        .unwrap()
    }

    fn plan() -> Plan {
        Plan {
            issue: IssueRef::new("m0x6A/rubato", 80),
            title: None,
            path: "docs/plans/80.md".into(),
            git_ref: None,
            spend_cap_usd: None,
            attempt_limit: None,
            affected_parts: vec![
                AffectedPart {
                    node: "rubato-server".into(),
                    status: PartStatus::Affected,
                    change: "x".into(),
                    seams: vec!["web-server".into()],
                },
                AffectedPart {
                    node: "web".into(),
                    status: PartStatus::Affected,
                    change: "y".into(),
                    seams: vec!["web-server".into()],
                },
            ],
            o_rows: vec![ORow {
                seam: "web-server".into(),
                breaks_like: None,
                signal: "s".into(),
                panel: "p".into(),
                owner: "rubato-server".into(),
            }],
            assumed: vec![],
            flows: vec![PlanFlow {
                from: "rubato-server".into(),
                to: "web".into(),
                seam: "web-server".into(),
                kind: FlowKind::Data,
                carries: "c".into(),
            }],
            data_models: vec![DataModel {
                model: "user_tokens".into(),
                kind: DataModelKind::Table,
                part: "rubato-server".into(),
                change: ModelChange::New,
                crosses: Some("web-server".into()),
                note: None,
            }],
        }
    }

    fn reasons(p: &Plan) -> Vec<FitReason> {
        plan_fit(Some(p), None, &map()).unwrap().reasons
    }

    #[test]
    fn a_plan_that_names_the_map_fits() {
        let fit = plan_fit(Some(&plan()), None, &map()).unwrap();
        assert!(fit.fits, "{:?}", fit.reasons);
        assert_eq!(plan_fit(None, None, &map()), None);
    }

    #[test]
    fn each_reason_on_its_own() {
        let mut checked = 0;
        let fit = plan_fit(Some(&plan()), Some("bad table"), &map()).unwrap();
        assert!(!fit.fits);
        assert_eq!(
            fit.reasons,
            [FitReason::ParseError {
                message: "bad table".into()
            }]
        );
        checked += 1;

        let mut p = plan();
        p.affected_parts[1].node = "web-nowhere".into();
        assert!(
            matches!(&reasons(&p)[0], FitReason::UnknownIds { ids } if ids == &["web-nowhere"])
        );
        checked += 1;

        let mut p = plan();
        p.flows[0].to = "github".into();
        assert_eq!(
            reasons(&p),
            [FitReason::FlowOffSeam {
                from: "rubato-server".into(),
                to: "github".into(),
                seam: "web-server".into()
            }]
        );
        checked += 1;

        let mut p = plan();
        p.o_rows.clear();
        assert_eq!(
            reasons(&p),
            [FitReason::MissingORow {
                seam: "web-server".into()
            }]
        );
        checked += 1;

        let mut p = plan();
        p.data_models[0].part = "rubato-companion".into();
        assert_eq!(
            reasons(&p),
            [FitReason::ModelOffPart {
                model: "user_tokens".into(),
                part: "rubato-companion".into()
            }]
        );
        checked += 1;

        let mut p = plan();
        p.data_models[0].crosses = Some("server-github".into());
        assert_eq!(
            reasons(&p),
            [FitReason::ModelOffSeam {
                model: "user_tokens".into(),
                seam: "server-github".into()
            }]
        );
        checked += 1;
        println!("checked {checked} fit reasons");
    }
}
