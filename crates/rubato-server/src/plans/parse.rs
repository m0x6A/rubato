//! Parse a plan Markdown file into `Plan` (ADR 0009). Must produce exactly what
//! `actions/plan-check/src/parse-plan.ts` produces; `schema/fixtures/plans/valid.json` is the
//! shared contract (seam `server-plan-file`).

use rubato_protocol::{
    AffectedPart, Assumption, AssumptionScope, DataModel, DataModelKind, FlowKind, IssueRef,
    ModelChange, ORow, PartStatus, Plan, PlanFlow,
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PlanParseError {
    #[error("frontmatter is not closed")]
    UnclosedFrontmatter,
    #[error("frontmatter needs issue: owner/name#number")]
    MissingIssue,
    #[error("issue must be owner/name#number, got {0:?}")]
    BadIssue(String),
    #[error("no table under an 'Affected parts' heading")]
    NoAffectedParts,
    #[error("{section}: missing column {column} (headers: {headers})")]
    MissingColumn {
        section: &'static str,
        column: &'static str,
        headers: String,
    },
    #[error("table after heading {0} has no header separator row")]
    NoSeparator(&'static str),
    #[error("flows: kind must be data or control, got {0:?}")]
    BadFlowKind(String),
    #[error("data models: kind must be table, type, schema, event or file, got {0:?}")]
    BadModelKind(String),
    #[error("data models: change must be new, changed or removed, got {0:?}")]
    BadModelChange(String),
}

struct Frontmatter<'a> {
    body: &'a str,
    data: Vec<(String, String)>,
}

fn split_frontmatter(text: &str) -> Result<Frontmatter<'_>, PlanParseError> {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Ok(Frontmatter {
            body: text,
            data: vec![],
        });
    };
    if first.trim() != "---" {
        return Ok(Frontmatter {
            body: text,
            data: vec![],
        });
    }
    let mut offset = first.len();
    let mut data = Vec::new();
    for line in lines {
        offset += line.len();
        if line.trim() == "---" {
            return Ok(Frontmatter {
                body: &text[offset..],
                data,
            });
        }
        let content = line.split_once(" #").map(|(a, _)| a).unwrap_or(line).trim();
        if let Some((k, v)) = content.split_once(':') {
            data.push((
                k.trim().to_owned(),
                v.trim().trim_matches('"').trim_matches('\'').to_owned(),
            ));
        }
    }
    Err(PlanParseError::UnclosedFrontmatter)
}

pub fn parse_issue_ref(text: &str) -> Result<IssueRef, PlanParseError> {
    let t = text.trim();
    let Some((repo, num)) = t.rsplit_once('#') else {
        return Err(PlanParseError::BadIssue(text.to_owned()));
    };
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts
            .iter()
            .any(|p| p.is_empty() || p.contains(char::is_whitespace))
    {
        return Err(PlanParseError::BadIssue(text.to_owned()));
    }
    let number = num
        .parse()
        .map_err(|_| PlanParseError::BadIssue(text.to_owned()))?;
    Ok(IssueRef::new(repo, number))
}

/// Split a table row on `|` outside backticks.
fn split_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    for ch in line.chars() {
        if ch == '`' {
            in_code = !in_code;
        }
        if ch == '|' && !in_code {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    cells.push(cur);
    if cells.first().is_some_and(|c| c.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|c| c.trim().is_empty()) {
        cells.pop();
    }
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

/// Heading text with `#`s and a leading `1.` / `1.2` numbering removed, lowercased.
fn heading_text(line: &str) -> String {
    let rest = line.trim_start_matches('#').trim();
    let mut parts = rest.splitn(2, ' ');
    let first = parts.next().unwrap_or("");
    let numbered = !first.is_empty()
        && first
            .trim_end_matches('.')
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.');
    if numbered {
        parts.next().unwrap_or("").trim().to_ascii_lowercase()
    } else {
        rest.to_ascii_lowercase()
    }
}

pub fn table_after_heading(
    body: &str,
    prefix: &str,
    name: &'static str,
) -> Result<Option<Table>, PlanParseError> {
    let lines: Vec<&str> = body.lines().collect();
    let Some(start) = lines
        .iter()
        .position(|l| is_heading(l) && heading_text(l).starts_with(prefix))
    else {
        return Ok(None);
    };
    let mut i = start + 1;
    while i < lines.len() && !is_heading(lines[i]) && !lines[i].trim_start().starts_with('|') {
        i += 1;
    }
    if i >= lines.len() || is_heading(lines[i]) {
        return Ok(None);
    }
    let mut table_lines = Vec::new();
    while i < lines.len() && lines[i].trim_start().starts_with('|') {
        table_lines.push(lines[i].trim());
        i += 1;
    }
    let (Some(header), Some(sep)) = (table_lines.first(), table_lines.get(1)) else {
        return Err(PlanParseError::NoSeparator(name));
    };
    let sep_ok = sep
        .trim_start_matches('|')
        .trim_start()
        .trim_start_matches(':')
        .starts_with("---");
    if !sep_ok {
        return Err(PlanParseError::NoSeparator(name));
    }
    let headers = split_row(header)
        .into_iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();
    let rows = table_lines[2..]
        .iter()
        .map(|l| split_row(l))
        .filter(|r| r.iter().any(|c| !c.is_empty()))
        .collect();
    Ok(Some(Table { headers, rows }))
}

fn column(headers: &[String], prefixes: &[&str]) -> Option<usize> {
    prefixes
        .iter()
        .find_map(|p| headers.iter().position(|h| h.starts_with(p)))
}

fn require(
    table: &Table,
    section: &'static str,
    column_name: &'static str,
    prefixes: &[&str],
) -> Result<usize, PlanParseError> {
    column(&table.headers, prefixes).ok_or_else(|| PlanParseError::MissingColumn {
        section,
        column: column_name,
        headers: table.headers.join(", "),
    })
}

pub fn clean_id(cell: &str) -> String {
    cell.chars()
        .filter(|c| !matches!(c, '`' | '*' | '_'))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn id_list(cell: &str) -> Vec<String> {
    cell.split(',')
        .map(clean_id)
        .filter(|s| !s.is_empty() && s != "—" && s != "-")
        .collect()
}

fn first_word(cell: &str) -> String {
    clean_id(cell)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn cell(row: &[String], i: usize) -> String {
    row.get(i).cloned().unwrap_or_default()
}

fn opt_cell(row: &[String], i: Option<usize>) -> Option<String> {
    i.and_then(|i| row.get(i))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub fn parse_plan(text: &str, path: &str) -> Result<Plan, PlanParseError> {
    let fm = split_frontmatter(text)?;
    let get = |k: &str| {
        fm.data
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    let issue = parse_issue_ref(get("issue").ok_or(PlanParseError::MissingIssue)?)?;
    let mut plan = Plan {
        issue,
        title: get("title")
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned),
        path: path.to_owned(),
        git_ref: None,
        spend_cap_usd: get("spend_cap_usd").and_then(|v| v.parse().ok()),
        attempt_limit: get("attempt_limit").and_then(|v| v.parse().ok()),
        affected_parts: vec![],
        o_rows: vec![],
        assumed: vec![],
        flows: vec![],
        data_models: vec![],
    };

    let parts = table_after_heading(fm.body, "affected parts", "affected parts")?
        .ok_or(PlanParseError::NoAffectedParts)?;
    let node = require(&parts, "affected parts", "node", &["node"])?;
    let status = require(&parts, "affected parts", "status", &["status"])?;
    let change = require(
        &parts,
        "affected parts",
        "change",
        &["what changes", "change"],
    )?;
    let seams = column(&parts.headers, &["seam"]);
    for row in &parts.rows {
        let st = match first_word(&cell(row, status)).as_str() {
            "affected" => PartStatus::Affected,
            "new" => PartStatus::New,
            "quiet" => PartStatus::Quiet,
            // Keep going; the schema check reports it as the TS side does.
            _ => PartStatus::Affected,
        };
        plan.affected_parts.push(AffectedPart {
            node: clean_id(&cell(row, node)),
            status: st,
            change: cell(row, change),
            seams: seams.map(|i| id_list(&cell(row, i))).unwrap_or_default(),
        });
    }

    if let Some(t) = table_after_heading(fm.body, "o-rows", "o-rows")? {
        let seam = require(&t, "o-rows", "seam", &["seam"])?;
        let breaks = column(&t.headers, &["breaks"]);
        let signal = require(&t, "o-rows", "signal", &["signal"])?;
        let panel = require(&t, "o-rows", "panel", &["panel"])?;
        let owner = require(&t, "o-rows", "owner", &["owner"])?;
        for row in &t.rows {
            plan.o_rows.push(ORow {
                seam: clean_id(&cell(row, seam)),
                breaks_like: opt_cell(row, breaks),
                signal: cell(row, signal),
                panel: cell(row, panel),
                owner: clean_id(&cell(row, owner)),
            });
        }
    }

    if let Some(t) = table_after_heading(fm.body, "assumed, not yet approved", "assumed")? {
        let id = t
            .headers
            .iter()
            .position(|h| h == "#")
            .or_else(|| column(&t.headers, &["id"]))
            .ok_or_else(|| PlanParseError::MissingColumn {
                section: "assumed",
                column: "id",
                headers: t.headers.join(", "),
            })?;
        let decision = require(&t, "assumed", "decision", &["decision"])?;
        let chosen = require(&t, "assumed", "chosen", &["chosen"])?;
        let alternatives = column(&t.headers, &["alternatives"]);
        let scope = require(&t, "assumed", "scope", &["scope"])?;
        for row in &t.rows {
            let sc = if first_word(&cell(row, scope)) == "seam" {
                AssumptionScope::Seam
            } else {
                AssumptionScope::Node
            };
            plan.assumed.push(Assumption {
                id: clean_id(&cell(row, id)),
                decision: cell(row, decision),
                chosen: cell(row, chosen),
                alternatives: opt_cell(row, alternatives),
                scope: sc,
            });
        }
    }
    if let Some(t) = table_after_heading(fm.body, "flows", "flows")? {
        let from = require(&t, "flows", "from", &["from"])?;
        let to = require(&t, "flows", "to", &["to"])?;
        let seam = require(&t, "flows", "seam", &["seam"])?;
        let kind = require(&t, "flows", "kind", &["kind"])?;
        let carries = require(&t, "flows", "carries", &["carries"])?;
        for row in &t.rows {
            let k = match first_word(&cell(row, kind)).as_str() {
                "data" => FlowKind::Data,
                "control" => FlowKind::Control,
                other => return Err(PlanParseError::BadFlowKind(other.to_owned())),
            };
            plan.flows.push(PlanFlow {
                from: clean_id(&cell(row, from)),
                to: clean_id(&cell(row, to)),
                seam: clean_id(&cell(row, seam)),
                kind: k,
                carries: cell(row, carries),
            });
        }
    }
    if let Some(t) = table_after_heading(fm.body, "data models", "data models")? {
        let model = require(&t, "data models", "model", &["model"])?;
        let kind = require(&t, "data models", "kind", &["kind"])?;
        let part = require(&t, "data models", "part", &["part"])?;
        let change = require(&t, "data models", "change", &["change"])?;
        let crosses = column(&t.headers, &["crosses"]);
        let note = column(&t.headers, &["note"]);
        for row in &t.rows {
            let k = match first_word(&cell(row, kind)).as_str() {
                "table" => DataModelKind::Table,
                "type" => DataModelKind::Type,
                "schema" => DataModelKind::Schema,
                "event" => DataModelKind::Event,
                "file" => DataModelKind::File,
                other => return Err(PlanParseError::BadModelKind(other.to_owned())),
            };
            let c = match first_word(&cell(row, change)).as_str() {
                "new" => ModelChange::New,
                "changed" => ModelChange::Changed,
                "removed" => ModelChange::Removed,
                other => return Err(PlanParseError::BadModelChange(other.to_owned())),
            };
            plan.data_models.push(DataModel {
                // Names keep their underscores; only the code ticks go.
                model: cell(row, model).replace('`', "").trim().to_owned(),
                kind: k,
                part: clean_id(&cell(row, part)),
                change: c,
                crosses: opt_cell(row, crosses)
                    .map(|s| clean_id(&s))
                    .filter(|s| !s.is_empty() && s != "—" && s != "-"),
                note: opt_cell(row, note),
            });
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn matches_the_typescript_parser_on_the_shared_fixture() {
        let path = "schema/fixtures/plans/valid.md";
        let text = std::fs::read_to_string(root().join(path)).unwrap();
        let plan = parse_plan(&text, path).unwrap();
        // Compare as Plan values: JSON `10` and `10.0` are the same number to both parsers.
        let expected: Plan = serde_json::from_str(
            &std::fs::read_to_string(root().join("schema/fixtures/plans/valid.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(plan, expected);
    }

    #[test]
    fn data_models_fixture_matches_typescript() {
        let read = |name: &str| {
            let p = format!("schema/fixtures/plans/{name}");
            parse_plan(&std::fs::read_to_string(root().join(&p)).unwrap(), &p).unwrap()
        };
        let plan = read("data-models.md");
        let expected: Plan = serde_json::from_str(
            &std::fs::read_to_string(root().join("schema/fixtures/plans/data-models.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(plan, expected);
        assert!(plan.models_off_part().is_empty());
        assert!(plan.models_off_seam().is_empty());

        // "None." and a plan without the section both parse to no models.
        assert!(read("data-models-none.md").data_models.is_empty());
        assert!(read("valid.md").data_models.is_empty());

        // The two that must fail, each for its own reason.
        let names = |ms: Vec<&rubato_protocol::DataModel>| {
            ms.iter().map(|m| m.model.clone()).collect::<Vec<_>>()
        };
        let off_part = read("model-off-part.md");
        assert_eq!(names(off_part.models_off_part()), ["user_tokens"]);
        assert!(off_part.models_off_seam().is_empty());
        let off_seam = read("model-off-seam.md");
        assert!(off_seam.models_off_part().is_empty());
        assert_eq!(names(off_seam.models_off_seam()), ["HookEvent"]);

        let path = "schema/fixtures/plans/data-models.md";
        let text = std::fs::read_to_string(root().join(path)).unwrap();
        assert_eq!(
            parse_plan(&text.replace("| table |", "| blob |"), path),
            Err(PlanParseError::BadModelKind("blob".into()))
        );
        assert_eq!(
            parse_plan(&text.replace("| new |", "| added |"), path),
            Err(PlanParseError::BadModelChange("added".into()))
        );
        println!("checked 5 plan fixtures, 5 data models");
    }

    #[test]
    fn flows_fixture_matches_typescript() {
        let path = "schema/fixtures/plans/flows.md";
        let text = std::fs::read_to_string(root().join(path)).unwrap();
        let plan = parse_plan(&text, path).unwrap();
        let expected: Plan = serde_json::from_str(
            &std::fs::read_to_string(root().join("schema/fixtures/plans/flows.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(plan, expected);
        assert_eq!(plan.flows.len(), 2);

        let map: rubato_protocol::SystemMap = serde_yaml_ng::from_str(
            &std::fs::read_to_string(root().join("docs/design/rubato-map.yml")).unwrap(),
        )
        .unwrap();
        assert!(plan.unknown_ids(&map).is_empty());
        assert!(plan.flows_off_seam(&map).is_empty());

        // The two fixtures that must fail, each for its own reason.
        let read = |name: &str| {
            let p = format!("schema/fixtures/plans/{name}");
            parse_plan(&std::fs::read_to_string(root().join(&p)).unwrap(), &p).unwrap()
        };
        let off = read("flow-off-seam.md");
        assert!(off.unknown_ids(&map).is_empty());
        let off_seam: Vec<_> = off
            .flows_off_seam(&map)
            .iter()
            .map(|f| (f.from.as_str(), f.to.as_str(), f.seam.as_str()))
            .collect();
        assert_eq!(off_seam, [("web", "github", "web-server")]);
        let unknown = read("unknown-flow-id.md");
        assert_eq!(
            unknown.unknown_ids(&map),
            ["rubato-orchestrator", "web-nowhere"]
        );
        assert!(
            unknown.flows_off_seam(&map).is_empty(),
            "an unknown seam is an unknown id, not a second finding"
        );

        // A plan without the table parses as before, and a kind that is neither is an error.
        let old = read("valid.md");
        assert!(old.flows.is_empty());
        let bad = text.replace("| data |", "| magic |");
        assert_eq!(
            parse_plan(&bad, path),
            Err(PlanParseError::BadFlowKind("magic".into()))
        );
        println!("checked 4 plan fixtures, 6 flows");
    }

    #[test]
    fn parses_every_plan_in_this_repo() {
        let mut n = 0;
        for entry in std::fs::read_dir(root().join("docs/plans")).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_some_and(|e| e == "md") {
                let text = std::fs::read_to_string(&p).unwrap();
                let plan = parse_plan(&text, &p.to_string_lossy())
                    .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                assert!(!plan.affected_parts.is_empty(), "{}", p.display());
                n += 1;
            }
        }
        println!("parsed {n} plans");
        assert!(n >= 5);
    }

    #[test]
    fn v0_plan_has_numbered_headings_and_ten_o_rows() {
        let text = std::fs::read_to_string(root().join("docs/plans/v0.md")).unwrap();
        let plan = parse_plan(&text, "docs/plans/v0.md").unwrap();
        assert_eq!(plan.affected_parts.len(), 11);
        assert_eq!(plan.o_rows.len(), 10);
        assert!(plan.assumed.len() > 10);
    }

    #[test]
    fn rejects_missing_issue_and_bad_issue() {
        assert_eq!(parse_plan("---\ntitle: x\n---\n## Affected parts\n| Node | Status | What changes |\n| --- | --- | --- |\n", "p").unwrap_err(), PlanParseError::MissingIssue);
        assert!(matches!(
            parse_plan("---\nissue: nope\n---\n", "p").unwrap_err(),
            PlanParseError::BadIssue(_)
        ));
        assert_eq!(
            parse_plan("---\nissue: a/b#1\n---\nno tables\n", "p").unwrap_err(),
            PlanParseError::NoAffectedParts
        );
    }
}
