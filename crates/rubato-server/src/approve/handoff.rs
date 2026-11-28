//! A handoff report from the companion (ADR 0055): `plan.ts handoff`'s outcome, forwarded from
//! the session's loopback endpoint. Recorded on the approval it names, but only for a launch this
//! companion started and only when that launch's issue is the approval's.

use super::records::{self, HandoffFacts};
use crate::kernel::state::App;
use crate::launch;
use opentelemetry::KeyValue;
use rubato_protocol::HandoffReport;
use rubato_protocol::otel::attr;
use uuid::Uuid;

/// Why a report was not recorded, on `rubato.events.dropped`.
pub mod refused {
    pub const FOREIGN_LAUNCH: &str = "handoff_foreign_launch";
    pub const UNKNOWN_APPROVAL: &str = "handoff_unknown_approval";
    pub const OTHER_ISSUE: &str = "handoff_other_issue";
}

fn dropped(app: &App, why: &'static str) {
    app.metrics
        .events_dropped
        .add(1, &[KeyValue::new(attr::DROP_REASON, why)]);
}

/// Record the report. Returns whether it was recorded.
pub async fn on_handoff_report(app: &App, companion_id: Uuid, r: &HandoffReport) -> bool {
    let approval_id = r.line.approval;
    let target = match launch::acting_target(app, r.launch_id).await {
        Ok(Some(t)) if t.companion_id == companion_id => t,
        Ok(_) => {
            dropped(app, refused::FOREIGN_LAUNCH);
            tracing::warn!(rubato.approval_id = %approval_id, rubato.launch_id = %r.launch_id, "handoff report for a launch this companion did not start");
            return false;
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "handoff: reading the launch failed");
            return false;
        }
    };
    // The approval is the launch's repository's tenant's (ADR 0087).
    let t = match app.repo_tenant(&target.issue.repo) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(rubato.issue = %target.issue, error = %format!("{e:#}"), "handoff: the issue's repository is in no tenant");
            return false;
        }
    };
    let t = &t;
    let approval = match records::get(&app.db, t, approval_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            dropped(app, refused::UNKNOWN_APPROVAL);
            tracing::warn!(rubato.issue = %target.issue, rubato.approval_id = %approval_id, "handoff report for an approval Rubato does not have");
            return false;
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "handoff: reading the approval failed");
            return false;
        }
    };
    if approval.issue != target.issue {
        dropped(app, refused::OTHER_ISSUE);
        tracing::warn!(rubato.issue = %target.issue, rubato.approval_id = %approval_id, approval_issue = %approval.issue, "handoff report names an approval of another issue");
        return false;
    }
    let facts = HandoffFacts {
        outcome: r.line.outcome.name().to_owned(),
        at: r.at,
        commit: r.line.commit.clone(),
        adrs_left: r.line.adrs_left.clone(),
    };
    match records::record_handoff(&app.db, t, approval_id, &target.issue, &facts).await {
        Ok(true) => {
            tracing::info!(
                rubato.issue = %target.issue,
                rubato.approval_id = %approval_id,
                rubato.launch_id = %r.launch_id,
                rubato.outcome = r.line.outcome.name(),
                commit = ?r.line.commit,
                adrs_accepted = ?r.line.adrs_accepted,
                adrs_left = ?r.line.adrs_left,
                error = ?r.line.error,
                "handoff reported"
            );
            app.notify_in(t, "handoff");
            true
        }
        Ok(false) => false,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "handoff: recording the report failed");
            false
        }
    }
}
