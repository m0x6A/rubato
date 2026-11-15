//! Hook payloads cut down to `PAYLOAD_ALLOWLIST` on the server (#190): on receipt, for a
//! companion from before #166 that still sends the whole hook stdin, and once per start over the
//! `companion_events` rows such companions left. Both use `rubato_protocol::scrub_payload`, the
//! rule the companion applies, and count each removed key as `rubato.events.scrubbed`.

use crate::kernel::telemetry::Metrics;
use anyhow::Result;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{HookEvent, scrub_payload};
use sqlx::{Row, SqlitePool};

/// Scrub a received hook before it is recorded. Counted under `rubato.reason=receipt`.
pub fn on_receipt(metrics: &Metrics, hook: &mut HookEvent) {
    let removed = hook.scrub();
    if removed > 0 {
        metrics.events_scrubbed.add(
            removed as u64,
            &[KeyValue::new(attr::DROP_REASON, "receipt")],
        );
        tracing::info!(rubato.session_id = %hook.session_id, keys_removed = removed, "hook payload scrubbed on receipt");
    }
}

/// What one sweep of `companion_events` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub rows_checked: u64,
    pub rows_rewritten: u64,
    pub keys_removed: u64,
}

/// Rewrite every stored hook envelope whose `payload` holds keys outside the allowlist. Rows
/// already clean are not written, so a second sweep rewrites nothing. Counted under
/// `rubato.reason=stored`; logs one line with what it checked.
pub async fn sweep_stored(db: &SqlitePool, metrics: &Metrics) -> Result<SweepReport> {
    let rows = sqlx::query("SELECT id, payload FROM companion_events WHERE kind = 'hook'")
        .fetch_all(db)
        .await?;
    let mut report = SweepReport::default();
    for r in rows {
        report.rows_checked += 1;
        let id: i64 = r.get("id");
        let text: String = r.get("payload");
        let Ok(serde_json::Value::Object(mut envelope)) = serde_json::from_str(&text) else {
            continue;
        };
        let mut payload = envelope.remove("payload").filter(|v| !v.is_null());
        let removed = scrub_payload(&mut payload);
        if removed == 0 {
            continue;
        }
        if let Some(p) = payload {
            envelope.insert("payload".into(), p);
        }
        let rewritten = serde_json::Value::Object(envelope).to_string();
        sqlx::query("UPDATE companion_events SET payload = ? WHERE id = ?")
            .bind(rewritten)
            .bind(id)
            .execute(db)
            .await?;
        report.rows_rewritten += 1;
        report.keys_removed += removed as u64;
    }
    if report.keys_removed > 0 {
        metrics.events_scrubbed.add(
            report.keys_removed,
            &[KeyValue::new(attr::DROP_REASON, "stored")],
        );
    }
    tracing::info!(
        rows_checked = report.rows_checked,
        rows_rewritten = report.rows_rewritten,
        keys_removed = report.keys_removed,
        "stored hook payloads scrubbed"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::otel::metric;

    const T: &str = "2026-11-15T08:00:00+00:00";

    async fn insert(db: &SqlitePool, seq: i64, kind: &str, payload: &str) {
        sqlx::query("INSERT INTO companion_events (companion_id, seq, kind, payload, received_at) VALUES ('c', ?, ?, ?, ?)")
            .bind(seq).bind(kind).bind(payload).bind(T).execute(db).await.unwrap();
    }

    async fn payload(db: &SqlitePool, seq: i64) -> String {
        sqlx::query("SELECT payload FROM companion_events WHERE seq = ?")
            .bind(seq)
            .fetch_one(db)
            .await
            .unwrap()
            .get("payload")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stored_hook_payloads_are_rewritten_once() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let db = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();

        let dirty = r#"{"type":"hook","session_id":"s1","hook_event_name":"PostToolUse","cwd":"/w","tool_name":"Bash","received_at":"2026-11-15T08:00:00Z","payload":{"session_id":"s1","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"cat .env"},"tool_response":{"stdout":"SECRET"}}}"#;
        let clean = r#"{"type":"hook","session_id":"s2","hook_event_name":"Stop","cwd":"/w","received_at":"2026-11-15T08:00:00Z","payload":{"hook_event_name":"Stop"}}"#;
        let beat = r#"{"type":"heartbeat","at":"2026-11-15T08:00:00Z","payload":{"tool_response":"not a hook"}}"#;
        insert(&db, 1, "hook", dirty).await;
        insert(&db, 2, "hook", clean).await;
        insert(&db, 3, "heartbeat", beat).await;
        assert!(
            payload(&db, 1).await.contains("SECRET"),
            "the fixture holds the secret"
        );

        let first = sweep_stored(&db, &metrics).await.unwrap();
        assert_eq!(
            first,
            SweepReport {
                rows_checked: 2,
                rows_rewritten: 1,
                keys_removed: 3
            }
        );
        let after = payload(&db, 1).await;
        for leaked in ["SECRET", "cat .env", "tool_input", "tool_response"] {
            assert!(!after.contains(leaked), "{leaked} in {after}");
        }
        let v: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["session_id"], "s1", "the lifted fields stay");
        assert_eq!(
            v["payload"],
            serde_json::json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"})
        );
        assert_eq!(
            payload(&db, 2).await,
            clean,
            "a clean hook row is not written"
        );
        assert_eq!(payload(&db, 3).await, beat, "only hook rows are swept");

        let second = sweep_stored(&db, &metrics).await.unwrap();
        assert_eq!(second.rows_checked, 2);
        assert_eq!(second.rows_rewritten, 0, "a second sweep rewrites nothing");
        let points = reader.points(metric::EVENTS_SCRUBBED);
        assert_eq!(points.len(), 1, "{points:?}");
        assert_eq!(points[0].value, 3);
        assert_eq!(points[0].attrs[attr::DROP_REASON], "stored");
        println!(
            "checked {} hook rows, rewrote {}",
            first.rows_checked, first.rows_rewritten
        );
    }
}
