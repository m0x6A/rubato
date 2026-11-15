//! Steps for `features/restore.feature`.

use super::session_messages::docstring;
use crate::world::*;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use rubato_protocol::{
    CompanionMessage, DraftFile, DraftRestore, LaunchResult, RestoreSkipped, ServerMessage,
};
use std::path::PathBuf;
use std::time::Duration;

/// `\n` in a table cell or a quoted value is a newline; `<root>` is the scenario's root dir.
fn unescape(w: &RubatoWorld, s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("<root>", &w.root.to_string_lossy())
}

#[when(
    expr = "the server sends a launch for user {string} on issue {int} of {string} with slug {string} and the draft:"
)]
async fn when_launch_with_draft(
    w: &mut RubatoWorld,
    user: String,
    issue: u64,
    repo: String,
    slug: String,
    step: &Step,
) {
    let rows = &step.table.as_ref().expect("a table of path and text").rows;
    let files = rows[1..]
        .iter()
        .map(|r| DraftFile {
            path: unescape(w, &r[0]),
            text: unescape(w, &r[1]),
        })
        .collect();
    let mut cmd = w.launch_cmd(&user, &repo, issue, &slug);
    cmd.draft = Some(DraftRestore {
        draft_id: uuid::Uuid::new_v4(),
        rev: 3,
        files,
    });
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

#[given(expr = "a worktree for branch {string} of {string} already has {string}:")]
async fn given_reused_worktree(
    w: &mut RubatoWorld,
    branch: String,
    repo: String,
    rel: String,
    step: &Step,
) {
    let wt = worktree_of(w, &repo, &branch);
    if !wt.exists() {
        let clone = w.repos.get(&repo).expect("the repository").clone();
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
    }
    let p = wt.join(&rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, format!("{}\n", docstring(step))).unwrap();
}

/// The `Started` result of the last launch.
async fn started(w: &RubatoWorld) -> (bool, Option<RestoreSkipped>) {
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(60), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started {
                command_id,
                draft_restored,
                restore_skipped,
                ..
            }) if *command_id == id => Some(Ok((*draft_restored, *restore_skipped))),
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
        Some(Ok(r)) => r,
        other => panic!("the launch did not start: {other:?}"),
    }
}

#[then(expr = "the server receives a launch result \"started\" with the draft restored")]
async fn then_restored(w: &mut RubatoWorld) {
    assert_eq!(started(w).await, (true, None));
}

#[then(
    expr = "the server receives a launch result \"started\" with the restore skipped as {string}"
)]
async fn then_skipped(w: &mut RubatoWorld, reason: String) {
    let (restored, skipped) = started(w).await;
    assert!(!restored);
    assert_eq!(skipped.map(|s| s.name()), Some(reason.as_str()));
}

#[then(expr = "the server receives a launch result \"started\" with no draft restored")]
async fn then_no_draft(w: &mut RubatoWorld) {
    assert_eq!(started(w).await, (false, None));
}

fn launched_worktree(w: &RubatoWorld) -> PathBuf {
    let cmd = w.last_launch.as_ref().unwrap();
    worktree_of(w, &cmd.issue.repo, &cmd.branch)
}

#[then(expr = "the launched worktree has {string} reading {string}")]
async fn then_file(w: &mut RubatoWorld, rel: String, text: String) {
    let p = launched_worktree(w).join(&rel);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), unescape(w, &text));
}

#[then(expr = "the launched worktree has no {string}")]
async fn then_no_file(w: &mut RubatoWorld, rel: String) {
    let p = launched_worktree(w).join(&rel);
    assert!(!p.exists(), "{} was written", p.display());
}

#[then(expr = "nothing was written at {string} outside the launched worktree")]
async fn then_nothing_outside(w: &mut RubatoWorld, path: String) {
    let path = unescape(w, &path);
    // Relative paths resolve against the worktree, absolute ones stand alone.
    let p = launched_worktree(w).join(&path);
    assert!(!p.exists(), "{} was written", p.display());
}
