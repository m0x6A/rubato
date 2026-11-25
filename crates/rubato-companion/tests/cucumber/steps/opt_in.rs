//! Steps for opting a session started by hand in to our hooks (ADR 0078): the real binary's
//! `render-hooks --into` and `status`, and hooks fired as Claude Code fires them.

use crate::world::*;
use cucumber::{given, then, when};
use rubato_companion::hooks::{HOOKS_TEMPLATE, render_manual_hooks, settings_path, write_into};
use rubato_companion::kernel::config::HookEndpoint;
use rubato_protocol::CompanionMessage;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn companion(w: &RubatoWorld) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rubato-companion"));
    c.arg("--state-dir").arg(&w.state_dir);
    c
}

/// `rubato-companion render-hooks --into <dir>/.claude/settings.local.json`, as the developer runs it.
fn opt_in_by_hand(w: &RubatoWorld, dir: &Path) {
    let out = companion(w)
        .args(["render-hooks", "--into"])
        .arg(settings_path(dir))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "render-hooks --into failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The command a settings file holds for `event`: ours, the one that posts to the companion.
fn hook_command(settings: &Path, event: &str) -> String {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
    v["hooks"][event]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["hooks"].as_array().unwrap().clone())
        .filter_map(|h| h["command"].as_str().map(str::to_owned))
        .find(|c| rubato_companion::hooks::our_port(c).is_some())
        .unwrap_or_else(|| panic!("no hook of ours for {event} in {}", settings.display()))
}

/// Run a hook command as Claude Code does: through `sh`, the payload on stdin, the session's
/// environment (`RUBATO_LAUNCH_ID` when launched).
fn run_hook(command: &str, payload: &serde_json::Value, launch_id: Option<String>) {
    let mut sh = Command::new("sh");
    sh.arg("-c")
        .arg(command)
        .env_remove("RUBATO_LAUNCH_ID")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if let Some(id) = launch_id {
        sh.env("RUBATO_LAUNCH_ID", id);
    }
    let mut child = sh.spawn().unwrap();
    use std::io::Write;
    // A hook that stands down (the opt-in in a launched session, `STAND_DOWN`) exits without
    // reading its input, so the write can meet a closed pipe; Claude Code does not fail on that
    // either. The exit status below is still checked.
    match child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
    {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
        r => r.unwrap(),
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "hook command failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[given("the workspace is written to local.yml")]
async fn given_local_yml(w: &mut RubatoWorld) {
    std::fs::write(
        w.root.join("local.yml"),
        serde_yaml_ng::to_string(&w.workspace).unwrap(),
    )
    .unwrap();
}

#[when(expr = "the developer opts the clone of {string} in with render-hooks --into")]
async fn when_opt_in_clone(w: &mut RubatoWorld, repo: String) {
    let dir = w.repos.get(&repo).cloned().expect("repo created");
    opt_in_by_hand(w, &dir);
}

#[given(expr = "the clone of {string} has our hooks on port {int}")]
async fn given_stale(w: &mut RubatoWorld, repo: String, port: u16) {
    let dir = w.repos.get(&repo).cloned().expect("repo created");
    let hooks = render_manual_hooks(
        HOOKS_TEMPLATE,
        port,
        &HookEndpoint::header_path(&w.state_dir),
    )
    .unwrap();
    write_into(&settings_path(&dir), &hooks).unwrap();
}

#[when("the developer runs rubato-companion status")]
async fn when_status(w: &mut RubatoWorld) {
    let out = companion(w)
        .arg("--workspace")
        .arg(w.root.join("local.yml"))
        .arg("status")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    w.status_out = Some(String::from_utf8(out.stdout).unwrap());
}

fn status(w: &RubatoWorld) -> &str {
    w.status_out.as_deref().expect("status was run")
}

#[then(expr = "status says {string} for the clone of {string}")]
async fn then_status_line(w: &mut RubatoWorld, says: String, repo: String) {
    let dir = w.repos.get(&repo).cloned().expect("repo created");
    let want = format!("{}: {says}", dir.display());
    assert!(
        status(w).lines().any(|l| l.starts_with(&want)),
        "no line starting {want:?} in:\n{}",
        status(w)
    );
}

#[then(expr = "status ends with {string}")]
async fn then_status_ends(w: &mut RubatoWorld, last: String) {
    assert_eq!(
        status(w).lines().last(),
        Some(last.as_str()),
        "{}",
        status(w)
    );
}

#[then("status prints no token")]
async fn then_no_token(w: &mut RubatoWorld) {
    let ep = HookEndpoint::load(&w.state_dir).expect("an endpoint");
    assert!(
        !status(w).contains(&ep.token),
        "the token leaked:\n{}",
        status(w)
    );
    assert!(status(w).contains("hook header file: "), "{}", status(w));
}

#[then(expr = "status says the hook endpoint is {string}")]
async fn then_endpoint(w: &mut RubatoWorld, state: String) {
    let line = status(w)
        .lines()
        .find(|l| l.starts_with("hook endpoint: port "))
        .unwrap_or_else(|| panic!("no endpoint line in:\n{}", status(w)));
    assert!(line.ends_with(&format!("({state})")), "{line}");
}

#[when("the developer opts the launched session's worktree in by hand")]
async fn when_opt_in_launched(w: &mut RubatoWorld) {
    let cmd = w.last_launch.as_ref().unwrap();
    let wt = worktree_of(w, &cmd.issue.repo, &cmd.branch);
    opt_in_by_hand(w, &wt);
}

fn launched_settings(w: &RubatoWorld) -> PathBuf {
    let cmd = w.last_launch.as_ref().unwrap();
    w.state_dir
        .join("launches")
        .join(cmd.id.to_string())
        .join("hooks.json")
}

/// One event of the launched session, with both the launch's `--settings` hooks and the hand
/// opt-in in its worktree: Claude Code runs both (hooks merge across settings levels, ADR 0013).
#[when(
    expr = "the launched session fires {string} as session {string} through both its own hooks and the hand opt-in"
)]
async fn when_fire_both(w: &mut RubatoWorld, event: String, session: String) {
    let cmd = w.last_launch.as_ref().unwrap().clone();
    let wt = worktree_of(w, &cmd.issue.repo, &cmd.branch);
    let payload = serde_json::json!({
        "session_id": session,
        "hook_event_name": event,
        "cwd": wt.to_string_lossy(),
        "permission_mode": "acceptEdits",
    });
    for settings in [launched_settings(w), settings_path(&wt)] {
        run_hook(
            &hook_command(&settings, &event),
            &payload,
            Some(cmd.id.to_string()),
        );
    }
}

#[then(expr = "the server receives exactly {int} hook event(s) {string} for session {string}")]
async fn then_exactly(w: &mut RubatoWorld, n: usize, event: String, session: String) {
    // Long enough for a second post to arrive if one was made (curl -m 2).
    tokio::time::sleep(Duration::from_secs(3)).await;
    let got = w.server().received.lock().await;
    let count = got
        .iter()
        .filter(|e| {
            matches!(&e.message, CompanionMessage::Hook(h)
                if h.session_id == session
                    && serde_json::to_value(h.hook_event_name).unwrap().as_str() == Some(event.as_str()))
        })
        .count();
    assert_eq!(count, n, "{event} for {session} received {count} times");
}

#[given(expr = "the developer opts the worktree for branch {string} of {string} in by hand")]
async fn given_opt_in_worktree(w: &mut RubatoWorld, branch: String, repo: String) {
    let wt = worktree_of(w, &repo, &branch);
    opt_in_by_hand(w, &wt);
}

/// A session started by hand in that worktree fires a hook: the command from the worktree's
/// merged settings file, with no launch id.
#[when(
    expr = "Claude Code started by hand in the worktree for branch {string} of {string} fires the opted-in hook {string}"
)]
async fn when_fire_opted_in(w: &mut RubatoWorld, branch: String, repo: String, fixture: String) {
    let wt = worktree_of(w, &repo, &branch);
    let mut payload: serde_json::Value =
        serde_json::from_str(&RubatoWorld::hook_fixture(&fixture)).unwrap();
    payload["cwd"] = serde_json::Value::String(wt.to_string_lossy().into_owned());
    let event = payload["hook_event_name"].as_str().unwrap().to_owned();
    run_hook(&hook_command(&settings_path(&wt), &event), &payload, None);
}
