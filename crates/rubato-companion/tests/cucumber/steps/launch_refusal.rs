//! Steps for `features/launch-refusal.feature (and the launch steps worktree.feature reuses)`.

use crate::world::*;
use cucumber::{then, when};
use rubato_companion::kernel::gitx;
use rubato_companion::kernel::tmux::session_name;
use rubato_protocol::{CapKind, CompanionMessage, IssueRef, LaunchResult, ServerMessage};
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

#[when(
    expr = "the server sends a launch for user {string} on issue {int} of {string} with slug {string} as attempt {int} of {int}"
)]
async fn when_launch_as_attempt(
    w: &mut RubatoWorld,
    user: String,
    issue: u64,
    repo: String,
    slug: String,
    attempt: u32,
    attempt_limit: u32,
) {
    let mut cmd = w.launch_cmd(&user, &repo, issue, &slug);
    cmd.attempt = attempt;
    cmd.attempt_limit = attempt_limit;
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

/// The first `n` messages the server received after the result of the last launch command, in
/// the order they arrived. Heartbeats come from their own task once a second and are left out.
async fn after_launch_result(w: &RubatoWorld, n: usize) -> Vec<CompanionMessage> {
    let id = w.last_launch.as_ref().unwrap().id;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let after: Vec<CompanionMessage> = {
            let got = w.server().received.lock().await;
            got.iter()
                .map(|e| &e.message)
                .skip_while(|m| {
                    !matches!(m, CompanionMessage::Launch(
                        LaunchResult::Refused { command_id, .. }
                        | LaunchResult::Started { command_id, .. }
                        | LaunchResult::Failed { command_id, .. },
                    ) if *command_id == id)
                })
                .skip(1)
                .filter(|m| !matches!(m, CompanionMessage::Heartbeat(_)))
                .take(n)
                .cloned()
                .collect()
        };
        if after.len() == n || tokio::time::Instant::now() >= deadline {
            return after;
        }
        let _ =
            tokio::time::timeout(Duration::from_millis(200), w.server().notify.notified()).await;
    }
}

#[then(expr = "right after that launch result the server receives a sessions snapshot")]
async fn then_sessions_right_after(w: &mut RubatoWorld) {
    let after = after_launch_result(w, 1).await;
    assert!(
        matches!(after.as_slice(), [CompanionMessage::Sessions { .. }]),
        "expected a sessions snapshot right after the launch result, got {after:?}"
    );
}

#[then(
    expr = "right after that launch result the server receives a cap hit of kind {string} for issue {int} of {string}, then a sessions snapshot"
)]
async fn then_cap_hit_then_sessions(w: &mut RubatoWorld, kind: String, issue: u64, repo: String) {
    let cmd = w.last_launch.clone().unwrap();
    let after = after_launch_result(w, 2).await;
    let [
        CompanionMessage::CapHit(hit),
        CompanionMessage::Sessions { .. },
    ] = after.as_slice()
    else {
        panic!("expected a cap hit and then a sessions snapshot, got {after:?}")
    };
    assert_eq!(
        serde_json::to_value(hit.kind).unwrap().as_str(),
        Some(kind.as_str())
    );
    assert_eq!(hit.kind, CapKind::Attempts);
    assert_eq!(hit.issue, IssueRef::new(repo, issue));
    assert_eq!(hit.value, f64::from(cmd.attempt));
    assert_eq!(hit.limit, f64::from(cmd.attempt_limit));
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
