//! `POST`, `PUT` and `DELETE /api/queue`. Every write is for one lane and is accepted only from
//! that companion's paired user, while the pairing is not revoked (ADR 0004, ADR 0020).

use super::store::{self, Added};
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::pairing;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::queue::{QUEUE_MAX_PER_COMPANION, QueueOrder, QueueRequest, routes};
use uuid::Uuid;

const SLICE: &str = "queue";

/// The reason values of a refused queue write, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const NOT_OWN_COMPANION: &str = "not_own_companion";
    pub const REVOKED_COMPANION: &str = "revoked_companion";
    pub const QUEUE_FULL: &str = "queue_full";
    pub const ORDER_MISMATCH: &str = "order_mismatch";
}

type Refusal = (StatusCode, String);

/// The caller may write this lane: it is theirs and not revoked. Returns the revoked
/// companions, whose rows the write clears.
async fn own_lane(app: &App, user: &str, companion_id: Uuid) -> Result<Vec<Uuid>, Refusal> {
    let pairings = pairing::list_pairings(&app.db, None)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    let lane = pairings
        .iter()
        .find(|p| p.companion_id == companion_id && p.user == user);
    let refuse = |why: &'static str, text: &str| {
        app.reject(routes::QUEUE, why, user);
        Err((StatusCode::FORBIDDEN, text.to_owned()))
    };
    match lane {
        // Someone else's companion and one that does not exist answer the same.
        None => refuse(
            reason::NOT_OWN_COMPANION,
            "You can queue issues only on your own companions.",
        ),
        Some(p) if p.revoked_at.is_some() => refuse(
            reason::REVOKED_COMPANION,
            "This companion's pairing was revoked; pair it again to queue issues on it.",
        ),
        Some(_) => Ok(pairings
            .iter()
            .filter(|p| p.revoked_at.is_some())
            .map(|p| p.companion_id)
            .collect()),
    }
}

pub async fn add(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<QueueRequest>,
) -> Result<StatusCode, Refusal> {
    let revoked = own_lane(&app, &user, req.companion_id).await?;
    let outcome = store::add(
        &app.db,
        req.companion_id,
        &req.issue,
        &user,
        Utc::now(),
        QUEUE_MAX_PER_COMPANION,
        &revoked,
    )
    .await
    .map_err(|e| (internal(SLICE, e), String::new()))?;
    if outcome == Added::Full {
        app.reject(routes::QUEUE, reason::QUEUE_FULL, &user);
        return Err((
            StatusCode::CONFLICT,
            format!(
                "This lane already holds {QUEUE_MAX_PER_COMPANION} issues; start or remove one first."
            ),
        ));
    }
    app.notify("queue");
    Ok(StatusCode::CREATED)
}

pub async fn reorder(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<QueueOrder>,
) -> Result<StatusCode, Refusal> {
    let revoked = own_lane(&app, &user, req.companion_id).await?;
    let same = store::reorder(&app.db, req.companion_id, &req.issues, &revoked)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    if !same {
        app.reject(routes::QUEUE, reason::ORDER_MISMATCH, &user);
        return Err((
            StatusCode::CONFLICT,
            "The lane changed meanwhile: the order sent differs from the issues queued there. Nothing was reordered.".into(),
        ));
    }
    app.notify("queue");
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<QueueRequest>,
) -> Result<StatusCode, Refusal> {
    let revoked = own_lane(&app, &user, req.companion_id).await?;
    let removed = store::remove(&app.db, req.companion_id, &req.issue, &revoked)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    app.notify("queue");
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

/// `rubato.queue.depth` for every pairing, on every poll tick: 0 for an empty lane and for a
/// revoked one, because a gauge recorded only on change keeps its last value.
pub async fn record_depth(app: &App) {
    let (pairings, queued) = match (
        pairing::list_pairings(&app.db, None).await,
        store::all(&app.db).await,
    ) {
        (Ok(p), Ok(q)) => (p, q),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(error = %e, "reading the queue depth failed");
            return;
        }
    };
    for p in &pairings {
        let depth = if p.revoked_at.is_some() {
            0
        } else {
            queued
                .iter()
                .filter(|q| q.companion_id == p.companion_id)
                .count()
        };
        app.metrics.queue_depth.record(
            depth as u64,
            &[KeyValue::new(
                attr::COMPANION_ID,
                p.companion_id.to_string(),
            )],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::IssueRef;
    use rubato_protocol::otel::metric;
    use std::sync::Arc;

    async fn pairing(app: &App, user: &str, revoked: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO pairings (companion_id, user, hostname, token_hash, created_at, revoked_at) VALUES (?, ?, 'h', ?, '2026-11-01T00:00:00Z', ?)")
            .bind(id.to_string())
            .bind(user)
            .bind(id.to_string())
            .bind(revoked.then_some("2026-11-02T00:00:00Z"))
            .execute(&app.db)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn depth_is_recorded_for_every_pairing_with_zeros() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            Arc::new(FakeGitHub::new()),
        );
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let busy = pairing(&app, "m0x6A", false).await;
        let empty = pairing(&app, "alice", false).await;
        let revoked = pairing(&app, "bob", true).await;
        for (lane, n) in [(busy, 1), (busy, 2), (revoked, 3)] {
            store::add(
                &app.db,
                lane,
                &IssueRef::new("o/r", n),
                "x",
                Utc::now(),
                50,
                &[],
            )
            .await
            .unwrap();
        }
        record_depth(&app).await;
        let depth = |c: Uuid| {
            reader
                .points(metric::QUEUE_DEPTH)
                .into_iter()
                .find(|p| p.attrs.get(attr::COMPANION_ID) == Some(&c.to_string()))
                .map(|p| p.value)
        };
        assert_eq!(depth(busy), Some(2));
        assert_eq!(depth(empty), Some(0), "an empty lane is recorded as 0");
        assert_eq!(depth(revoked), Some(0), "a revoked lane counts nothing");
        // The lane empties: the next tick says 0, not the last value.
        store::remove(&app.db, busy, &IssueRef::new("o/r", 1), &[])
            .await
            .unwrap();
        store::remove(&app.db, busy, &IssueRef::new("o/r", 2), &[])
            .await
            .unwrap();
        record_depth(&app).await;
        assert_eq!(depth(busy), Some(0));
        println!("checked 3 pairings over 2 ticks");
    }
}
