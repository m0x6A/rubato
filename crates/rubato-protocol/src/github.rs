//! What the server reads from GitHub and shows in context (seam `server-github`).

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChecksStatus {
    Green,
    Red,
    Pending,
    /// No checks configured.
    None,
}

/// The pull request for an issue branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PullRequestState {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub open: bool,
    pub draft: bool,
    pub checks: ChecksStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub url: String,
}

/// A decision recorded as an issue comment with the `Decision:` template (ADR 0014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionRecord {
    pub issue: IssueRef,
    pub comment_id: u64,
    pub author: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub created_at: DateTime<Utc>,
    pub url: String,
}

/// The parsed parts of a `Decision:` comment body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecisionBody {
    pub title: String,
    pub context: Option<String>,
    pub options: Option<String>,
    pub chosen: Option<String>,
    pub why: Option<String>,
}

impl DecisionRecord {
    pub const PREFIX: &str = "Decision:";

    /// Parse a comment body. `None` when the first line does not start with `Decision:`.
    pub fn parse_body(body: &str) -> Option<DecisionBody> {
        let mut lines = body.lines();
        let first = lines.next()?.trim();
        let title = first.strip_prefix(Self::PREFIX)?.trim();
        if title.is_empty() {
            return None;
        }
        let mut sections: [(&str, Vec<&str>); 4] = [
            ("context", vec![]),
            ("options", vec![]),
            ("chosen", vec![]),
            ("why", vec![]),
        ];
        let mut current: Option<usize> = None;
        for line in lines {
            let heading = line
                .trim()
                .trim_start_matches('#')
                .trim()
                .trim_end_matches(':')
                .to_ascii_lowercase();
            if let Some(i) = sections.iter().position(|(name, _)| *name == heading) {
                current = Some(i);
                continue;
            }
            if let Some(i) = current {
                sections[i].1.push(line);
            }
        }
        let text = |i: usize| {
            let t = sections[i].1.join("\n").trim().to_owned();
            if t.is_empty() { None } else { Some(t) }
        };
        Some(DecisionBody {
            title: title.to_owned(),
            context: text(0),
            options: text(1),
            chosen: text(2),
            why: text(3),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decision_template() {
        let body = "Decision: Use a websocket\n\n## Context\nBoth directions.\n\n## Options\n- ws\n- sse\n\n## Chosen\nws\n\n## Why\nOne connection.";
        let d = DecisionRecord::parse_body(body).unwrap();
        assert_eq!(d.title, "Use a websocket");
        assert_eq!(d.context.as_deref(), Some("Both directions."));
        assert_eq!(d.options.as_deref(), Some("- ws\n- sse"));
        assert_eq!(d.chosen.as_deref(), Some("ws"));
        assert_eq!(d.why.as_deref(), Some("One connection."));
    }

    #[test]
    fn ignores_other_comments() {
        assert!(DecisionRecord::parse_body("Looks good to me").is_none());
        assert!(DecisionRecord::parse_body("Decision:   ").is_none());
    }
}
