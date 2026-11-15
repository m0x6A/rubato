//! Where an unpinned issue probably belongs (ADR 0026): a score per node and seam from what its
//! title and body say, the best one kept when it reaches [`SUGGEST_THRESHOLD`]. Pure: the same
//! map, title and body always give the same suggestion. A person confirms it; nothing here pins.

use crate::kernel::github::OpenIssueInfo;
use rubato_protocol::{MapRef, PinSuggestion, SystemMap, parse_map_placement};

/// Lowest score shown (design A6).
pub const SUGGEST_THRESHOLD: u8 = 50;

const ID: u8 = 100;
const PATH: u8 = 80;
/// Per description word in common, capped at [`WORDS_MAX`].
const WORD: u8 = 15;
const WORDS_MAX: u8 = 60;

/// Associations GitHub gives people with access to the repo: their `Map:` lines place issues.
/// The repo is public, so anyone else's only suggests (ADR 0026).
pub fn trusted(author_association: &str) -> bool {
    matches!(author_association, "OWNER" | "MEMBER" | "COLLABORATOR")
}

/// What one listed issue's body means for the map.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placement {
    /// A `Map:` line from a trusted author: pin it (the pins slice decides whether it may).
    pub pin: Option<MapRef>,
    /// A `Map:` line from anyone else, shown with the issue.
    pub untrusted_line: Option<MapRef>,
    pub suggestion: Option<PinSuggestion>,
}

/// The `Map:` line that speaks to workspace `ws`: unqualified, or naming `ws` (ADR 0046).
pub fn line_for(body: &str, ws: &str) -> Option<MapRef> {
    parse_map_placement(body)
        .filter(|p| p.workspace.as_deref().is_none_or(|w| w == ws))
        .map(|p| p.target)
}

/// Read the `Map:` line for workspace `ws` and, when nothing places the issue there, suggest a
/// place on that workspace's map.
pub fn place(map: &SystemMap, issue: &OpenIssueInfo, ws: &str) -> Placement {
    match line_for(&issue.body, ws) {
        Some(target) if trusted(&issue.author_association) => Placement {
            pin: Some(target),
            ..Placement::default()
        },
        Some(target) => {
            let known = if target.is_node() {
                map.node(target.id()).is_some()
            } else {
                map.seam(target.id()).is_some()
            };
            Placement {
                suggestion: known.then(|| PinSuggestion {
                    target: target.clone(),
                    score: ID,
                    why: vec!["named by a Map: line from someone outside the team".into()],
                }),
                untrusted_line: Some(target),
                pin: None,
            }
        }
        None => Placement {
            suggestion: suggest(map, &issue.title, &issue.body),
            ..Placement::default()
        },
    }
}

/// Words of `text`, lower-cased, split on anything that cannot be in an id.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .map(|w| w.trim_matches('.').to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Words that say something: five letters or more, not a common English word.
fn meaningful(w: &str) -> bool {
    const COMMON: &[&str] = &[
        "about", "after", "every", "their", "there", "these", "those", "which", "where", "while",
        "would", "should", "could", "other", "being", "through", "without", "between", "issue",
        "issues", "change", "changes",
    ];
    w.len() >= 5 && w.chars().all(|c| c.is_ascii_alphabetic()) && !COMMON.contains(&w)
}

/// Whether a word of `text` is the path or names a file under it (`web/src/App.tsx` for `web`),
/// so a short path such as `web` does not match `websocket`.
fn mentions_path(text: &str, path: &str) -> bool {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || "/-_.".contains(c))))
        .map(|w| w.trim_start_matches("./"))
        .any(|w| w == path || w.starts_with(&format!("{path}/")))
}

fn score_one(
    id: &str,
    path: Option<&str>,
    description: Option<&str>,
    words: &[String],
    text: &str,
) -> (u8, Vec<String>) {
    let id_l = id.to_ascii_lowercase();
    if words.contains(&id_l) {
        return (ID, vec![format!("id {id} in the issue")]);
    }
    if let Some(p) = path.filter(|p| !p.is_empty())
        && mentions_path(text, &p.trim_matches('/').to_ascii_lowercase())
    {
        return (PATH, vec![format!("path {p} in the issue")]);
    }
    let mut common: Vec<String> = description
        .map(tokens)
        .unwrap_or_default()
        .into_iter()
        .filter(|w| meaningful(w) && words.contains(w))
        .collect();
    common.sort();
    common.dedup();
    let score = (common.len() as u8).saturating_mul(WORD).min(WORDS_MAX);
    if common.is_empty() {
        (0, vec![])
    } else {
        (score, vec![format!("words: {}", common.join(", "))])
    }
}

/// The best place on the map for an issue with this title and body, if any scores at least
/// [`SUGGEST_THRESHOLD`]. Ties go to nodes before seams, then to the lower id.
pub fn suggest(map: &SystemMap, title: &str, body: &str) -> Option<PinSuggestion> {
    let text = format!("{title}\n{body}").to_ascii_lowercase();
    let words = tokens(&text);
    let mut scored: Vec<(u8, u8, String, PinSuggestion)> = Vec::new();
    for n in &map.nodes {
        let (score, why) = score_one(
            &n.id,
            n.path.as_deref(),
            n.description.as_deref(),
            &words,
            &text,
        );
        scored.push((
            score,
            0,
            n.id.clone(),
            PinSuggestion {
                target: MapRef::node(&n.id),
                score,
                why,
            },
        ));
    }
    for s in &map.seams {
        let description = [s.description.as_deref(), s.carries.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let (score, why) = score_one(&s.id, None, Some(&description), &words, &text);
        scored.push((
            score,
            1,
            s.id.clone(),
            PinSuggestion {
                target: MapRef::seam(&s.id),
                score,
                why,
            },
        ));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    scored
        .into_iter()
        .next()
        .filter(|(score, ..)| *score >= SUGGEST_THRESHOLD)
        .map(|(.., s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::{MapNode, NodeKind, Seam};

    fn map() -> SystemMap {
        let node = |id: &str, path: Option<&str>, description: &str| MapNode {
            id: id.into(),
            kind: NodeKind::Context,
            repo: Some("o/r".into()),
            path: path.map(str::to_owned),
            description: Some(description.into()),
        };
        SystemMap {
            version: 1,
            name: "t".into(),
            owner_rule: None,
            repos: vec![],
            nodes: vec![
                node(
                    "rubato-server",
                    Some("crates/rubato-server"),
                    "Shared server: pins, polling, websocket fan-out",
                ),
                node(
                    "web",
                    Some("web"),
                    "Browser frontend drawing the system map",
                ),
                node(
                    "plan-check",
                    Some("actions/pc"),
                    "GitHub Action checking plan files",
                ),
            ],
            seams: vec![Seam {
                id: "web-server".into(),
                from: "web".into(),
                to: "rubato-server".into(),
                carries: Some("overview over websocket".into()),
                owner: "rubato-server".into(),
                guard: None,
                description: Some("The browser reads the overview".into()),
                flows: vec![],
            }],
            scenarios: vec![],
        }
    }

    fn info(body: &str, association: &str) -> OpenIssueInfo {
        OpenIssueInfo {
            number: 1,
            title: "A title".into(),
            body: body.into(),
            author_association: association.into(),
            ..OpenIssueInfo::default()
        }
    }

    #[test]
    fn suggestions_are_deterministic_and_explained() {
        let m = map();
        type Case<'a> = (&'a str, &'a str, Option<(MapRef, u8)>);
        let cases: [Case; 6] = [
            // An exact id beats everything; the seam id is a token of its own.
            (
                "The web-server seam drops events",
                "",
                Some((MapRef::seam("web-server"), 100)),
            ),
            (
                "Fix rubato-server polling",
                "",
                Some((MapRef::node("rubato-server"), 100)),
            ),
            // A path in the body.
            (
                "Broken action",
                "see actions/pc/src/cli.ts",
                Some((MapRef::node("plan-check"), 80)),
            ),
            // Four description words in common: 60.
            (
                "Polling and fan-out",
                "pins polling websocket shared server",
                Some((MapRef::node("rubato-server"), 60)),
            ),
            // One word: 15, below the threshold.
            ("Something about polling", "", None),
            ("Nothing at all", "", None),
        ];
        for (title, body, want) in &cases {
            let got = suggest(&m, title, body).map(|s| (s.target, s.score));
            assert_eq!(got, *want, "{title:?}");
        }
        let s = suggest(&m, "Fix rubato-server polling", "").unwrap();
        assert_eq!(s.why, ["id rubato-server in the issue"]);
        // The same input twice gives the same answer.
        assert_eq!(
            suggest(&m, "pins polling websocket shared server", ""),
            suggest(&m, "pins polling websocket shared server", "")
        );
        println!("checked {} suggestion cases", cases.len());
    }

    #[test]
    fn only_members_place_an_issue() {
        let m = map();
        for who in ["OWNER", "MEMBER", "COLLABORATOR"] {
            let p = place(
                &m,
                &info("Map: seam web-server", who),
                rubato_protocol::DEFAULT_WORKSPACE,
            );
            assert_eq!(p.pin, Some(MapRef::seam("web-server")), "{who}");
            assert_eq!(p.suggestion, None);
        }
        for who in ["CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR", "NONE", ""] {
            let p = place(
                &m,
                &info("Map: seam web-server", who),
                rubato_protocol::DEFAULT_WORKSPACE,
            );
            assert_eq!(p.pin, None, "{who} does not place an issue");
            assert_eq!(p.untrusted_line, Some(MapRef::seam("web-server")));
            assert_eq!(
                p.suggestion.map(|s| (s.target, s.score)),
                Some((MapRef::seam("web-server"), 100)),
                "the line becomes the suggestion"
            );
        }
        // An untrusted line naming no map id suggests nothing.
        let p = place(
            &m,
            &info("Map: node billing", "NONE"),
            rubato_protocol::DEFAULT_WORKSPACE,
        );
        assert_eq!((p.pin, p.suggestion), (None, None));
        // No line: scored.
        let p = place(
            &m,
            &info("rubato-server is slow", "NONE"),
            rubato_protocol::DEFAULT_WORKSPACE,
        );
        assert_eq!(
            p.suggestion.map(|s| s.target),
            Some(MapRef::node("rubato-server"))
        );
    }
}
