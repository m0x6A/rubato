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

/// The line that places an issue: `Map: node <id>` or `Map: seam <id>`.
pub fn map_line(target: &MapRef) -> String {
    match target {
        MapRef::Node { id } => format!("Map: node {id}"),
        MapRef::Seam { id } => format!("Map: seam {id}"),
    }
}

fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The placement an issue body names: the last well-formed `Map:` line outside a code fence.
/// The word `Map:` is case-sensitive and the line holds nothing else, so prose that mentions a
/// map does not place an issue by accident. The id is not checked against a map here.
pub fn parse_map_line(body: &str) -> Option<MapRef> {
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
        let mut words = rest.split_whitespace();
        let (Some(kind), Some(id), None) = (words.next(), words.next(), words.next()) else {
            continue;
        };
        if !is_id(id) {
            continue;
        }
        found = match kind {
            "node" => Some(MapRef::node(id)),
            "seam" => Some(MapRef::seam(id)),
            _ => continue,
        };
    }
    found
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
        let cases: [(&str, Option<MapRef>); 5] = [
            ("voice-note.md", Some(MapRef::seam("web-server"))),
            ("in-code-fence.md", None),
            ("two-lines.md", Some(MapRef::node("rubato-server"))),
            ("unknown-id.md", Some(MapRef::node("billing"))),
            ("malformed.md", None),
        ];
        for (file, want) in &cases {
            assert_eq!(parse_map_line(&fixture(file)), *want, "{file}");
        }
        println!("checked {} issue body fixtures", cases.len());
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
