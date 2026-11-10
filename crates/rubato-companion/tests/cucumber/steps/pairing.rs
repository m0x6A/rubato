//! Steps for `features/pairing.feature (and the setup steps every feature shares)`.

use crate::world::*;
use cucumber::{given, then, when};
use rubato_companion::kernel::config::State;
use rubato_companion::kernel::tmux_ide::version_at_least;
use rubato_protocol::CompanionMessage;
use std::time::Duration;
use uuid::Uuid;

#[given(expr = "a test server that approves pairing for user {string}")]
async fn given_server(w: &mut RubatoWorld, user: String) {
    w.ensure_server(&user).await;
}

#[given("no state file")]
async fn given_no_state(w: &mut RubatoWorld) {
    let _ = std::fs::remove_file(State::path(&w.state_dir));
}

#[given(expr = "a paired companion for user {string}")]
pub(crate) async fn given_paired(w: &mut RubatoWorld, user: String) {
    w.ensure_server(&user).await;
    let s = w.server().clone();
    State {
        companion_id: Uuid::new_v4(),
        server_url: s.url.clone(),
        user,
        pairing_token: s.token.clone(),
        last_seq: 0,
        last_server_seq: 0,
    }
    .save(&w.state_dir)
    .unwrap();
}

#[given("the companion daemon is running and connected")]
async fn given_daemon(w: &mut RubatoWorld) {
    w.start_daemon().await;
    let mut connected = w.handles.as_ref().unwrap().connected.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !*connected.borrow() {
            connected.changed().await.unwrap();
        }
    })
    .await
    .expect("daemon connected to the test server");
    // The client reports connected once its hello is sent; the server handler subscribes to the
    // push channel before reading. Wait for the hello to be processed so a pushed launch is not
    // lost in that gap.
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| {
            matches!(m, CompanionMessage::Hello { .. }).then_some(())
        })
        .await;
    assert!(got.is_some(), "server processed the hello");
}

#[when("the companion pairs with the server")]
async fn when_pair(w: &mut RubatoWorld) {
    let s = w.server().clone();
    let paired = rubato_companion::pairing::pair(
        &s.url,
        Uuid::new_v4(),
        "testhost",
        "0.1.0-test",
        |start| assert_eq!(start.user_code, "ABCD-1234"),
    )
    .await
    .unwrap();
    State {
        companion_id: Uuid::new_v4(),
        server_url: s.url.clone(),
        user: paired.user,
        pairing_token: paired.token,
        last_seq: 0,
        last_server_seq: 0,
    }
    .save(&w.state_dir)
    .unwrap();
}

#[when("the companion is asked to run")]
async fn when_run_unpaired(w: &mut RubatoWorld) {
    w.run_error = match State::load(&w.state_dir).unwrap() {
        None => Some("not paired: run `rubato-companion pair --server <url>` first".into()),
        Some(_) => None,
    };
}

#[when("the companion daemon starts")]
async fn when_daemon_starts(w: &mut RubatoWorld) {
    w.start_daemon().await;
}

#[then("the state file holds the pairing token with mode 0600")]
async fn then_state_private(w: &mut RubatoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let p = State::path(&w.state_dir);
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let s = State::load(&w.state_dir).unwrap().unwrap();
    assert_eq!(s.pairing_token, w.server().token);
}

#[then(expr = "the state file names user {string}")]
async fn then_state_user(w: &mut RubatoWorld, user: String) {
    assert_eq!(State::load(&w.state_dir).unwrap().unwrap().user, user);
}

#[then(expr = "it fails with an error mentioning {string}")]
async fn then_fails(w: &mut RubatoWorld, word: String) {
    let e = w.run_error.clone().expect("an error");
    assert!(e.contains(&word), "{e}");
}

#[then(expr = "the server receives a hello for user {string} with protocol version {int}")]
async fn then_hello(w: &mut RubatoWorld, user: String, version: u32) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hello {
                user: u,
                protocol_version,
                ..
            } if *u == user && *protocol_version == version => Some(()),
            _ => None,
        })
        .await;
    assert!(got.is_some(), "no matching hello");
}

#[then("the server receives a heartbeat naming the tmux-ide version")]
async fn then_heartbeat(w: &mut RubatoWorld) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Heartbeat(h) => h.tmux_ide_version.clone(),
            _ => None,
        })
        .await;
    let v = got.expect("a heartbeat with tmux-ide version");
    assert!(version_at_least(&v, "2.9.0"), "{v}");
}

#[when("the companion is told to shut down")]
async fn when_shutdown(w: &mut RubatoWorld) {
    w.shutdown
        .as_ref()
        .expect("daemon started")
        .send(true)
        .unwrap();
}

#[then(expr = "the daemon returns Ok within {int} second(s)")]
async fn then_returns_ok(w: &mut RubatoWorld, secs: u64) {
    let join = w.join.take().expect("daemon started");
    let r = tokio::time::timeout(std::time::Duration::from_secs(secs), join)
        .await
        .unwrap_or_else(|_| panic!("the daemon did not return within {secs} s"));
    r.expect("the runtime task panicked")
        .expect("the runtime returned an error");
}

#[then(expr = "the companion logs no {string} event")]
async fn then_no_event(_w: &mut RubatoWorld, text: String) {
    assert!(!crate::logs::contains(&text), "found a {text:?} log line");
}
