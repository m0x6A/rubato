//! Steps for `features/stale.feature`.

use super::pairing::given_paired;
use crate::world::*;
use cucumber::{given, then, when};
use rubato_protocol::CompanionMessage;
use std::time::Duration;

#[given(
    expr = "a paired companion for user {string} with a session stale threshold of {int} second(s)"
)]
async fn given_paired_stale(w: &mut RubatoWorld, user: String, secs: u64) {
    given_paired(w, user).await;
    w.workspace.companion.limits.session_stale_secs = secs;
    w.server().limits.lock().unwrap().session_stale_secs = secs;
}

#[when(expr = "{int} seconds pass")]
async fn when_seconds_pass(_w: &mut RubatoWorld, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

#[then(expr = "the companion reports that session with effective status {string}")]
async fn then_effective(w: &mut RubatoWorld, status: String) {
    let stale = w.workspace.companion.limits.session_stale_secs;
    let s = w
        .handles
        .as_ref()
        .unwrap()
        .status_of("0f8fad5b-d9cb-469f-a165-70867728950e", stale)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(s).unwrap().as_str(),
        Some(status.as_str())
    );
}

#[then(expr = "the server receives a stale event for session {string}")]
async fn then_stale(w: &mut RubatoWorld, session: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Stale(s) if s.session_id.as_deref() == Some(session.as_str()) => {
                Some(s.silent_for_secs)
            }
            _ => None,
        })
        .await;
    assert!(got.is_some_and(|s| s >= 1), "{got:?}");
}

#[then(expr = "the server receives exactly {int} stale event(s) within {int} more second(s)")]
async fn then_stale_count(w: &mut RubatoWorld, n: usize, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    assert_eq!(
        w.server()
            .count(|m| matches!(m, CompanionMessage::Stale(_)))
            .await,
        n
    );
}
