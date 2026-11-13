//! Open pull requests of the map's repos (issue #82, ADR 0034): listed once per repo per tick,
//! same-repo branches only. A pull request's view and merging as the person (ADR 0033) build on
//! these types. Facts GitHub reported; nothing here is a progress state.

use crate::github::ChecksStatus;
use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One open pull request from a branch of the repo itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenPull {
    /// `owner/name`, in the map's spelling.
    pub repo: String,
    pub number: u64,
    pub title: String,
    /// GitHub login of who opened it.
    pub author: String,
    /// The head branch.
    pub branch: String,
    /// The branch it merges into.
    pub base: String,
    pub draft: bool,
    pub head_sha: String,
    /// The folded check runs at `head_sha`; absent until they have been read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<ChecksStatus>,
    pub updated_at: DateTime<Utc>,
    pub url: String,
    /// The issue the branch names (`issue/<n>-…`, ADR 0006), if it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
}

/// One map repo's open pull requests and how fresh that listing is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RepoPulls {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pulls: Vec<OpenPull>,
    /// When the last listing succeeded; absent until one has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_at: Option<DateTime<Utc>>,
    /// The last listing hit its one page: there are more than `pulls` shows.
    #[serde(default)]
    pub truncated: bool,
    /// Why the last listing failed, when it did; `pulls` are from the last good one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OpenPull {
    /// Whether the stored checks need another read on the full lane (ADR 0034): never read,
    /// the head moved since, or not finished.
    pub fn checks_due(checks: Option<ChecksStatus>, checks_sha: Option<&str>, head: &str) -> bool {
        match checks {
            None => true,
            Some(_) if checks_sha != Some(head) => true,
            Some(ChecksStatus::Pending | ChecksStatus::None) => true,
            Some(ChecksStatus::Green | ChecksStatus::Red) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_are_read_again_until_finished_at_the_same_head() {
        let due = |c, s| OpenPull::checks_due(c, s, "abc");
        assert!(due(None, None), "never read");
        assert!(due(Some(ChecksStatus::Green), Some("old")), "head moved");
        assert!(
            due(Some(ChecksStatus::Pending), Some("abc")),
            "still running"
        );
        assert!(
            due(Some(ChecksStatus::None), Some("abc")),
            "not started yet"
        );
        assert!(!due(Some(ChecksStatus::Green), Some("abc")));
        assert!(!due(Some(ChecksStatus::Red), Some("abc")));
    }
}
