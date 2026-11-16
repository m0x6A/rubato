//! What a launch was for (ADR 0071, plan 220 §2): the parts of its `LaunchCommand` the registry
//! does not otherwise keep, so the agent-facing MCP server can answer `get_assignment`. Known
//! from `begin_launch`, before the launcher starts `claude`, so a session that asks at once is
//! answered; kept on the entry and in `launches.json` afterwards.

use super::Registry;
use chrono::{DateTime, Utc};
use rubato_protocol::{IssueRef, LaunchCommand, PlanningMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// The parts of a launch only its command knows. Kept on the entry and in `launches.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    /// Map node and seam ids the issue was pinned to when the server sent the launch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<String>,
    /// Git sha of the map those pins were resolved against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_version: Option<String>,
    pub planning: PlanningMode,
    pub attempt: u32,
    pub attempt_limit: u32,
    /// When the companion began (pending) or recorded (started) the launch.
    pub launched_at: DateTime<Utc>,
}

impl Assignment {
    pub fn of(cmd: &LaunchCommand, now: DateTime<Utc>) -> Self {
        Self {
            node_ids: cmd.node_ids.clone(),
            map_version: cmd.map_version.clone(),
            planning: cmd.planning,
            attempt: cmd.attempt,
            attempt_limit: cmd.attempt_limit,
            launched_at: now,
        }
    }
}

/// A launch between `begin_launch` and the launcher's return: what it was sent to do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingLaunch {
    pub launch_id: Uuid,
    pub issue: IssueRef,
    pub branch: String,
    pub plan_path: Option<String>,
    pub spend_cap_usd: f64,
    pub assignment: Assignment,
}

impl PendingLaunch {
    pub fn of(cmd: &LaunchCommand, now: DateTime<Utc>) -> Self {
        Self {
            launch_id: cmd.id,
            issue: cmd.issue.clone(),
            branch: cmd.branch.clone(),
            plan_path: cmd.plan_path.clone(),
            spend_cap_usd: cmd.spend_cap_usd,
            assignment: Assignment::of(cmd, now),
        }
    }
}

/// What `get_assignment` answers: an owned copy, so the caller holds no registry lock.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssignmentView {
    pub launch_id: Uuid,
    pub repo: String,
    pub issue: IssueRef,
    pub branch: Option<String>,
    /// Absent while the launch is pending: the launcher has not made it yet.
    pub worktree: Option<PathBuf>,
    pub plan_path: Option<String>,
    pub spend_usd: Option<f64>,
    pub spend_cap_usd: Option<f64>,
    /// The launcher has not returned yet.
    pub pending: bool,
    #[serde(flatten)]
    pub assignment: Assignment,
}

impl Registry {
    /// The assignment of a live launch of this companion, or of the one in flight. `None` for a
    /// launch that ended, one from before an upgrade that kept no assignment, or one never seen.
    pub fn assignment_of(&self, launch_id: Uuid) -> Option<AssignmentView> {
        if let Some(e) = self.by_launch(launch_id).filter(|e| !e.ended) {
            let assignment = e.assignment.clone()?;
            let issue = e.issue.clone()?;
            return Some(AssignmentView {
                launch_id,
                repo: issue.repo.clone(),
                issue,
                branch: e.branch.clone(),
                worktree: e.worktree.clone(),
                plan_path: e.plan_path.clone(),
                spend_usd: e.spend_usd,
                spend_cap_usd: e.spend_cap_usd,
                pending: false,
                assignment,
            });
        }
        let p = self.pending.as_ref().filter(|p| p.launch_id == launch_id)?;
        Some(AssignmentView {
            launch_id,
            repo: p.issue.repo.clone(),
            issue: p.issue.clone(),
            branch: Some(p.branch.clone()),
            worktree: None,
            plan_path: p.plan_path.clone(),
            spend_usd: None,
            spend_cap_usd: Some(p.spend_cap_usd),
            pending: true,
            assignment: p.assignment.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap()
    }

    fn command() -> LaunchCommand {
        LaunchCommand {
            id: Uuid::from_u128(1),
            user: "m0x6A".into(),
            issue: IssueRef::new("m0x6A/rubato", 7),
            repo_url: "https://github.com/m0x6A/rubato".into(),
            branch: "issue/7-agent".into(),
            base_ref: "origin/main".into(),
            node_ids: vec!["rubato-companion".into(), "companion-mcp".into()],
            spend_cap_usd: 25.0,
            attempt_limit: 3,
            attempt: 2,
            session_id: Uuid::from_u128(2),
            prompt: "plan it".into(),
            otel_resource: BTreeMap::new(),
            map_version: Some("abc1234".into()),
            plan_path: Some("docs/plans/7.md".into()),
            draft: None,
            planning: PlanningMode::Rubato,
        }
    }

    #[test]
    fn a_pending_launch_is_known_before_the_launcher_returns() {
        let mut reg = Registry::new(Uuid::nil(), "m0x6A");
        let cmd = command();
        assert!(reg.assignment_of(cmd.id).is_none());
        reg.begin_launch(&cmd, now());
        let a = reg.assignment_of(cmd.id).expect("pending launch is known");
        assert!(a.pending);
        assert_eq!(a.issue, cmd.issue);
        assert_eq!(a.branch.as_deref(), Some("issue/7-agent"));
        assert_eq!(a.assignment.node_ids, cmd.node_ids);
        assert_eq!(a.assignment.attempt, 2);
        assert_eq!(a.assignment.attempt_limit, 3);
        assert_eq!(a.assignment.planning, PlanningMode::Rubato);
        assert_eq!(a.assignment.launched_at, now());
        assert_eq!(a.worktree, None, "no worktree until the launcher made it");
    }

    #[test]
    fn a_failed_launch_is_forgotten_and_a_started_one_lives_on_its_entry() {
        let mut reg = Registry::new(Uuid::nil(), "m0x6A");
        let cmd = command();
        reg.begin_launch(&cmd, now());
        reg.end_launch();
        assert!(reg.assignment_of(cmd.id).is_none(), "failed: forgotten");

        reg.begin_launch(&cmd, now());
        reg.end_launch();
        reg.apply_launch(&cmd, "%1", "rubato-7", Path::new("/w/issue-7"), now());
        let a = reg.assignment_of(cmd.id).expect("started");
        assert!(!a.pending);
        assert_eq!(a.worktree.as_deref(), Some(Path::new("/w/issue-7")));
        assert_eq!(a.spend_cap_usd, Some(25.0));
        assert_eq!(a.plan_path.as_deref(), Some("docs/plans/7.md"));
        assert_eq!(a.assignment.map_version.as_deref(), Some("abc1234"));
    }

    #[test]
    fn an_ended_or_unknown_launch_has_no_assignment() {
        let mut reg = Registry::new(Uuid::nil(), "m0x6A");
        let cmd = command();
        reg.begin_launch(&cmd, now());
        reg.end_launch();
        reg.apply_launch(&cmd, "%1", "rubato-7", Path::new("/w/issue-7"), now());
        assert!(reg.assignment_of(Uuid::from_u128(99)).is_none());
        reg.find_by_launch(cmd.id).unwrap().ended = true;
        assert!(reg.assignment_of(cmd.id).is_none());
    }
}
