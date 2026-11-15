//! Link comments (ADR 0055): the comment Rubato posts as the approver once a handoff commit is
//! on the branch carries a `Rubato-Approval: <id>` line. The comments pass keeps every comment
//! with one, so a post whose outcome was unknown is found by its line. Read by `approve`.

use chrono::{DateTime, Utc};
use rubato_protocol::IssueRef;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The line that names the approval a link comment is for.
pub const LINK_LINE: &str = "Rubato-Approval:";

/// A comment that carries a `Rubato-Approval:` line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkComment {
    pub issue: IssueRef,
    pub comment_id: u64,
    pub author: String,
    pub approval_id: Uuid,
    pub url: String,
    pub created_at: DateTime<Utc>,
}

/// The approval a comment body names on a line of its own, `Rubato-Approval: <uuid>`.
pub fn link_approval(body: &str) -> Option<Uuid> {
    body.lines().find_map(|l| {
        l.trim()
            .strip_prefix(LINK_LINE)
            .and_then(|rest| rest.trim().trim_matches('`').parse().ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_approval_only_on_a_line_of_its_own() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            link_approval(&format!(
                "Plan approved in Rubato.\n\nRubato-Approval: {id}\n"
            )),
            Some(id)
        );
        assert_eq!(
            link_approval(&format!("  Rubato-Approval: `{id}`")),
            Some(id)
        );
        assert_eq!(link_approval(&format!("see Rubato-Approval: {id}")), None);
        assert_eq!(link_approval("Rubato-Approval: not-a-uuid"), None);
        assert_eq!(link_approval("Approved: plan f3c5e84"), None);
    }
}
