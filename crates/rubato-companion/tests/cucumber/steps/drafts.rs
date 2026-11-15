//! Steps for `features/drafts.feature`.

use super::session_messages::docstring;
use crate::world::*;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use rubato_protocol::{CompanionMessage, DraftBundle, DraftRefusal, bundle_hash, capability};
use std::path::Path;
use std::time::Duration;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn launched_worktree(w: &RubatoWorld) -> std::path::PathBuf {
    let cmd = w.last_launch.as_ref().expect("a launch");
    worktree_of(w, &cmd.issue.repo, &cmd.branch)
}

#[given(expr = "the launched session's worktree has {string}:")]
async fn given_worktree_file(w: &mut RubatoWorld, rel: String, step: &Step) {
    write(
        &launched_worktree(w),
        &rel,
        &format!("{}\n", docstring(step)),
    );
}

#[when(expr = "the launched session's worktree now has {string}:")]
async fn when_worktree_file(w: &mut RubatoWorld, rel: String, step: &Step) {
    write(
        &launched_worktree(w),
        &rel,
        &format!("{}\n", docstring(step)),
    );
}

#[given(expr = "the launched session's worktree has {string} of {int} characters")]
async fn given_large_file(w: &mut RubatoWorld, rel: String, n: usize) {
    write(&launched_worktree(w), &rel, &"x".repeat(n));
}

#[given(expr = "the clone of {string} has {string}:")]
async fn given_clone_file(w: &mut RubatoWorld, repo: String, rel: String, step: &Step) {
    let clone = w.repos.get(&repo).expect("the repository").clone();
    write(&clone, &rel, &format!("{}\n", docstring(step)));
}

#[given(expr = "the server limits drafts to {int} bytes")]
async fn given_draft_limit(w: &mut RubatoWorld, n: u64) {
    w.server().limits.lock().unwrap().draft_max_bytes = n;
}

#[given("the server does not announce drafts")]
async fn given_no_drafts(w: &mut RubatoWorld) {
    w.server()
        .capabilities
        .lock()
        .unwrap()
        .retain(|c| c != capability::DRAFTS);
}

#[when(expr = "a session started by hand in the clone of {string} posts a {string} hook")]
async fn when_hand_started_hook(w: &mut RubatoWorld, repo: String, event: String) {
    let clone = w.repos.get(&repo).expect("the repository").clone();
    let session = uuid::Uuid::new_v4().to_string();
    let body = serde_json::json!({
        "session_id": session, "hook_event_name": event, "cwd": clone.to_string_lossy(),
    });
    assert_eq!(w.post_hook_body(&body.to_string(), true).await, 202);
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| {
            matches!(m, CompanionMessage::Hook(h) if h.session_id == session).then_some(())
        })
        .await;
    assert!(got.is_some(), "the hook reached the server");
}

async fn drafts(w: &RubatoWorld) -> Vec<DraftBundle> {
    w.server()
        .received
        .lock()
        .await
        .iter()
        .filter_map(|e| match &e.message {
            CompanionMessage::Draft(b) => Some(b.clone()),
            _ => None,
        })
        .collect()
}

async fn last(w: &RubatoWorld) -> DraftBundle {
    drafts(w).await.pop().expect("a draft bundle")
}

#[then(expr = "the server receives {int} draft bundle(s)")]
async fn then_receives(w: &mut RubatoWorld, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while drafts(w).await.len() < n && tokio::time::Instant::now() < deadline {
        let _ =
            tokio::time::timeout(Duration::from_millis(200), w.server().notify.notified()).await;
    }
    let got = drafts(w).await;
    assert_eq!(got.len(), n, "{got:?}");
}

#[then(expr = "the server has received {int} draft bundle(s)")]
async fn then_has_received(w: &mut RubatoWorld, n: usize) {
    let got = drafts(w).await;
    assert_eq!(got.len(), n, "{got:?}");
}

#[then(expr = "no draft bundle arrives within {int} second(s)")]
async fn then_none(w: &mut RubatoWorld, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let got = drafts(w).await;
    assert!(got.is_empty(), "{got:?}");
}

#[then(expr = "the last draft bundle has the files {string}")]
async fn then_files(w: &mut RubatoWorld, files: String) {
    let b = last(w).await;
    let want: Vec<&str> = files.split(", ").collect();
    let got: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(got, want);
    let cmd = w.last_launch.as_ref().unwrap();
    assert_eq!(
        (b.launch_id, &b.issue, &b.branch),
        (cmd.id, &cmd.issue, &cmd.branch)
    );
    assert_eq!(b.session_id, Some(cmd.session_id.to_string()));
}

#[then("the last draft bundle's hash matches its files")]
async fn then_hash(w: &mut RubatoWorld) {
    let b = last(w).await;
    let files: Vec<(String, String)> = b
        .files
        .iter()
        .map(|f| (f.path.clone(), f.text.clone()))
        .collect();
    assert_eq!(b.bundle_hash, bundle_hash(&b.files[0].path, &files));
    assert_eq!(b.refused, None);
}

#[then("the last two draft bundles have different hashes")]
async fn then_different(w: &mut RubatoWorld) {
    let got = drafts(w).await;
    let [.., a, b] = got.as_slice() else {
        panic!("fewer than two bundles: {got:?}")
    };
    assert_ne!(a.bundle_hash, b.bundle_hash);
    assert!(
        b.files.iter().any(|f| f.text.contains("second cut")),
        "{b:?}"
    );
}

#[then(expr = "the last draft bundle lists {string} as missing")]
async fn then_missing(w: &mut RubatoWorld, path: String) {
    assert_eq!(last(w).await.missing, [path]);
}

#[then("the last draft bundle is refused as too large, with no files and no hash")]
async fn then_too_large(w: &mut RubatoWorld) {
    let b = last(w).await;
    assert_eq!(b.refused, Some(DraftRefusal::TooLarge));
    assert!(b.files.is_empty(), "{b:?}");
    assert_eq!(b.bundle_hash, "");
}
