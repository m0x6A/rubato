//! Steps for `features/discovery.feature`.

use crate::world::*;
use cucumber::{given, then, when};
use rubato_companion::kernel::gitx::default_worktree_path;
use rubato_companion::kernel::tmux::session_name;
use rubato_protocol::{CompanionMessage, IssueRef};
use std::time::Duration;

#[given(
    expr = "a worktree for branch {string} of {string} with a tmux pane running the stand-in claude in it"
)]
async fn given_worktree_with_pane(w: &mut RubatoWorld, branch: String, repo: String) {
    let clone = w.repos.get(&repo).cloned().expect("repo created");
    let wt = default_worktree_path(&clone, &branch);
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    RubatoWorld::git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &branch,
            &wt.to_string_lossy(),
            "main",
        ],
    );
    let claude = w.workspace.companion.claude_command.clone();
    w.tmux()
        .new_session(&session_name(&repo, &branch), &wt, &claude)
        .unwrap();
}

#[given(
    expr = "a worktree for branch {string} of {string} with a tmux pane running the stand-in claude under a wrapper script"
)]
async fn given_worktree_with_wrapped_pane(w: &mut RubatoWorld, branch: String, repo: String) {
    let clone = w.repos.get(&repo).cloned().expect("repo created");
    let wt = default_worktree_path(&clone, &branch);
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    RubatoWorld::git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &branch,
            &wt.to_string_lossy(),
            "main",
        ],
    );
    // Not `exec`: the shell stays the pane's command and claude runs as its child.
    let wrapper = w.root.join("wrapper.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n'{}'\necho done\n",
            w.workspace.companion.claude_command
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        &wrapper,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    w.tmux()
        .new_session(
            &session_name(&repo, &branch),
            &wt,
            &format!("sh '{}'", wrapper.display()),
        )
        .unwrap();
}

#[given(expr = "a tmux pane running the stand-in claude in the clone of {string}")]
async fn given_pane_in_clone(w: &mut RubatoWorld, repo: String) {
    let clone = w.repos.get(&repo).cloned().expect("repo created");
    let claude = w.workspace.companion.claude_command.clone();
    w.tmux().new_session("plain", &clone, &claude).unwrap();
}

#[when(expr = "the clone of {string} checks out a new branch {string}")]
async fn when_checkout(w: &mut RubatoWorld, repo: String, branch: String) {
    let clone = w.repos.get(&repo).cloned().expect("repo created");
    // mtime granularity: the checkout must land on a later time than the first read.
    tokio::time::sleep(Duration::from_millis(50)).await;
    RubatoWorld::git(&clone, &["checkout", "-q", "-b", &branch]);
}

#[then(
    expr = "the server receives a sessions snapshot with a session on branch {string} and no issue"
)]
async fn then_snapshot_no_issue(w: &mut RubatoWorld, branch: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(15), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.branch.as_deref() == Some(branch.as_str()))
                .cloned(),
            _ => None,
        })
        .await;
    let s = got.expect("a session on the branch");
    assert_eq!(s.issue, None);
}

#[given("a private tmux server")]
async fn given_private_tmux(w: &mut RubatoWorld) {
    w.tmux().new_session("probe", &w.root, "sh").unwrap();
}

#[then(
    expr = "the server receives a sessions snapshot with issue {int} of {string} on branch {string}"
)]
async fn then_snapshot_branch(w: &mut RubatoWorld, issue: u64, repo: String, branch: String) {
    let want = IssueRef::new(repo, issue);
    let got = w
        .server()
        .wait_for(Duration::from_secs(15), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.issue.as_ref() == Some(&want))
                .cloned(),
            _ => None,
        })
        .await;
    let s = got.expect("snapshot with the issue");
    assert_eq!(s.branch.as_deref(), Some(branch.as_str()));
}

#[then(expr = "that snapshot entry has source {string} and status {string}")]
async fn then_snapshot_entry(w: &mut RubatoWorld, source: String, status: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(5), |m| match m {
            CompanionMessage::Sessions { sessions } => {
                sessions.iter().find(|s| s.issue.is_some()).map(|s| {
                    (
                        serde_json::to_value(s.source).unwrap(),
                        serde_json::to_value(s.status).unwrap(),
                    )
                })
            }
            _ => None,
        })
        .await
        .unwrap();
    assert_eq!(got.0.as_str(), Some(source.as_str()));
    assert_eq!(got.1.as_str(), Some(status.as_str()));
}

/// A hook from the session in that worktree's pane, as Claude Code with our hooks opted in would
/// post it (plan 147 Q3): the fixture with its `cwd` set to the worktree.
#[when(
    expr = "Claude Code in the worktree for branch {string} of {string} posts the recorded hook {string}"
)]
async fn when_pane_posts_hook(w: &mut RubatoWorld, branch: String, repo: String, fixture: String) {
    let wt = default_worktree_path(w.repos.get(&repo).unwrap(), &branch);
    let mut body: serde_json::Value =
        serde_json::from_str(&RubatoWorld::hook_fixture(&fixture)).unwrap();
    body["cwd"] = serde_json::Value::String(wt.to_string_lossy().into_owned());
    assert_eq!(w.post_hook_body(&body.to_string(), true).await, 202);
}

#[then(
    expr = "the server receives a sessions snapshot where issue {int} of {string} has source {string} and status {string}"
)]
async fn then_snapshot_issue_state(
    w: &mut RubatoWorld,
    issue: u64,
    repo: String,
    source: String,
    status: String,
) {
    let want = IssueRef::new(repo, issue);
    let got = w
        .server()
        .wait_for(Duration::from_secs(15), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| {
                    s.issue.as_ref() == Some(&want)
                        && serde_json::to_value(s.source).unwrap().as_str() == Some(source.as_str())
                        && serde_json::to_value(s.status).unwrap().as_str() == Some(status.as_str())
                })
                .cloned(),
            _ => None,
        })
        .await;
    assert!(
        got.is_some(),
        "no snapshot with {want} from {source} as {status}"
    );
}

#[then(expr = "the companion logs {string}")]
async fn then_logs(_w: &mut RubatoWorld, text: String) {
    for _ in 0..50 {
        if crate::logs::contains(&text) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no log line containing {text:?} within 5 s");
}

#[then("the companion daemon is still running")]
async fn then_still_running(w: &mut RubatoWorld) {
    let join = w.join.as_ref().expect("daemon started");
    assert!(!join.is_finished(), "the daemon ended on its own");
}
