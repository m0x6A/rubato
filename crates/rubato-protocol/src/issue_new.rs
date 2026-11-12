//! Creating an issue from inside Rubato (ADR 0025): the request the form sends, the answer, and
//! the body the server renders under #71's headings. The GitHub App writes the issue; the body
//! names the person and ends with the `Rubato-Request:` and `Map:` lines.

use crate::ids::{IssueRef, MapRef};
use crate::issue_body::map_line;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `POST /api/issues`. Each list item is one line of its section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewIssueRequest {
    /// Made by the form once per draft; a second submit with the same id creates nothing more.
    pub request_id: String,
    /// `owner/name`, a repo the map names.
    pub repo: String,
    /// The part or seam the issue was started from. It is pinned there at creation.
    pub target: MapRef,
    pub title: String,
    pub summary: String,
    /// "What the change does", at least one line.
    pub does: Vec<String>,
    /// "How we'll know it works", at least one line; written as checkboxes.
    pub know: Vec<String>,
    /// "What it must not break", at least one line.
    pub must_not_break: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_questions: Vec<String>,
}

/// Where a creation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreationState {
    /// Sent to GitHub, not confirmed yet: it may exist. The next listing finds it by its
    /// `Rubato-Request:` line, or after ten minutes unseen it is `failed`.
    Opening,
    Created,
    Failed,
}

/// The answer to `POST /api/issues`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewIssueResponse {
    pub state: CreationState,
    /// Set once created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// What to tell the person, for `opening` and `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The line that ties an issue on GitHub to the request that created it.
pub fn request_line(request_id: &str) -> String {
    format!("Rubato-Request: {request_id}")
}

/// The request id a body names, if any: the last `Rubato-Request:` line.
pub fn parse_request_line(body: &str) -> Option<String> {
    body.lines()
        .filter_map(|l| l.trim().strip_prefix("Rubato-Request: "))
        .map(str::trim)
        .rfind(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(str::to_owned)
}

/// Why a request cannot be sent, in the words the form shows. Empty when it can.
pub fn problems(req: &NewIssueRequest) -> Vec<&'static str> {
    let blank = |s: &str| s.trim().is_empty();
    let none = |v: &[String]| v.iter().all(|s| blank(s));
    let mut out = Vec::new();
    if parse_request_line(&request_line(&req.request_id)).as_deref() != Some(&req.request_id) {
        out.push("the request id must be letters, digits, - or _");
    }
    if blank(&req.title) {
        out.push("a title");
    }
    if blank(&req.summary) {
        out.push("a summary");
    }
    if none(&req.does) {
        out.push("at least one line under What the change does");
    }
    if none(&req.know) {
        out.push("at least one line under How we'll know it works");
    }
    if none(&req.must_not_break) {
        out.push("at least one line under What it must not break");
    }
    out
}

/// The issue body: #71's headings in order, checkboxes under "How we'll know it works", the
/// creator in backticks (so GitHub notifies nobody), then the `Rubato-Request:` and `Map:`
/// lines. Blank lines in the lists are left out. What the person typed is kept as typed.
pub fn render_issue(req: &NewIssueRequest, creator: &str) -> String {
    fn lines(items: &[String], bullet: &str) -> String {
        items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| format!("{bullet}{s}\n"))
            .collect()
    }
    let mut b = String::new();
    b.push_str("## Summary\n");
    b.push_str(req.summary.trim());
    b.push_str("\n\n## What the change does\n");
    b.push_str(&lines(&req.does, "- "));
    b.push_str("\n## How we'll know it works\n");
    b.push_str(&lines(&req.know, "- [ ] "));
    b.push_str("\n## What it must not break\n");
    b.push_str(&lines(&req.must_not_break, "- "));
    if let Some(c) = req
        .context
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        b.push_str("\n## Context\n");
        b.push_str(c);
        b.push('\n');
    }
    let questions = lines(&req.open_questions, "- ");
    if !questions.is_empty() {
        b.push_str("\n## Open questions\n");
        b.push_str(&questions);
    }
    let who = creator.trim_start_matches('@');
    b.push_str(&format!("\n---\nOpened in Rubato by `{who}`.\n\n"));
    b.push_str(&request_line(&req.request_id));
    b.push('\n');
    b.push_str(&map_line(&req.target));
    b.push('\n');
    b
}

/// Route paths.
pub mod routes {
    pub const ISSUES: &str = "/api/issues";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_body::parse_map_line;

    fn req() -> NewIssueRequest {
        NewIssueRequest {
            request_id: "3f2a9c1e-0b7d-4c55-9e61-2d0c5b7a8e10".into(),
            repo: "m0x6A/rubato".into(),
            target: MapRef::node("rubato-server"),
            title: "Poll less".into(),
            summary: "The poll costs too many calls.".into(),
            does: vec!["Lists branches once per repo.".into(), "  ".into()],
            know: vec!["The call-order test asserts fewer calls.".into()],
            must_not_break: vec!["Progress latency.".into()],
            context: Some("Found in #49.".into()),
            open_questions: vec![],
        }
    }

    /// The rendered body, exactly: the headings in order, checkboxes for "know", no bare `@`,
    /// the request and map lines last.
    #[test]
    fn the_body_follows_the_issue_pattern() {
        let body = render_issue(&req(), "@m0x6A");
        assert_eq!(
            body,
            "## Summary\nThe poll costs too many calls.\n\n\
             ## What the change does\n- Lists branches once per repo.\n\n\
             ## How we'll know it works\n- [ ] The call-order test asserts fewer calls.\n\n\
             ## What it must not break\n- Progress latency.\n\n\
             ## Context\nFound in #49.\n\n\
             ---\nOpened in Rubato by `m0x6A`.\n\n\
             Rubato-Request: 3f2a9c1e-0b7d-4c55-9e61-2d0c5b7a8e10\n\
             Map: node rubato-server\n"
        );
        assert!(!body.contains('@'), "nobody is notified");
        assert_eq!(parse_map_line(&body), Some(MapRef::node("rubato-server")));
        assert_eq!(
            parse_request_line(&body).as_deref(),
            Some("3f2a9c1e-0b7d-4c55-9e61-2d0c5b7a8e10")
        );
        let with_questions = NewIssueRequest {
            open_questions: vec!["Which lane?".into()],
            context: None,
            ..req()
        };
        let b = render_issue(&with_questions, "m0x6A");
        assert!(b.contains("## Open questions\n- Which lane?\n"));
        assert!(!b.contains("## Context"));
    }

    #[test]
    fn a_request_without_the_required_sections_is_refused() {
        assert!(problems(&req()).is_empty());
        let empty = NewIssueRequest {
            title: " ".into(),
            summary: String::new(),
            does: vec![" ".into()],
            know: vec![],
            must_not_break: vec![],
            request_id: "a b".into(),
            ..req()
        };
        assert_eq!(problems(&empty).len(), 6, "{:?}", problems(&empty));
    }
}
