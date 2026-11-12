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

/// Whether an issue comment or issue body comes from someone Rubato trusts to say a plan was
/// reviewed or approved (ADR 0027) or where an issue sits (ADR 0026): not a Bot, and the
/// author's association with the repo is OWNER, MEMBER or COLLABORATOR. The repo is public.
pub fn trusted_author(association: &str, is_bot: bool) -> bool {
    !is_bot && matches!(association, "OWNER" | "MEMBER" | "COLLABORATOR")
}

/// A design review posted as an issue comment, first line `Review: <track> — <outcome>`
/// (ADR 0027). Only a trusted author's review is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReviewRecord {
    pub issue: IssueRef,
    pub comment_id: u64,
    pub author: String,
    /// `trivial`, `small` or `full`, as written.
    pub track: String,
    /// The rest of the first line, e.g. `passed after fixes`.
    pub outcome: String,
    /// The plan version the review read, when its `Plan` section names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_open: Option<String>,
    pub created_at: DateTime<Utc>,
    pub url: String,
}

/// The parsed parts of a `Review:` comment body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewBody {
    pub track: String,
    pub outcome: String,
    pub plan_sha: Option<String>,
    pub left_open: Option<String>,
}

/// A person's approval of a plan, first line `Approved: plan <sha>` or
/// `Approved: plan <sha> on <branch>` (ADR 0027). Only a trusted author's approval counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRecord {
    pub issue: IssueRef,
    pub comment_id: u64,
    pub author: String,
    /// The plan hash approved (checkboxes cleared, git blob sha). Absent when the comment names
    /// none: the approval then shows as "version not named".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// The branch the approval names; else the branch Rubato reads the plan from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub url: String,
}

/// The parsed parts of an `Approved:` comment body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApprovalBody {
    pub sha: Option<String>,
    pub branch: Option<String>,
    pub note: Option<String>,
}

/// How an approval relates to the plan Rubato holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalVersion {
    /// The approved hash is the plan's hash now.
    Current,
    /// The plan changed after the approval (finding `ApprovalStale`).
    Earlier,
    /// The comment names no hash.
    NotNamed,
    /// No plan file to compare with on that branch.
    NoPlan,
    /// The approval names the plan as it is now, but the plan does not fit the map, so the
    /// approval does not count (ADR 0029, finding `ApprovedWithoutFit`).
    DoesNotFit,
}

/// The newest counted approval of an issue's plan, and whether it is of the current version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalState {
    pub record: ApprovalRecord,
    pub version: ApprovalVersion,
    /// The hash of the plan compared with, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_sha: Option<String>,
}

/// A hex token of 7 to 40 characters, lower-cased.
fn sha_token(word: &str) -> Option<String> {
    let w = word.trim_matches(|c: char| c == '`' || c == '.' || c == ',');
    (w.len() >= 7 && w.len() <= 40 && w.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| w.to_ascii_lowercase())
}

/// The text under each `## <heading>` of a comment body, headings lower-cased.
fn sections<'a>(lines: impl Iterator<Item = &'a str>, names: &[&str]) -> Vec<Option<String>> {
    let mut found: Vec<Vec<&str>> = vec![Vec::new(); names.len()];
    let mut current: Option<usize> = None;
    for line in lines {
        let t = line.trim();
        if t.starts_with('#') {
            let heading = t.trim_start_matches('#').trim().trim_end_matches(':');
            current = names.iter().position(|n| n.eq_ignore_ascii_case(heading));
            continue;
        }
        if let Some(i) = current {
            found[i].push(line);
        }
    }
    found
        .into_iter()
        .map(|v| {
            let t = v.join("\n").trim().to_owned();
            (!t.is_empty()).then_some(t)
        })
        .collect()
}

impl ReviewRecord {
    pub const PREFIX: &str = "Review:";

    /// Parse a comment body. `None` unless the first line is `Review: <track> — <outcome>`
    /// (an em dash, an en dash or ` - ` separates them).
    pub fn parse_body(body: &str) -> Option<ReviewBody> {
        let mut lines = body.lines();
        let rest = lines.next()?.trim().strip_prefix(Self::PREFIX)?.trim();
        let (track, outcome) = ["—", "–", " - "]
            .iter()
            .find_map(|sep| rest.split_once(sep))?;
        let (track, outcome) = (track.trim(), outcome.trim());
        if track.is_empty() || outcome.is_empty() || track.contains(char::is_whitespace) {
            return None;
        }
        let s = sections(lines, &["plan", "left open"]);
        let plan_sha = s[0]
            .as_deref()
            .and_then(|p| p.split_whitespace().find_map(sha_token));
        Some(ReviewBody {
            track: track.to_ascii_lowercase(),
            outcome: outcome.to_owned(),
            plan_sha,
            left_open: s[1].clone(),
        })
    }
}

impl ApprovalRecord {
    pub const PREFIX: &str = "Approved:";

    /// Parse a comment body. `None` unless the first line starts with `Approved:`. The words
    /// after it are `plan <sha>` and optionally `on <branch>`; a missing or malformed sha leaves
    /// `sha` empty rather than dropping the approval.
    pub fn parse_body(body: &str) -> Option<ApprovalBody> {
        let mut lines = body.lines();
        let rest = lines.next()?.trim().strip_prefix(Self::PREFIX)?.trim();
        let words: Vec<&str> = rest.split_whitespace().collect();
        let mut sha = None;
        let mut branch = None;
        let mut i = 0;
        while i < words.len() {
            match words[i] {
                "plan" if i + 1 < words.len() => {
                    sha = sha_token(words[i + 1]);
                    i += 2;
                }
                "on" if i + 1 < words.len() => {
                    branch = Some(words[i + 1].trim_matches('`').to_owned());
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let note = sections(lines, &["note"]).remove(0);
        Some(ApprovalBody { sha, branch, note })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusts_members_only() {
        assert!(trusted_author("OWNER", false));
        assert!(trusted_author("MEMBER", false));
        assert!(trusted_author("COLLABORATOR", false));
        assert!(!trusted_author("NONE", false));
        assert!(!trusted_author("CONTRIBUTOR", false));
        assert!(!trusted_author("FIRST_TIME_CONTRIBUTOR", false));
        assert!(!trusted_author("OWNER", true), "a Bot never counts");
    }

    #[test]
    fn parses_review_template() {
        let body = "Review: full — passed after fixes\n\n## Plan\n`docs/plans/71.md` blob `9f2c1d3e4b5a6978`.\n\n## Spec check\nok\n\n## Left open\nA1 to A10.";
        let r = ReviewRecord::parse_body(body).unwrap();
        assert_eq!(r.track, "full");
        assert_eq!(r.outcome, "passed after fixes");
        assert_eq!(r.plan_sha.as_deref(), Some("9f2c1d3e4b5a6978"));
        assert_eq!(r.left_open.as_deref(), Some("A1 to A10."));
        let plain = ReviewRecord::parse_body("Review: small - passed").unwrap();
        assert_eq!(
            (plain.track.as_str(), plain.outcome.as_str()),
            ("small", "passed")
        );
        assert_eq!(plain.plan_sha, None);
    }

    /// The shared comment fixtures under `schema/fixtures/comments/`.
    #[test]
    fn parses_the_comment_fixtures() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/fixtures/comments");
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        let r = ReviewRecord::parse_body(&read("review.md")).unwrap();
        assert_eq!(
            (r.track.as_str(), r.outcome.as_str()),
            ("full", "passed after fixes")
        );
        assert_eq!(
            r.plan_sha.as_deref(),
            Some("f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9")
        );
        assert_eq!(r.left_open.as_deref(), Some("Nothing blocks."));
        let a = ApprovalRecord::parse_body(&read("approved.md")).unwrap();
        assert_eq!(
            a.sha.as_deref(),
            Some("f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9")
        );
        assert_eq!(
            (a.branch, a.note.as_deref()),
            (None, Some("Go ahead with PR 1."))
        );
        let b = ApprovalRecord::parse_body(&read("approved-on-branch.md")).unwrap();
        assert_eq!(
            (b.sha.as_deref(), b.branch.as_deref()),
            (Some("f3c5e84"), Some("issue/71-plan"))
        );
        let n = ApprovalRecord::parse_body(&read("approved-no-sha.md")).unwrap();
        assert_eq!(
            n.sha, None,
            "an approval that names no version is kept, unnamed"
        );
        let other = read("not-a-signal.md");
        assert!(ReviewRecord::parse_body(&other).is_none());
        assert!(ApprovalRecord::parse_body(&other).is_none());
        assert!(DecisionRecord::parse_body(&other).is_none());
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            5,
            "fixtures checked"
        );
    }

    #[test]
    fn ignores_comments_that_are_not_reviews() {
        assert!(ReviewRecord::parse_body("Review this please").is_none());
        assert!(ReviewRecord::parse_body("Review: looks fine").is_none());
        assert!(ReviewRecord::parse_body("Review:  — passed").is_none());
        assert!(ReviewRecord::parse_body("I said Review: full — passed").is_none());
    }

    #[test]
    fn parses_approvals() {
        let a =
            ApprovalRecord::parse_body("Approved: plan 9F2C1D3E on issue/71-plan\n\n## Note\nGo.")
                .unwrap();
        assert_eq!(a.sha.as_deref(), Some("9f2c1d3e"));
        assert_eq!(a.branch.as_deref(), Some("issue/71-plan"));
        assert_eq!(a.note.as_deref(), Some("Go."));
        let bare = ApprovalRecord::parse_body("Approved: plan").unwrap();
        assert_eq!(bare, ApprovalBody::default());
        let short = ApprovalRecord::parse_body("Approved: plan abc").unwrap();
        assert_eq!(short.sha, None, "too short to be a sha");
        assert!(ApprovalRecord::parse_body("Not Approved: plan 9f2c1d3e").is_none());
        assert!(ApprovalRecord::parse_body("Decision: x").is_none());
    }

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
