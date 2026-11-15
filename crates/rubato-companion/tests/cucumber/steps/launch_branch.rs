//! Steps for `features/launch-branch.feature`.

use crate::world::*;
use cucumber::{then, when};
use rubato_protocol::ServerMessage;

#[when(
    expr = "the server sends a launch for user {string} on issue {int} of {string} on branch {string}"
)]
async fn when_launch_on_branch(
    w: &mut RubatoWorld,
    user: String,
    issue: u64,
    repo: String,
    branch: String,
) {
    let mut cmd = w.launch_cmd(&user, &repo, issue, "unused");
    cmd.branch = branch.clone();
    cmd.otel_resource.insert("rubato.branch".into(), branch);
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

#[when(
    expr = "the server sends a launch for user {string} on issue {int} of {string} with slug {string} and node {string}"
)]
async fn when_launch_with_node(
    w: &mut RubatoWorld,
    user: String,
    issue: u64,
    repo: String,
    slug: String,
    node: String,
) {
    let mut cmd = w.launch_cmd(&user, &repo, issue, &slug);
    cmd.otel_resource.insert("rubato.node".into(), node.clone());
    cmd.node_ids = vec![node];
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

#[then(expr = "no file {string} exists in the worktree or the clone")]
async fn then_no_file(w: &mut RubatoWorld, name: String) {
    // Wait for the stand-in to have run, so the script has had its chance to misbehave.
    let _ = standin_log(w).await;
    let cmd = w.last_launch.as_ref().unwrap();
    let mut dirs = vec![w.repos.get(&cmd.issue.repo).unwrap().clone()];
    dirs.extend(w.started_worktree.clone());
    for d in &dirs {
        assert!(
            !d.join(&name).exists(),
            "{} exists",
            d.join(&name).display()
        );
    }
}
