//! Identifiers shared by every part: issues and map references.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A GitHub issue, addressed as `owner/name#number`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct IssueRef {
    /// Repository in `owner/name` form.
    pub repo: String,
    /// Issue number within the repository.
    pub number: u64,
}

impl IssueRef {
    pub fn new(repo: impl Into<String>, number: u64) -> Self {
        Self {
            repo: repo.into(),
            number,
        }
    }

    /// Issue number from a branch named `issue/<n>-<slug>` (ADR 0006), if it follows the convention.
    pub fn number_from_branch(branch: &str) -> Option<u64> {
        let rest = branch.strip_prefix("issue/")?;
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let after = &rest[digits.len()..];
        if !(after.is_empty() || after.starts_with('-')) {
            return None;
        }
        digits.parse().ok()
    }

    /// Whether `branch` is this issue's own launch branch: `issue/<number>-<slug>` with the slug
    /// passing [`is_branch_slug`] (ADR 0006). The companion refuses a launch on anything else; the
    /// branch also names the worktree folder and goes into the launch script.
    pub fn is_branch_for(&self, branch: &str) -> bool {
        branch
            .strip_prefix("issue/")
            .and_then(|rest| rest.strip_prefix(&self.number.to_string()))
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(is_branch_slug)
    }
}

/// A branch slug: words of letters and digits (any script, as the server's `slugify` keeps
/// them) joined by single hyphens, at most 100 characters. No `/`, `.`, `_`, space or quote, so
/// a branch built from it cannot leave its worktree folder or end a shell word.
pub fn is_branch_slug(s: &str) -> bool {
    s.chars().count() <= 100
        && s.split('-')
            .all(|w| !w.is_empty() && w.chars().all(char::is_alphanumeric))
}

impl fmt::Display for IssueRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

/// Error for `IssueRef::from_str`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("expected owner/name#number, got {0:?}")]
pub struct ParseIssueRefError(pub String);

impl FromStr for IssueRef {
    type Err = ParseIssueRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (repo, num) = s
            .rsplit_once('#')
            .ok_or_else(|| ParseIssueRefError(s.to_owned()))?;
        if repo.split('/').filter(|p| !p.is_empty()).count() != 2 {
            return Err(ParseIssueRefError(s.to_owned()));
        }
        let number = num.parse().map_err(|_| ParseIssueRefError(s.to_owned()))?;
        Ok(Self::new(repo, number))
    }
}

/// A reference into the system map: a node or a seam. Ids come from `system-map.yml` (ADR 0007).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapRef {
    Node { id: String },
    Seam { id: String },
}

impl MapRef {
    pub fn node(id: impl Into<String>) -> Self {
        Self::Node { id: id.into() }
    }

    pub fn seam(id: impl Into<String>) -> Self {
        Self::Seam { id: id.into() }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Node { id } | Self::Seam { id } => id,
        }
    }

    pub fn is_node(&self) -> bool {
        matches!(self, Self::Node { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_issue_ref() {
        let r: IssueRef = "m0x6A/rubato#4".parse().unwrap();
        assert_eq!(r, IssueRef::new("m0x6A/rubato", 4));
        assert_eq!(r.to_string(), "m0x6A/rubato#4");
    }

    #[test]
    fn rejects_malformed_issue_ref() {
        assert!("rubato#4".parse::<IssueRef>().is_err());
        assert!("m0x6A/rubato".parse::<IssueRef>().is_err());
        assert!("m0x6A/rubato#x".parse::<IssueRef>().is_err());
    }

    #[test]
    fn branch_convention() {
        assert_eq!(IssueRef::number_from_branch("issue/12-protocol"), Some(12));
        assert_eq!(IssueRef::number_from_branch("issue/7"), Some(7));
        assert_eq!(IssueRef::number_from_branch("issue/7x-bad"), None);
        assert_eq!(IssueRef::number_from_branch("feature/12-protocol"), None);
        assert_eq!(IssueRef::number_from_branch("main"), None);
    }

    #[test]
    fn branch_slugs() {
        for ok in ["restock", "v0-2", "öppna-kartan", "a1-b2-c3"] {
            assert!(is_branch_slug(ok), "{ok:?}");
        }
        let long = "a".repeat(101);
        for bad in [
            "",
            "x';touch p;'",
            "..",
            "a--b",
            "-a",
            "a-",
            "a b",
            "a/b",
            "a.b",
            "a_b",
            "$(id)",
            long.as_str(),
        ] {
            assert!(!is_branch_slug(bad), "{bad:?}");
        }
        assert!(is_branch_slug(&"a".repeat(100)));
    }

    #[test]
    fn a_launch_branch_is_the_issues_own() {
        let issue = IssueRef::new("acme/pantry", 7);
        for ok in ["issue/7-restock", "issue/7-öppna-kartan", "issue/7-v0-2"] {
            assert!(issue.is_branch_for(ok), "{ok:?}");
        }
        for bad in [
            "issue/9-x",
            "issue/7",
            "issue/7-",
            "issue/07-x",
            "issue/77-x",
            "issue/7-../x",
            "issue/7-../../escape",
            "issue/7-a/b",
            "issue/7-x.lock",
            "issue/7-a--b",
            "issue/7-x';touch p;'",
            "-issue/7-x",
            "main",
            "feature/7-x",
        ] {
            assert!(!issue.is_branch_for(bad), "{bad:?}");
        }
    }
}
