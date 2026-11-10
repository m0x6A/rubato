//! Steps for `features/launch-refusal.feature (and the launch steps worktree.feature reuses)`.

use crate::world::*;
use cucumber::{then, when};
use rubato_companion::kernel::gitx;
use rubato_companion::kernel::tmux::session_name;
use rubato_protocol::{CompanionMessage, LaunchResult, ServerMessage};
use std::time::Duration;

#[when(
    expr = "the server sends a launch for user {string} on issue {int} of {string} with slug {string}"
)]
pub(crate) async fn when_launch(
    w: &mut RubatoWorld,
    user: String,
    issue: u64,
    repo: String,
    slug: String,
) {
    let cmd = w.launch_cmd(&user, &repo, issue, &slug);
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

#[then(expr = "the server receives a launch result {string} with reason {string}")]
async fn then_launch_refused(w: &mut RubatoWorld, outcome: String, reason: String) {
    assert_eq!(outcome, "refused");
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Refused {
                command_id,
                reason: r,
                ..
            }) if *command_id == id => Some(
                serde_json::to_value(r)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            ),
            CompanionMessage::Launch(LaunchResult::Failed {
                command_id, error, ..
            }) if *command_id == id => Some(format!("FAILED: {error}")),
            _ => None,
        })
        .await;
    assert_eq!(got.as_deref(), Some(reason.as_str()));
}

#[then(expr = "no worktree exists for branch {string} of {string}")]
async fn then_no_worktree(w: &mut RubatoWorld, branch: String, repo: String) {
    let list = gitx::worktrees(w.repos.get(&repo).unwrap()).unwrap();
    assert!(
        !list
            .iter()
            .any(|(_, b)| b.as_deref() == Some(branch.as_str())),
        "{list:?}"
    );
    assert!(!worktree_of(w, &repo, &branch).exists());
}

#[then(expr = "no tmux session exists for {string} branch {string}")]
async fn then_no_session(w: &mut RubatoWorld, repo: String, branch: String) {
    assert!(!w.tmux().has_session(&session_name(&repo, &branch)));
}
