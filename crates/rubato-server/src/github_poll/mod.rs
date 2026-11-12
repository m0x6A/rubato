//! Slice `github_poll`: per-issue GitHub reads (metadata, branch, PR and checks, decisions,
//! reviews and approvals), the open issues of the map's repos, the installation's repo coverage
//! and the rate limit. Names no other slice; the composition root drives it. Owns tables
//! `pull_requests`, `decisions`, `issue_meta`, `open_issues`, `backlog_repos`, `uncovered_repos`
//! and `plan_signals`.

mod backlog;
mod coverage;
mod poll;
mod store;
mod suggest;

pub use backlog::{BACKLOG_MAX_PER_REPO, Listed, MapLine, list_open_issues};
pub use coverage::check_coverage;
pub use poll::{
    CommentsRead, RepoView, Standing, has_open_pr, poll_comments, poll_pr, poll_pr_of_gone_branch,
    read_meta, record_rate_limit,
};
pub use store::{
    IssueMeta, all_approvals, all_decisions, all_issue_meta, all_prs, all_reviews, backlog,
    issue_meta, set_target_sha, uncovered,
};
pub use suggest::{SUGGEST_THRESHOLD, suggest};
