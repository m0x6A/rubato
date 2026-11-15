//! Slice `github_poll`: per-issue GitHub reads (metadata, branch, PR and checks, decisions,
//! reviews and approvals), the open issues of the map's repos, the installation's repo coverage
//! and the rate limit. Names no other slice; the composition root drives it. Owns tables
//! `pull_requests`, `decisions`, `issue_meta`, `open_issues`, `backlog_repos`, `uncovered_repos`,
//! `plan_signals` (reviews, approvals and link comments), `repo_pulls` and `repo_pulls_status`.

mod backlog;
mod coverage;
mod links;
mod poll;
mod pulls;
mod store;
mod suggest;

pub use backlog::{BACKLOG_MAX_PER_REPO, Listed, MapLine, list_open_issues};
pub use coverage::check_coverage;
pub use links::{LINK_LINE, LinkComment, link_approval};
pub use poll::{
    BranchPrefs, CommentsRead, RepoView, Standing, has_open_pr, poll_comments, poll_pr,
    poll_pr_of_gone_branch, read_meta, record_rate_limit,
};
pub use pulls::{PullsPass, list_repo_pulls, repo_pulls};
pub use store::{
    IssueMeta, all_approvals, all_decisions, all_issue_meta, all_prs, all_reviews,
    approved_branches, backlog, issue_meta, link_comments, set_target_sha, uncovered,
};
pub use suggest::{SUGGEST_THRESHOLD, suggest};
