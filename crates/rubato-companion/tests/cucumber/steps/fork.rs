//! Steps for `features/fork.feature`: room turns, typed while planning, run in a read-only fork
//! while building (ADR 0057, plan 122 §D8).

use super::session_messages::{launched_session, send};
use crate::world::*;
use cucumber::{given, then, when};
use rubato_protocol::{
    CompanionMessage, RoomKey, RoomReply, RoomTurnMode, SessionMessage, SessionMessageKind,
    room_turn_text,
};
use std::time::Duration;
use uuid::Uuid;

fn turn_text() -> String {
    room_turn_text(
        &[
            ("bob".to_owned(), "plain message".to_owned()),
            ("alice".to_owned(), "@m0x6A-agent why b?".to_owned()),
        ],
        16000,
    )
}

fn room_turn(w: &RubatoWorld, mode: RoomTurnMode, fork: Option<String>) -> SessionMessage {
    let cmd = w.last_launch.as_ref().expect("a launch");
    SessionMessage {
        id: Uuid::new_v4(),
        user: cmd.user.clone(),
        launch_id: cmd.id,
        session_id: Some(launched_session(w)),
        issue: cmd.issue.clone(),
        kind: SessionMessageKind::RoomTurn {
            room: RoomKey {
                repo: cmd.issue.repo.clone(),
                number: cmd.issue.number,
                branch: cmd.branch.clone(),
            },
            turn_id: Uuid::new_v4(),
            addressed_by: "alice".into(),
            text: turn_text(),
            mode,
            fork_session_id: fork,
        },
        created_at: chrono::Utc::now(),
    }
}

#[given(expr = "a fork turn costs {float} dollars")]
async fn given_cost(w: &mut RubatoWorld, usd: f64) {
    std::fs::write(w.root.join("fork-cost"), format!("{usd}")).unwrap();
}

#[when("the server sends the launched session a room turn while planning")]
async fn when_session_turn(w: &mut RubatoWorld) {
    let m = room_turn(w, RoomTurnMode::Session, None);
    send(w, m);
}

#[when("the server sends the launched session a room turn while building")]
async fn when_fork_turn(w: &mut RubatoWorld) {
    let m = room_turn(w, RoomTurnMode::Fork, None);
    send(w, m);
}

#[when("the server sends the launched session a room turn resuming that fork")]
async fn when_fork_again(w: &mut RubatoWorld) {
    let fork = Some(format!("fork-of-{}", launched_session(w)));
    let m = room_turn(w, RoomTurnMode::Fork, fork);
    send(w, m);
}

async fn reply_to_last(w: &RubatoWorld) -> RoomReply {
    let id = w.last_message.as_ref().expect("a turn sent").id;
    w.server()
        .wait_for(Duration::from_secs(20), |m| match m {
            CompanionMessage::RoomReply(r) if r.message_id == id => Some(r.clone()),
            _ => None,
        })
        .await
        .expect("a room reply for that turn")
}

#[then(
    expr = "the server receives a room reply {string} costing {float} from fork session {string}"
)]
async fn then_reply(w: &mut RubatoWorld, text: String, usd: f64, fork: String) {
    let r = reply_to_last(w).await;
    let fork = fork.replace("<session>", &launched_session(w));
    assert_eq!(r.error, None);
    assert_eq!(r.text, text);
    assert_eq!(r.cost_usd, Some(usd));
    assert_eq!(r.fork_session_id.as_deref(), Some(fork.as_str()));
}

#[then(expr = "the server receives a room reply with the error {string}")]
async fn then_reply_error(w: &mut RubatoWorld, part: String) {
    let r = reply_to_last(w).await;
    let e = r.error.expect("an error");
    assert!(e.contains(&part), "{e}");
    assert!(r.text.is_empty(), "no answer text with an error");
}

fn fork_log(w: &RubatoWorld) -> String {
    let cmd = w.last_launch.as_ref().unwrap();
    let wt = worktree_of(w, &cmd.issue.repo, &cmd.branch);
    std::fs::read_to_string(wt.join(".claude-fork.log")).unwrap_or_default()
}

/// The ARGS line of each fork run, in order.
fn fork_runs(w: &RubatoWorld) -> Vec<String> {
    fork_log(w)
        .lines()
        .filter_map(|l| l.strip_prefix("ARGS: ").map(str::to_owned))
        .collect()
}

#[then(expr = "fork run {int} resumed the launched session with {string}")]
async fn then_run_resumed_session(w: &mut RubatoWorld, n: usize, flags: String) {
    let runs = fork_runs(w);
    let run = runs
        .get(n - 1)
        .unwrap_or_else(|| panic!("no fork run {n}: {runs:?}"));
    let want = format!("--resume {}", launched_session(w));
    assert!(run.contains(&want), "{run}");
    for f in flags.split_whitespace() {
        assert!(
            run.split_whitespace().any(|a| a == f),
            "{f} missing from {run}"
        );
    }
}

#[then(expr = "fork run {int} resumed the fork without {string}")]
async fn then_run_resumed_fork(w: &mut RubatoWorld, n: usize, flag: String) {
    let runs = fork_runs(w);
    let run = runs
        .get(n - 1)
        .unwrap_or_else(|| panic!("no fork run {n}: {runs:?}"));
    let want = format!("--resume fork-of-{}", launched_session(w));
    assert!(run.contains(&want), "{run}");
    assert!(
        !run.split_whitespace().any(|a| a == flag),
        "{flag} in {run}"
    );
}

#[then("the fork ran with no Rubato hooks and the turn on its stdin")]
async fn then_no_hooks(w: &mut RubatoWorld) {
    let log = fork_log(w);
    assert!(
        log.contains("--settings {\"disableAllHooks\":true}"),
        "{log}"
    );
    assert!(!log.contains("hooks.json"), "{log}");
    // Nothing of the launch. The one Rubato variable a fork carries is the marker its owner's
    // `rubato-companion mcp` bridge stands down on, so a fork gets no Rubato MCP (ADR 0086).
    assert!(
        !log.lines().any(|l| {
            (l.starts_with("RUBATO_") && !l.starts_with("RUBATO_FORK_TURN=1"))
                || l.starts_with("TMUX=")
        }),
        "{log}"
    );
    assert!(log.lines().any(|l| l == "RUBATO_FORK_TURN=1"), "{log}");
    assert!(
        log.contains("alice: @m0x6A-agent why b?"),
        "the turn on stdin: {log}"
    );
}

#[then(expr = "there was {int} fork run(s)")]
async fn then_runs(w: &mut RubatoWorld, n: usize) {
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fork_runs(w).len(), n, "{}", fork_log(w));
}

#[then("the server receives a spend cap hit for the launched issue")]
async fn then_cap_hit(w: &mut RubatoWorld) {
    let issue = w.last_launch.as_ref().unwrap().issue.clone();
    let hit = w
        .server()
        .wait_for(Duration::from_secs(20), |m| match m {
            CompanionMessage::CapHit(h) if h.issue == issue => Some(h.clone()),
            _ => None,
        })
        .await
        .expect("a cap hit");
    assert_eq!(hit.kind, rubato_protocol::CapKind::Spend);
}

#[then("the stand-in claude was typed the room turn")]
async fn then_typed_turn(w: &mut RubatoWorld) {
    let want = format!("{}\n", turn_text());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while typed(w) != want && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(typed(w), want);
}
