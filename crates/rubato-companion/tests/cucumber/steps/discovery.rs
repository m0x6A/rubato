//! Steps for `features/discovery.feature`.

use crate::world::*;
use cucumber::{given, then, when};
use rubato_companion::kernel::tmux::session_name;
use rubato_companion::kernel::tmux_ide::{Fleet, TmuxIde, default_worktree_path, version_at_least};
use rubato_companion::sessions::Registry;
use rubato_protocol::{CompanionMessage, IssueRef, SessionStatus, StateSource};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

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

#[given("a private tmux server")]
async fn given_private_tmux(w: &mut RubatoWorld) {
    w.tmux().new_session("probe", &w.root, "sh").unwrap();
}

#[given(expr = "the recorded tmux-ide fleet {string}")]
async fn given_fleet(w: &mut RubatoWorld, file: String) {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures")
            .join(file),
    )
    .unwrap();
    w.fleet = Some(Fleet::parse(&text).unwrap());
}

#[when("the companion runs tmux-ide team --json")]
async fn when_team(w: &mut RubatoWorld) {
    let ti = TmuxIde::new("tmux-ide", w.tmux());
    w.team_result = Some(
        tokio::task::spawn_blocking(move || Ok((ti.team()?, ti.version()?)))
            .await
            .unwrap(),
    );
}

#[when("the companion merges it into an empty registry")]
async fn when_merge_fleet(w: &mut RubatoWorld) {
    let mut reg = Registry::new(Uuid::nil(), "m0x6A");
    reg.apply_fleet(w.fleet.as_ref().unwrap(), chrono::Utc::now());
    w.registry = Some(reg);
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

#[then(expr = "it parses and the tmux-ide version is at least {string}")]
async fn then_team_parses(w: &mut RubatoWorld, min: String) {
    let (fleet, version) = w.team_result.take().unwrap().expect("tmux-ide team --json");
    let _ = fleet.agents().count();
    assert!(version_at_least(&version, &min), "{version}");
}

#[then(expr = "pane {string} has status {string} from source {string}")]
async fn then_pane(w: &mut RubatoWorld, pane: String, status: String, source: String) {
    let reg = w.registry.as_ref().unwrap();
    let e = reg
        .entries()
        .iter()
        .find(|e| e.pane_id.as_deref() == Some(pane.as_str()))
        .expect("pane known");
    let want_status: SessionStatus =
        serde_json::from_value(serde_json::Value::String(status)).unwrap();
    let want_source: StateSource =
        serde_json::from_value(serde_json::Value::String(source)).unwrap();
    assert_eq!((e.status, e.source), (want_status, want_source));
}

/// What `tmux-ide events --follow` does with no adopted session (no `events.jsonl`): exit at once.
/// Every other subcommand goes to the real tmux-ide.
#[given("tmux-ide has no events to follow")]
async fn given_no_events(w: &mut RubatoWorld) {
    let path = w.fake_bin.join("tmux-ide-no-events");
    std::fs::write(
        &path,
        "#!/bin/sh\n[ \"$1\" = events ] && exit 0\nexec tmux-ide \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    w.workspace.companion.tmux_ide_command = path.to_string_lossy().into_owned();
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
