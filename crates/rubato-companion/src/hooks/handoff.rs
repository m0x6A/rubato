//! The handoff arm (ADR 0055): a report `plan.ts handoff` posted to `/handoff` goes to the
//! server as `CompanionMessage::HandoffReport`, through the outbox, so it survives a reconnect.
//! Only for a launch this companion knows, and only to a server that announced
//! `handoff_reports`; anything else is counted on `rubato.events.dropped` and logged at warn.

use crate::kernel::telemetry::Metrics;
use crate::sessions::Registry;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{CompanionMessage, HandoffReport, capability};
use tokio::sync::{Mutex, mpsc};

/// What happened to one report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forwarded {
    Sent,
    /// No launch with that id here: the header named someone else's, or a stale one.
    UnknownLaunch,
    /// The server's Welcome lacked `handoff_reports`: an older server would drop it.
    NotAnnounced,
}

pub async fn on_handoff(
    report: HandoffReport,
    registry: &Mutex<Registry>,
    out: &mpsc::UnboundedSender<CompanionMessage>,
    metrics: &Metrics,
    server_capabilities: &std::sync::Mutex<Vec<String>>,
) -> Forwarded {
    let line = &report.line;
    let issue = registry
        .lock()
        .await
        .by_launch(report.launch_id)
        .map(|e| e.issue.clone());
    let Some(issue) = issue else {
        metrics.events_dropped.add(
            1,
            &[KeyValue::new(attr::DROP_REASON, "handoff_unknown_launch")],
        );
        tracing::warn!(launch_id = %report.launch_id, rubato.approval_id = %line.approval, "handoff report for a launch not started here");
        return Forwarded::UnknownLaunch;
    };
    let announced = server_capabilities
        .lock()
        .unwrap()
        .iter()
        .any(|c| c == capability::HANDOFF_REPORTS);
    if !announced {
        metrics.events_dropped.add(
            1,
            &[KeyValue::new(attr::DROP_REASON, "handoff_not_announced")],
        );
        tracing::warn!(launch_id = %report.launch_id, rubato.approval_id = %line.approval, "handoff report not forwarded: the server did not announce handoff_reports");
        return Forwarded::NotAnnounced;
    }
    metrics
        .handoff_reports
        .add(1, &[KeyValue::new(attr::OUTCOME, line.outcome.name())]);
    tracing::info!(launch_id = %report.launch_id, rubato.issue = ?issue, rubato.approval_id = %line.approval, outcome = line.outcome.name(), commit = ?line.commit, "handoff report forwarded");
    let _ = out.send(CompanionMessage::HandoffReport(report));
    Forwarded::Sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::metric_reader;
    use crate::sessions::LaunchRecord;
    use chrono::Utc;
    use rubato_protocol::otel::metric;
    use rubato_protocol::{HandoffLine, HandoffOutcome, IssueRef};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn report(launch: u128) -> HandoffReport {
        HandoffReport {
            launch_id: Uuid::from_u128(launch),
            line: HandoffLine {
                event: "rubato.plan.handoff".into(),
                outcome: HandoffOutcome::Committed,
                approval: Uuid::from_u128(5),
                commit: Some("abc1234".into()),
                adrs_accepted: vec!["0050".into()],
                adrs_left: vec![],
                error: None,
            },
            at: Utc::now(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_report_for_a_launch_here_goes_to_a_server_that_takes_it() {
        let mut reg = Registry::new(Uuid::nil(), "m0x6A");
        reg.restore_launches(
            vec![LaunchRecord {
                launch_id: Uuid::from_u128(7),
                session_id: "s7".into(),
                worktree: PathBuf::from("/w"),
                pane_id: None,
                tmux_session: None,
                issue: IssueRef::new("acme/pantry", 7),
                branch: None,
                spend_cap_usd: None,
                plan_path: None,
            }],
            None,
            false,
            Utc::now(),
        );
        let reg = Mutex::new(reg);
        let (out, mut rx) = mpsc::unbounded_channel();
        let (reader, metrics) = metric_reader::start();
        let caps = std::sync::Mutex::new(vec![capability::HANDOFF_REPORTS.to_owned()]);

        assert_eq!(
            on_handoff(report(7), &reg, &out, &metrics, &caps).await,
            Forwarded::Sent
        );
        match rx.try_recv() {
            Ok(CompanionMessage::HandoffReport(r)) => assert_eq!(r, report_like(&r)),
            other => panic!("expected a handoff report, got {other:?}"),
        }
        let points = reader.points(metric::HANDOFF_REPORTS);
        assert_eq!(points[0].attrs[attr::OUTCOME], "committed");

        assert_eq!(
            on_handoff(report(99), &reg, &out, &metrics, &caps).await,
            Forwarded::UnknownLaunch
        );
        let older = std::sync::Mutex::new(vec![capability::SESSION_MESSAGES.to_owned()]);
        assert_eq!(
            on_handoff(report(7), &reg, &out, &metrics, &older).await,
            Forwarded::NotAnnounced
        );
        assert!(rx.try_recv().is_err(), "nothing else was sent");
        let dropped = reader.points(metric::EVENTS_DROPPED);
        let reasons: Vec<&str> = dropped
            .iter()
            .map(|p| p.attrs[attr::DROP_REASON].as_str())
            .collect();
        assert!(reasons.contains(&"handoff_unknown_launch"), "{reasons:?}");
        assert!(reasons.contains(&"handoff_not_announced"), "{reasons:?}");
    }

    /// The report as sent: the launch from the header, the line as the skill printed it.
    fn report_like(r: &HandoffReport) -> HandoffReport {
        HandoffReport {
            at: r.at,
            ..report(7)
        }
    }
}
