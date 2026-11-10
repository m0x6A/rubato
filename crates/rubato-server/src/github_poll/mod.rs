//! Slice `github_poll`: per-issue GitHub reads (metadata, branch, PR and checks, decisions) and
//! the rate limit. Names no other slice; the composition root drives it. Owns tables
//! `pull_requests`, `decisions` and `issue_meta`.

mod poll;
mod store;

pub use poll::{find_branch, poll_decisions, poll_pr, record_rate_limit, upsert_meta};
pub use store::{IssueMeta, all_decisions, all_issue_meta, all_prs, issue_meta};
