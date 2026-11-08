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
}
