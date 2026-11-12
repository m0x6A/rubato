//! Steps for `features/launch-timeout.feature` (plan 40).

use crate::world::*;
use cucumber::{given, then, when};
use rubato_protocol::{CompanionMessage, LaunchResult};
use std::path::PathBuf;
use std::time::Duration;

fn command_id(r: &LaunchResult) -> uuid::Uuid {
    match r {
        LaunchResult::Started { command_id, .. }
        | LaunchResult::Refused { command_id, .. }
        | LaunchResult::Failed { command_id, .. } => *command_id,
    }
}

fn marker(w: &RubatoWorld) -> PathBuf {
    w.root.join("tmux-ide-hung")
}

/// Wraps the real tmux-ide. On `<sub> <verb>` it records the pid of a 60 s `sleep` and waits for
/// it; anything else goes to the real one. Written by a child `sh` so this process never holds
/// it open for writing.
#[given(expr = "a tmux-ide that hangs on {string}")]
async fn given_hanging_tmux_ide(w: &mut RubatoWorld, call: String) {
    let (sub, verb) = call
        .split_once(' ')
        .expect("two words, like \"worktree create\"");
    let script = w.fake_bin.join("tmux-ide-hanging");
    let body = format!(
        "#!/bin/sh\nif [ \"$1\" = '{sub}' ] && [ \"$2\" = '{verb}' ]; then\n  sleep 60 &\n  echo $! > '{}'\n  wait\n  exit 1\nfi\nexec tmux-ide \"$@\"\n",
        marker(w).display()
    );
    let ok = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
        .arg("sh")
        .arg(&body)
        .arg(&script)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    w.workspace.companion.tmux_ide_command = script.to_string_lossy().into_owned();
}

#[given(expr = "a command timeout of {int} seconds")]
async fn given_timeout(w: &mut RubatoWorld, secs: u64) {
    w.workspace.companion.command_timeout_secs = secs;
}

#[when("the launch is inside the hanging tmux-ide")]
async fn when_inside(w: &mut RubatoWorld) {
    let m = marker(w);
    for _ in 0..300 {
        if std::fs::read_to_string(&m).is_ok_and(|t| !t.trim().is_empty()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the launch never reached the hanging tmux-ide");
}

#[then(expr = "the server receives a failed launch result whose error contains {string}")]
async fn then_failed(w: &mut RubatoWorld, text: String) {
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(r) if command_id(r) == id => Some(r.clone()),
            _ => None,
        })
        .await;
    match got {
        Some(LaunchResult::Failed { error, .. }) => {
            assert!(error.contains(&text), "{error}");
        }
        other => panic!("expected a failed launch, got {other:?}"),
    }
}

/// Order on the wire: the companion's envelopes arrive in the order it sent them.
#[then("the hook event reached the server before the launch result")]
async fn then_hook_first(w: &mut RubatoWorld) {
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w.server().received.lock().await;
    let at = |pred: &dyn Fn(&CompanionMessage) -> bool| got.iter().position(|e| pred(&e.message));
    let hook = at(&|m| matches!(m, CompanionMessage::Hook(_))).expect("a hook event");
    let result = at(&|m| matches!(m, CompanionMessage::Launch(r) if command_id(r) == id))
        .expect("a launch result");
    assert!(
        hook < result,
        "the hook (#{hook}) waited for the launch (#{result})"
    );
}

#[then("the hanging tmux-ide is no longer running")]
async fn then_gone(w: &mut RubatoWorld) {
    let pid = std::fs::read_to_string(marker(w)).expect("the hanging call ran");
    for _ in 0..100 {
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the hanging tmux-ide's child {pid} still runs");
}
