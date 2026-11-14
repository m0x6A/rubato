//! Tables `spend_sessions` and `spend_points` (ADR 0037, design 103 §2).
//!
//! - `spend_sessions` holds the latest spend per session, keyed by the companion id of the
//!   pairing the snapshot arrived on (never the id a session claims) and the session id, with the
//!   issue it was first recorded under. Read by the point statement in `record`.
//! - `spend_points` holds, per issue and 15-minute bucket, the issue's total over every session
//!   it has had. Read by `series`.

use super::downsample::{MAX_POINTS, downsample};
use crate::kernel::db::parse_ts;
use crate::kernel::telemetry::Metrics;
use anyhow::Result;
use chrono::{DateTime, DurationRound, SecondsFormat, TimeDelta, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{IssueRef, SessionState, SpendPoint};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use uuid::Uuid;

/// Width of one stored point (plan 103, A3).
const BUCKET_MINUTES: i64 = 15;

/// One fixed-width form, so `at` and `bucket` compare and sort as text.
fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn bucket(t: DateTime<Utc>) -> DateTime<Utc> {
    t.duration_trunc(TimeDelta::minutes(BUCKET_MINUTES))
        .unwrap_or(t)
}

#[derive(Debug, Default)]
struct Counts {
    stored: u64,
    unchanged: u64,
    rejected: u64,
}

/// Record the spend of one companion's `Sessions` snapshot, received at `now`. `companion_id`
/// is the pairing's, never one a session claims. Counts each session under
/// `rubato.spend.points`; an error counts `failed` once and is returned for the caller to log.
pub async fn record(
    db: &SqlitePool,
    metrics: &Metrics,
    companion_id: Uuid,
    sessions: &[SessionState],
    now: DateTime<Utc>,
) -> Result<()> {
    let mut counts = Counts::default();
    let result = write(db, companion_id, sessions, now, &mut counts).await;
    for (outcome, n) in [
        ("stored", counts.stored),
        ("unchanged", counts.unchanged),
        ("rejected", counts.rejected),
        ("failed", u64::from(result.is_err())),
    ] {
        if n > 0 {
            metrics
                .spend_points
                .add(n, &[KeyValue::new(attr::OUTCOME, outcome)]);
        }
    }
    result
}

async fn write(
    db: &SqlitePool,
    companion_id: Uuid,
    sessions: &[SessionState],
    now: DateTime<Utc>,
    counts: &mut Counts,
) -> Result<()> {
    let at = stamp(now);
    let bucket = stamp(bucket(now));
    for s in sessions {
        let (Some(session_id), Some(issue), Some(usd)) = (&s.session_id, &s.issue, s.spend_usd)
        else {
            continue;
        };
        if !usd.is_finite() || usd < 0.0 {
            counts.rejected += 1;
            continue;
        }
        // A session keeps the issue it was first recorded under (plan A9): the issue is not
        // updated on conflict, and the point below is written for the row's issue.
        let row = sqlx::query(
            "INSERT INTO spend_sessions (companion_id, session_id, repo, number, usd, at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(companion_id, session_id) DO UPDATE SET usd = excluded.usd, at = excluded.at \
             WHERE excluded.usd <> spend_sessions.usd AND excluded.at >= spend_sessions.at \
             RETURNING repo, number",
        )
        .bind(companion_id.to_string())
        .bind(session_id)
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .bind(usd)
        .bind(&at)
        .fetch_optional(db)
        .await?;
        let Some(row) = row else {
            counts.unchanged += 1;
            continue;
        };
        let repo: String = row.get("repo");
        let number: i64 = row.get("number");
        // One statement, so the total is read and written together: whichever writer runs it
        // last sums every session row there is (design 103 §2, Concurrency).
        sqlx::query(
            "INSERT INTO spend_points (repo, number, bucket, usd, at) \
             SELECT ?, ?, ?, SUM(usd), ? FROM spend_sessions WHERE repo = ? AND number = ? \
             ON CONFLICT(repo, number, bucket) DO UPDATE SET usd = excluded.usd, at = excluded.at \
             WHERE excluded.at >= spend_points.at",
        )
        .bind(&repo)
        .bind(number)
        .bind(&bucket)
        .bind(&at)
        .bind(&repo)
        .bind(number)
        .execute(db)
        .await?;
        counts.stored += 1;
    }
    Ok(())
}

/// Every issue's series, oldest first, downsampled to at most `MAX_POINTS`. Each point is at
/// the start of its bucket.
pub async fn series(db: &SqlitePool) -> Result<HashMap<IssueRef, Vec<SpendPoint>>> {
    let rows = sqlx::query(
        "SELECT repo, number, bucket, usd FROM spend_points ORDER BY repo, number, bucket",
    )
    .fetch_all(db)
    .await?;
    let mut out: HashMap<IssueRef, Vec<SpendPoint>> = HashMap::new();
    for r in rows {
        let issue = IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64);
        out.entry(issue).or_default().push(SpendPoint {
            at: parse_ts(&r.get::<String, _>("bucket")),
            usd: r.get("usd"),
        });
    }
    Ok(out
        .into_iter()
        .map(|(issue, points)| (issue, downsample(points, MAX_POINTS)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::metric_reader;
    use chrono::{Duration, TimeZone};
    use rubato_protocol::otel::{attr, metric};
    use rubato_protocol::{SessionStatus, StateSource};
    use sqlx::Row;

    async fn db() -> (SqlitePool, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        (crate::kernel::db::connect(&url).await.unwrap(), tmp)
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 8, 0, 0).unwrap()
    }

    fn issue() -> IssueRef {
        IssueRef::new("m0x6A/rubato", 103)
    }

    fn session(id: &str, usd: Option<f64>) -> SessionState {
        SessionState {
            session_id: Some(id.into()),
            companion_id: Uuid::nil(),
            user: "m0x6A".into(),
            tmux: None,
            cwd: "/w".into(),
            worktree: None,
            branch: Some("issue/103-x".into()),
            repo: Some("m0x6A/rubato".into()),
            issue: Some(issue()),
            status: SessionStatus::Working,
            source: StateSource::Hook,
            last_seen: t0(),
            spend_usd: usd,
            branch_drift: None,
        }
    }

    fn count(reader: &metric_reader::Reader, outcome: &str) -> u64 {
        reader
            .points(metric::SPEND_POINTS)
            .iter()
            .filter(|p| p.attrs[attr::OUTCOME] == outcome)
            .map(|p| p.value)
            .sum()
    }

    async fn rows(db: &SqlitePool, table: &str) -> i64 {
        sqlx::query(&format!("SELECT COUNT(*) AS n FROM {table}"))
            .fetch_one(db)
            .await
            .unwrap()
            .get("n")
    }

    async fn latest(db: &SqlitePool) -> Option<f64> {
        series(db)
            .await
            .unwrap()
            .get(&issue())
            .and_then(|s| s.last().map(|p| p.usd))
    }

    async fn session_usd(db: &SqlitePool, companion_id: Uuid, id: &str) -> Option<f64> {
        sqlx::query("SELECT usd FROM spend_sessions WHERE companion_id = ? AND session_id = ?")
            .bind(companion_id.to_string())
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap()
            .map(|r| r.get("usd"))
    }

    #[tokio::test]
    async fn the_same_snapshot_twice_stores_one_row_and_counts_unchanged() {
        let (db, _t) = db().await;
        let (reader, metrics) = metric_reader::start();
        let c = Uuid::new_v4();
        let snap = [session("s1", Some(1.0))];
        record(&db, &metrics, c, &snap, t0()).await.unwrap();
        record(&db, &metrics, c, &snap, t0() + Duration::minutes(1))
            .await
            .unwrap();
        assert_eq!(rows(&db, "spend_sessions").await, 1);
        assert_eq!(rows(&db, "spend_points").await, 1);
        assert_eq!(count(&reader, "stored"), 1);
        assert_eq!(count(&reader, "unchanged"), 1);
    }

    #[tokio::test]
    async fn two_sessions_on_one_issue_sum() {
        let (db, _t) = db().await;
        let (_reader, metrics) = metric_reader::start();
        let snap = [session("s1", Some(1.0)), session("s2", Some(2.5))];
        record(&db, &metrics, Uuid::new_v4(), &snap, t0())
            .await
            .unwrap();
        assert_eq!(latest(&db).await, Some(3.5));
    }

    #[tokio::test]
    async fn a_session_that_leaves_the_snapshot_keeps_its_share() {
        let (db, _t) = db().await;
        let (_reader, metrics) = metric_reader::start();
        let c = Uuid::new_v4();
        let both = [session("s1", Some(1.0)), session("s2", Some(2.0))];
        record(&db, &metrics, c, &both, t0()).await.unwrap();
        // s2 ended and was swept from the companion's registry.
        let one = [session("s1", Some(1.5))];
        record(&db, &metrics, c, &one, t0() + Duration::minutes(20))
            .await
            .unwrap();
        let s = series(&db).await.unwrap()[&issue()].clone();
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[1].usd, 3.5);
    }

    #[tokio::test]
    async fn an_older_snapshot_after_a_newer_one_changes_nothing() {
        let (db, _t) = db().await;
        let (reader, metrics) = metric_reader::start();
        let c = Uuid::new_v4();
        let later = t0() + Duration::minutes(20);
        record(&db, &metrics, c, &[session("s1", Some(4.0))], later)
            .await
            .unwrap();
        record(&db, &metrics, c, &[session("s1", Some(1.0))], t0())
            .await
            .unwrap();
        assert_eq!(session_usd(&db, c, "s1").await, Some(4.0));
        assert_eq!(
            rows(&db, "spend_points").await,
            1,
            "no point for the old bucket"
        );
        assert_eq!(latest(&db).await, Some(4.0));
        assert_eq!(count(&reader, "unchanged"), 1);
    }

    #[tokio::test]
    async fn nan_and_negative_spend_are_rejected() {
        let (db, _t) = db().await;
        let (reader, metrics) = metric_reader::start();
        let snap = [
            session("s1", Some(f64::NAN)),
            session("s2", Some(-1.0)),
            session("s3", Some(f64::INFINITY)),
            session("s4", None),
        ];
        record(&db, &metrics, Uuid::new_v4(), &snap, t0())
            .await
            .unwrap();
        assert_eq!(rows(&db, "spend_sessions").await, 0);
        assert_eq!(rows(&db, "spend_points").await, 0);
        assert_eq!(
            count(&reader, "rejected"),
            3,
            "no spend at all is not rejected"
        );
        assert_eq!(count(&reader, "stored"), 0);
    }

    /// The row key is the pairing's companion id, not a claim in the snapshot: companion B
    /// naming companion A's session id writes its own row.
    #[tokio::test]
    async fn another_companion_cannot_overwrite_a_session_row() {
        let (db, _t) = db().await;
        let (_reader, metrics) = metric_reader::start();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut theirs = session("s1", Some(5.0));
        theirs.companion_id = a;
        record(&db, &metrics, a, &[theirs], t0()).await.unwrap();
        let mut forged = session("s1", Some(0.0));
        forged.companion_id = a; // claims to be A's session
        record(&db, &metrics, b, &[forged], t0() + Duration::minutes(1))
            .await
            .unwrap();
        assert_eq!(
            session_usd(&db, a, "s1").await,
            Some(5.0),
            "A's row untouched"
        );
        assert_eq!(session_usd(&db, b, "s1").await, Some(0.0), "B's own row");
        assert_eq!(rows(&db, "spend_sessions").await, 2);
        assert_eq!(latest(&db).await, Some(5.0));
    }

    #[tokio::test]
    async fn a_session_without_an_issue_is_not_recorded() {
        let (db, _t) = db().await;
        let (reader, metrics) = metric_reader::start();
        let mut loose = session("s1", Some(2.0));
        loose.issue = None;
        record(&db, &metrics, Uuid::new_v4(), &[loose], t0())
            .await
            .unwrap();
        assert_eq!(rows(&db, "spend_sessions").await, 0);
        assert!(series(&db).await.unwrap().is_empty());
        assert!(reader.points(metric::SPEND_POINTS).is_empty());
    }

    /// Two companions' snapshots for one issue, written at once: whichever point statement
    /// runs last sums both sessions (design 103 §2, Concurrency).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_snapshots_for_one_issue_end_at_the_sum_of_both() {
        let (db, _t) = db().await;
        let (_reader, metrics) = metric_reader::start();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        for round in 1..=50 {
            let at = t0() + Duration::seconds(round);
            let sa = [session("sa", Some(round as f64))];
            let sb = [session("sb", Some(round as f64 * 2.0))];
            let (ra, rb) = tokio::join!(
                record(&db, &metrics, a, &sa, at),
                record(&db, &metrics, b, &sb, at)
            );
            ra.unwrap();
            rb.unwrap();
            assert_eq!(latest(&db).await, Some(round as f64 * 3.0), "round {round}");
        }
    }

    #[tokio::test]
    async fn a_series_is_oldest_first_and_downsampled() {
        let (db, _t) = db().await;
        let (_reader, metrics) = metric_reader::start();
        let c = Uuid::new_v4();
        for i in 0..40 {
            let snap = [session("s1", Some(i as f64 + 1.0))];
            record(&db, &metrics, c, &snap, t0() + Duration::minutes(15 * i))
                .await
                .unwrap();
        }
        let s = series(&db).await.unwrap()[&issue()].clone();
        assert_eq!(s.len(), MAX_POINTS);
        assert_eq!(s[0].at, t0());
        assert_eq!(s.last().unwrap().usd, 40.0);
        assert!(s.windows(2).all(|w| w[0].at < w[1].at));
    }
}
