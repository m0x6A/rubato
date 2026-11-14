//! What an issue body says about the map (ADR 0026): the `Map:` line that places an issue, and
//! the suggested place for an issue that has none. Pure text in, values out.

use crate::ids::MapRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where Rubato suggests an unpinned issue belongs, and why. A person confirms it with one
/// click through the pin route; nothing is pinned by a suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PinSuggestion {
    pub target: MapRef,
    /// 0–100. Only suggestions at or above the server's threshold are sent.
    pub score: u8,
    /// What matched, in words a person can check: `id web-server in the body`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<String>,
}

/// Where a `Map:` line places an issue: the part or seam, and the workspace it names, if any
/// (ADR 0046). On the wire it is a `MapRef` with an optional `workspace`, so an old value reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MapPlacement {
    #[serde(flatten)]
    pub target: MapRef,
    /// A workspace id: the line places the issue in this workspace only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

impl MapPlacement {
    pub fn new(target: MapRef, workspace: Option<&str>) -> Self {
        Self {
            target,
            workspace: workspace.map(str::to_string),
        }
    }
}

impl From<MapRef> for MapPlacement {
    fn from(target: MapRef) -> Self {
        Self {
            target,
            workspace: None,
        }
    }
}

/// The line that places an issue: `Map: node <id>` or `Map: seam <id>`.
pub fn map_line(target: &MapRef) -> String {
    map_line_in(target, None)
}

/// The line that places an issue, naming a workspace when given one (ADR 0046):
/// `Map: node <id> in <workspace>`.
pub fn map_line_in(target: &MapRef, workspace: Option<&str>) -> String {
    let (kind, id) = match target {
        MapRef::Node { id } => ("node", id),
        MapRef::Seam { id } => ("seam", id),
    };
    match workspace {
        Some(ws) => format!("Map: {kind} {id} in {ws}"),
        None => format!("Map: {kind} {id}"),
    }
}

fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The last well-formed `Map:` line outside a code fence. The word `Map:` is case-sensitive and
/// the line holds nothing else, so prose that mentions a map does not place an issue by accident.
/// With `qualified`, `<kind> <id> in <workspace>` is well-formed too. Ids are not checked
/// against a map here.
fn last_line(body: &str, qualified: bool) -> Option<MapPlacement> {
    let mut fence: Option<&str> = None;
    let mut found = None;
    for raw in body.lines() {
        let line = raw.trim();
        if let Some(open) = fence {
            if line.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if line.starts_with("```") {
            fence = Some("```");
            continue;
        }
        if line.starts_with("~~~") {
            fence = Some("~~~");
            continue;
        }
        let Some(rest) = line.strip_prefix("Map: ") else {
            continue;
        };
        let words: Vec<&str> = rest.split_whitespace().collect();
        let (kind, id, workspace) = match words.as_slice() {
            [kind, id] => (*kind, *id, None),
            [kind, id, "in", ws] if qualified && is_id(ws) => (*kind, *id, Some(*ws)),
            _ => continue,
        };
        if !is_id(id) {
            continue;
        }
        let target = match kind {
            "node" => MapRef::node(id),
            "seam" => MapRef::seam(id),
            _ => continue,
        };
        found = Some(MapPlacement::new(target, workspace));
    }
    found
}

/// The placement an issue body names, as a server without workspaces reads it: the last
/// well-formed two-word `Map:` line (ADR 0026). A line naming a workspace is not read here.
pub fn parse_map_line(body: &str) -> Option<MapRef> {
    last_line(body, false).map(|p| p.target)
}

/// The placement an issue body names, with the workspace its line names, if any (ADR 0046).
pub fn parse_map_placement(body: &str) -> Option<MapPlacement> {
    last_line(body, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> String {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/issues")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    #[test]
    fn the_shared_fixtures_place_or_do_not() {
        let cases: [(&str, Option<MapRef>); 7] = [
            ("voice-note.md", Some(MapRef::seam("web-server"))),
            ("in-code-fence.md", None),
            ("two-lines.md", Some(MapRef::node("rubato-server"))),
            ("unknown-id.md", Some(MapRef::node("billing"))),
            ("malformed.md", None),
            // A server without workspaces reads only the two-word form (ADR 0046).
            ("qualified.md", None),
            ("qualified-bad.md", None),
        ];
        for (file, want) in &cases {
            assert_eq!(parse_map_line(&fixture(file)), *want, "{file}");
        }
        println!("checked {} issue body fixtures", cases.len());
    }

    #[test]
    fn a_line_may_name_its_workspace() {
        let cases: [(&str, Option<MapPlacement>); 7] = [
            (
                "qualified.md",
                Some(MapPlacement::new(MapRef::node("auth-api"), Some("shop"))),
            ),
            ("qualified-bad.md", None),
            (
                "voice-note.md",
                Some(MapPlacement::new(MapRef::seam("web-server"), None)),
            ),
            ("in-code-fence.md", None),
            (
                "two-lines.md",
                Some(MapPlacement::new(MapRef::node("rubato-server"), None)),
            ),
            (
                "unknown-id.md",
                Some(MapPlacement::new(MapRef::node("billing"), None)),
            ),
            ("malformed.md", None),
        ];
        for (file, want) in &cases {
            assert_eq!(parse_map_placement(&fixture(file)), *want, "{file}");
        }
        println!(
            "checked {} issue body fixtures for a workspace",
            cases.len()
        );
        // Written lines read back, with and without a workspace.
        for (t, ws) in [
            (MapRef::node("auth-api"), Some("shop")),
            (MapRef::seam("web-server"), None),
        ] {
            let body = format!("x\n\n{}\n", map_line_in(&t, ws));
            assert_eq!(
                parse_map_placement(&body),
                Some(MapPlacement::new(t.clone(), ws))
            );
        }
        assert_eq!(
            map_line_in(&MapRef::node("a"), None),
            map_line(&MapRef::node("a"))
        );
    }

    #[test]
    fn a_placement_reads_an_old_map_ref_and_writes_none_when_unqualified() {
        let old: MapPlacement = serde_json::from_str(r#"{"kind":"node","id":"web"}"#).unwrap();
        assert_eq!(old, MapPlacement::new(MapRef::node("web"), None));
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"kind":"node","id":"web"}"#
        );
        let q = MapPlacement::new(MapRef::seam("s"), Some("shop"));
        assert_eq!(
            serde_json::to_string(&q).unwrap(),
            r#"{"kind":"seam","id":"s","workspace":"shop"}"#
        );
    }

    #[test]
    fn written_lines_read_back() {
        for t in [MapRef::node("rubato-server"), MapRef::seam("web-server")] {
            let body = format!("## Summary\nx\n\n{}\n", map_line(&t));
            assert_eq!(parse_map_line(&body), Some(t));
        }
        assert_eq!(parse_map_line(""), None);
        // An unclosed fence hides everything after it.
        assert_eq!(parse_map_line("```\nMap: node web\n"), None);
    }
}
