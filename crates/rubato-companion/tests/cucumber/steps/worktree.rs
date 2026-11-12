//! Steps for `features/worktree.feature`.

use super::launch_refusal::when_launch;
use crate::world::*;
use cucumber::{given, then};
use rubato_companion::kernel::gitx;
use rubato_companion::kernel::tmux::session_name;
use rubato_protocol::{CompanionMessage, IssueRef, LaunchResult};
use std::path::PathBuf;
use std::time::Duration;

#[given(expr = "a temporary repository {string} with a bare origin")]
async fn given_repo(w: &mut RubatoWorld, name: String) {
    w.make_repo(&name);
}

#[given(
    expr = "the server has launched issue {int} of {string} with slug {string} for user {string}"
)]
async fn given_launched(w: &mut RubatoWorld, issue: u64, repo: String, slug: String, user: String) {
    when_launch(w, user, issue, repo, slug).await;
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started { command_id, .. })
                if *command_id == id =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(got.is_some(), "first launch started");
}

#[then(expr = "the server receives a launch result {string}")]
async fn then_launch_started(w: &mut RubatoWorld, outcome: String) {
    assert_eq!(outcome, "started");
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(60), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started {
                command_id,
                worktree,
                ..
            }) if *command_id == id => Some(Ok(PathBuf::from(worktree))),
            CompanionMessage::Launch(LaunchResult::Failed {
                command_id, error, ..
            }) if *command_id == id => Some(Err(error.clone())),
            CompanionMessage::Launch(LaunchResult::Refused {
                command_id, detail, ..
            }) if *command_id == id => Some(Err(detail.clone())),
            _ => None,
        })
        .await;
    match got {
        Some(Ok(wt)) => w.started_worktree = Some(wt),
        other => panic!("launch did not start: {other:?}"),
    }
}

#[then(expr = "a worktree exists for branch {string} of {string} beside the clone")]
async fn then_worktree(w: &mut RubatoWorld, branch: String, repo: String) {
    let wt = worktree_of(w, &repo, &branch);
    assert_eq!(
        w.started_worktree.as_deref(),
        Some(wt.as_path()),
        "reported worktree is the sibling path"
    );
    assert!(wt.is_dir(), "{}", wt.display());
    assert_eq!(gitx::current_branch(&wt).as_deref(), Some(branch.as_str()));
}

#[then(expr = "a tmux session exists for {string} branch {string} with a pane in that worktree")]
async fn then_session(w: &mut RubatoWorld, repo: String, branch: String) {
    let name = session_name(&repo, &branch);
    assert!(
        w.tmux().has_session(&name).unwrap(),
        "no tmux session {name}"
    );
    let wt = worktree_of(w, &repo, &branch);
    let panes = w.tmux().panes().unwrap();
    assert!(
        panes.iter().any(|p| p.session == name && p.cwd == wt),
        "{panes:?}"
    );
}

#[then("the stand-in claude was started with the session id from the command")]
async fn then_session_id(w: &mut RubatoWorld) {
    let log = standin_log(w).await;
    let id = w.last_launch.as_ref().unwrap().session_id.to_string();
    assert!(log.contains(&format!("--session-id {id}")), "{log}");
}

#[then(expr = "the stand-in claude received {string} in OTEL_RESOURCE_ATTRIBUTES")]
async fn then_otel_attr(w: &mut RubatoWorld, attr: String) {
    let log = standin_log(w).await;
    let line = log
        .lines()
        .find(|l| l.starts_with("OTEL_RESOURCE_ATTRIBUTES="))
        .expect("OTEL_RESOURCE_ATTRIBUTES set");
    assert!(line.contains(&attr), "{line}");
    assert!(log.contains("CLAUDE_CODE_ENABLE_TELEMETRY=1"));
    assert!(log.contains("OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf"));
}

#[then("the stand-in claude received a settings file whose hooks post to the companion's port")]
async fn then_settings(w: &mut RubatoWorld) {
    let log = standin_log(w).await;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    let path = args
        .split_whitespace()
        .skip_while(|a| *a != "--settings")
        .nth(1)
        .expect("--settings <file>");
    let text = std::fs::read_to_string(path).unwrap();
    let port = w.handles.as_ref().unwrap().hook_endpoint.port;
    assert!(
        text.contains(&format!("http://127.0.0.1:{port}/hook")),
        "{text}"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600,
        "hooks file holds the token"
    );
}

#[then(
    expr = "the server receives a sessions snapshot with issue {int} of {string} from source {string}"
)]
async fn then_snapshot_source(w: &mut RubatoWorld, issue: u64, repo: String, source: String) {
    let want = IssueRef::new(repo, issue);
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.issue.as_ref() == Some(&want))
                .map(|s| {
                    serde_json::to_value(s.source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                }),
            _ => None,
        })
        .await;
    assert_eq!(got.as_deref(), Some(source.as_str()));
}
