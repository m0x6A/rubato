//! Steps for `features/headless.feature` (ADR 0062). The headless stand-in `claude` writes
//! stream-json, records its argv and pid beside itself, and posts no hooks: the companion's own
//! child process handling is all that is under test.

use crate::world::*;
use cucumber::{given, then, when};
use rubato_protocol::{
    ClientKind, CompanionMessage, HookEventKind, LaunchResult, ServerMessage, SessionMessage,
    SessionMessageKind, SessionStatus, StateSource,
};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use uuid::Uuid;

/// The turn a scenario plants as left over by a previous run; kept to reap it after the kill.
static PLANTED: Mutex<Option<std::process::Child>> = Mutex::new(None);

fn log_path(w: &RubatoWorld) -> PathBuf {
    w.fake_bin.join("headless.log")
}

fn pids_path(w: &RubatoWorld) -> PathBuf {
    w.fake_bin.join("headless.pids")
}

#[given("the companion is in headless mode with tmux unusable")]
async fn given_headless(w: &mut RubatoWorld) {
    let d = w.fake_bin.display().to_string();
    let script = format!(
        "#!/bin/sh\n\
         echo \"ARGS: $*\" >> '{d}/headless.log'\n\
         echo $$ >> '{d}/headless.pids'\n\
         echo '{{\"type\":\"system\",\"subtype\":\"init\"}}'\n\
         if [ -f '{d}/headless-sleep' ]; then sleep \"$(cat '{d}/headless-sleep')\"; fi\n\
         echo '{{\"type\":\"result\",\"result\":\"PONG\",\"total_cost_usd\":0.01,\"is_error\":false}}'\n"
    );
    let claude = w.fake_bin.join("claude-headless");
    std::fs::write(&claude, script).unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let c = &mut w.workspace.companion;
    c.mode = ClientKind::Headless;
    c.claude_command = claude.display().to_string();
    // Not needed (ADR 0062); a call fails at once.
    c.tmux_command = "/nonexistent/tmux".into();
}

#[given(expr = "the headless stand-in takes {int} seconds per turn")]
async fn given_sleep(w: &mut RubatoWorld, secs: u64) {
    std::fs::write(w.fake_bin.join("headless-sleep"), secs.to_string()).unwrap();
}

fn launched_session(w: &RubatoWorld) -> String {
    w.last_launch
        .as_ref()
        .expect("a launch")
        .session_id
        .to_string()
}

async fn wait_stop_hook(w: &RubatoWorld, n: usize) -> Vec<Option<String>> {
    let session = launched_session(w);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let replies: Vec<Option<String>> = w
            .server()
            .received
            .lock()
            .await
            .iter()
            .filter_map(|e| match &e.message {
                CompanionMessage::Hook(h)
                    if h.session_id == session && h.hook_event_name == HookEventKind::Stop =>
                {
                    Some(
                        h.payload
                            .as_ref()
                            .and_then(|p| p.get("last_assistant_message"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    )
                }
                _ => None,
            })
            .collect();
        if replies.len() >= n {
            return replies;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {n} Stop hook(s) for {session}, got {}",
            replies.len()
        );
        let _ =
            tokio::time::timeout(Duration::from_millis(200), w.server().notify.notified()).await;
    }
}

#[given(
    expr = "the server has launched issue {int} of {string} with slug {string} for user {string} and its first turn ended"
)]
async fn given_launched_turn_ended(
    w: &mut RubatoWorld,
    issue: u64,
    repo: String,
    slug: String,
    user: String,
) {
    let cmd = w.launch_cmd(&user, &repo, issue, &slug);
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd.clone()));
    let started = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started { command_id, .. })
                if *command_id == cmd.id =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(started.is_some(), "the launch did not start");
    wait_stop_hook(w, 1).await;
}

fn turns(w: &RubatoWorld) -> Vec<String> {
    std::fs::read_to_string(log_path(w))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[then(
    expr = "the headless stand-in ran {int} turn(s), the first with the launch's prompt and session id"
)]
async fn then_first_turn(w: &mut RubatoWorld, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let t = loop {
        let t = turns(w);
        if t.len() >= n || tokio::time::Instant::now() > deadline {
            break t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    println!("checked {} headless turn(s)", t.len());
    assert_eq!(t.len(), n, "{t:?}");
    let session = launched_session(w);
    let first = &t[0];
    assert!(
        first.contains(&format!("--session-id {session}")),
        "{first}"
    );
    assert!(first.contains(" -p "), "{first}");
    assert!(first.contains("--output-format stream-json"), "{first}");
    assert!(
        first.contains("--settings "),
        "hooks settings passed: {first}"
    );
}

#[then(expr = "the headless stand-in ran {int} turns, the last resuming the session with {string}")]
async fn then_resumed(w: &mut RubatoWorld, n: usize, text: String) {
    let session = launched_session(w);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let t = loop {
        let t = turns(w);
        if t.len() >= n || tokio::time::Instant::now() > deadline {
            break t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    println!("checked {} headless turn(s)", t.len());
    assert_eq!(t.len(), n, "never two turns at once, nor a lost one: {t:?}");
    let last = t.last().unwrap();
    assert!(
        last.contains(&format!("--resume {session} {text}")),
        "{last}"
    );
}

#[then(
    expr = "the server receives a hook event {string} for the launched session with the reply {string}"
)]
async fn then_reply(w: &mut RubatoWorld, event: String, reply: String) {
    assert_eq!(event, "Stop");
    let replies = wait_stop_hook(w, 1).await;
    assert_eq!(replies[0].as_deref(), Some(reply.as_str()));
}

#[then(
    expr = "the server receives a sessions snapshot for the launched session from source {string} with status {string}"
)]
async fn then_snapshot(w: &mut RubatoWorld, source: String, status: String) {
    let session = launched_session(w);
    let source: StateSource = serde_json::from_value(serde_json::json!(source)).unwrap();
    let status: SessionStatus = serde_json::from_value(serde_json::json!(status)).unwrap();
    let got = w
        .server()
        .wait_for(Duration::from_secs(20), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| {
                    s.session_id.as_deref() == Some(session.as_str())
                        && s.source == source
                        && s.status == status
                })
                .map(|_| ()),
            _ => None,
        })
        .await;
    assert!(got.is_some(), "no snapshot with {source:?} and {status:?}");
}

#[when("the owner stops the launched session")]
async fn when_stop(w: &mut RubatoWorld) {
    // The turn is running before the stop goes out.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while turns(w).is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the first turn never started"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let cmd = w.last_launch.clone().expect("a launch");
    let m = SessionMessage {
        id: Uuid::new_v4(),
        user: cmd.user.clone(),
        launch_id: cmd.id,
        session_id: Some(cmd.session_id.to_string()),
        issue: cmd.issue.clone(),
        kind: SessionMessageKind::Stop {
            requested_by: cmd.user.clone(),
        },
        created_at: chrono::Utc::now(),
    };
    w.last_message = Some(m.clone());
    w.server().push(ServerMessage::SessionMessage(m));
}

#[when("the server says the pairing was revoked")]
async fn when_revoked(w: &mut RubatoWorld) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while turns(w).is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the first turn never started"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    w.server().push(ServerMessage::Error {
        code: "auth".into(),
        message: "pairing revoked".into(),
    });
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[then("no headless stand-in turn is running")]
async fn then_none_running(w: &mut RubatoWorld) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let pids = std::fs::read_to_string(pids_path(w)).unwrap_or_default();
        let running: Vec<&str> = pids.lines().filter(|p| alive(p)).collect();
        if running.is_empty() {
            println!(
                "checked {} stand-in turn(s): none running",
                pids.lines().count()
            );
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "still running: {running:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[given("a headless turn the previous run left running")]
async fn given_planted(w: &mut RubatoWorld) {
    let child = std::process::Command::new("sh")
        .args(["-c", "sleep 60"])
        .process_group(0)
        .spawn()
        .unwrap();
    // The record is what `turns.json` holds (rubato_companion::sessions::TurnRecord).
    let record = rubato_companion::sessions::TurnRecord {
        launch_id: Uuid::new_v4(),
        pgid: child.id(),
        started_at: chrono::Utc::now(),
    };
    std::fs::create_dir_all(&w.state_dir).unwrap();
    std::fs::write(
        w.state_dir.join("turns.json"),
        serde_json::to_vec(&[record]).unwrap(),
    )
    .unwrap();
    *PLANTED.lock().unwrap() = Some(child);
}

#[then("the planted turn is no longer running")]
async fn then_planted_gone(_w: &mut RubatoWorld) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let exited = PLANTED
            .lock()
            .unwrap()
            .as_mut()
            .expect("a planted turn")
            .try_wait()
            .unwrap()
            .is_some();
        if exited {
            PLANTED.lock().unwrap().take();
            return;
        }
        if tokio::time::Instant::now() > deadline {
            let mut c = PLANTED.lock().unwrap().take().unwrap();
            let _ = c.kill();
            let _ = c.wait();
            panic!("the planted turn still runs: the companion did not reap it at start");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The pair a headless turn ends with carries one `prompt_id`, as Claude Code's own hooks do:
/// the server's session log drops a hook without one (`no_prompt_id`), and a room turn's reply
/// is found through it (plan 122 §D8).
#[then("the launched session's UserPromptSubmit and Stop share one prompt id")]
async fn then_one_prompt_id(w: &mut RubatoWorld) {
    wait_stop_hook(w, 1).await;
    let session = launched_session(w);
    let ids =
        |kind: HookEventKind, got: &[rubato_protocol::CompanionEnvelope]| -> Vec<Option<String>> {
            got.iter()
                .filter_map(|e| match &e.message {
                    CompanionMessage::Hook(h)
                        if h.session_id == session && h.hook_event_name == kind =>
                    {
                        Some(
                            h.payload
                                .as_ref()
                                .and_then(|p| p.get("prompt_id"))
                                .and_then(|v| v.as_str())
                                .map(str::to_owned),
                        )
                    }
                    _ => None,
                })
                .collect()
        };
    let got = w.server().received.lock().await;
    let prompts = ids(HookEventKind::UserPromptSubmit, &got);
    let stops = ids(HookEventKind::Stop, &got);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(stops.len(), 1, "{stops:?}");
    assert!(prompts[0].is_some(), "UserPromptSubmit without a prompt_id");
    assert_eq!(prompts[0], stops[0], "the pair has two prompt ids");
}
