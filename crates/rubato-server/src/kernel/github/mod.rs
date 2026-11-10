//! GitHub behind one trait: octocrab in production, local git repos for e2e runs, an in-memory
//! fake in tests. Items are re-exported here so slices name them `kernel::github::<item>`.

mod api;
mod fake;
mod local;

pub use api::{Comment, GitHubApi, IssueInfo, Octo, PullInfo, summarize_check_runs};
pub use fake::{FakeGitHub, FakeInner};
pub use local::LocalGit;
