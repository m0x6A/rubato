//! Steps for `features/restart.feature`.

use crate::world::*;
use cucumber::{then, when};
use rubato_companion::kernel::config::HookEndpoint;
use rubato_protocol::CompanionMessage;
use std::time::Duration;

async fn hellos(w: &RubatoWorld) -> usize {
    w.server()
        .count(|m| matches!(m, CompanionMessage::Hello { .. }))
        .await
}

fn endpoint(w: &RubatoWorld) -> HookEndpoint {
    w.handles
        .as_ref()
        .expect("daemon running")
        .hook_endpoint
        .clone()
}

async fn stop(w: &mut RubatoWorld) {
    w.endpoint_before = Some(endpoint(w));
    w.stop_daemon().await;
}

/// Start, wait until connected and until the server has the new Hello, so a push is not lost.
async fn start_again(w: &mut RubatoWorld) {
    let before = hellos(w).await;
    w.start_daemon().await;
    let mut connected = w.handles.as_ref().unwrap().connected.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !*connected.borrow() {
            connected.changed().await.unwrap();
        }
    })
    .await
    .expect("daemon connected again");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while hellos(w).await <= before {
        assert!(tokio::time::Instant::now() < deadline, "no second hello");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[when("the companion restarts")]
async fn when_restarts(w: &mut RubatoWorld) {
    stop(w).await;
    start_again(w).await;
}

#[when("the companion stops")]
async fn when_stops(w: &mut RubatoWorld) {
    stop(w).await;
}

#[when("something else takes the companion's hook port")]
async fn when_port_taken(w: &mut RubatoWorld) {
    let port = w.endpoint_before.as_ref().unwrap().port;
    w.held_port =
        Some(std::net::TcpListener::bind(("127.0.0.1", port)).expect("the old port is free"));
}

#[when("the companion starts again")]
async fn when_starts_again(w: &mut RubatoWorld) {
    start_again(w).await;
}

#[then("the hook endpoint has the same port and token as before the restart")]
async fn then_same_endpoint(w: &mut RubatoWorld) {
    let before = w.endpoint_before.clone().unwrap();
    assert_eq!(endpoint(w), before);
    assert_eq!(HookEndpoint::load(&w.state_dir), Some(before));
}

#[then("the hook endpoint has a new port and a new token")]
async fn then_new_endpoint(w: &mut RubatoWorld) {
    let before = w.endpoint_before.clone().unwrap();
    let now = endpoint(w);
    assert_ne!(now.port, before.port);
    assert_ne!(now.token, before.token);
    assert_eq!(HookEndpoint::load(&w.state_dir), Some(now));
}

/// A snapshot received after the restart's Hello with the launch and `hooks_lost` as given.
async fn snapshot_after_restart(w: &RubatoWorld, lost: bool) {
    let id = w.last_launch.as_ref().unwrap().id;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let found = {
            let got = w.server().received.lock().await;
            // Only what the restarted companion sent: after its Hello, the last one.
            let start = got
                .iter()
                .rposition(|e| matches!(e.message, CompanionMessage::Hello { .. }))
                .unwrap_or(0);
            got[start..].iter().find_map(|e| match &e.message {
                CompanionMessage::Sessions { sessions } => sessions
                    .iter()
                    .find(|s| s.launch_id == Some(id))
                    .map(|s| s.hooks_lost),
                _ => None,
            })
        };
        match found {
            Some(l) if l == lost => return,
            Some(l) => panic!("the launch came back with hooks_lost {l}"),
            None => {}
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no snapshot with launch {id} after the restart"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[then("the server receives a sessions snapshot with the launch, its hooks not lost")]
async fn then_hooks_kept(w: &mut RubatoWorld) {
    snapshot_after_restart(w, false).await;
}

#[then("the server receives a sessions snapshot with the launch, its hooks lost")]
async fn then_hooks_lost(w: &mut RubatoWorld) {
    snapshot_after_restart(w, true).await;
}
