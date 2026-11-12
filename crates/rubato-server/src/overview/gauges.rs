//! Gauges from the derived picture: issues by state, open plans per seam, unknown plan ids,
//! approvals of an earlier plan version.

use super::build;
use super::inputs::build_overview_inputs;
use crate::kernel::state::{App, LoadedMap};
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;

pub async fn refresh_gauges(app: &App, map: &LoadedMap) -> Result<()> {
    let built = build_overview_inputs(app).await?;
    let built = build::build(&built, Some(map), &app.config.limits, Utc::now());
    let (by_state, by_seam) = build::counts(&built);
    for (state, n) in by_state {
        app.metrics.issues_by_state.record(
            n as u64,
            &[KeyValue::new(
                attr::ISSUE_STATE,
                format!("{state:?}").to_lowercase(),
            )],
        );
    }
    for (seam, n) in by_seam {
        app.metrics
            .seam_open_plans
            .record(n as u64, &[KeyValue::new(attr::SEAM, seam)]);
    }
    let unknown: usize = built
        .findings
        .iter()
        .map(|f| {
            if let rubato_protocol::Finding::UnknownIds { ids, .. } = f {
                ids.len()
            } else {
                0
            }
        })
        .sum();
    app.metrics
        .plan_unknown_id_count
        .record(unknown as u64, &[]);
    let stale = built
        .findings
        .iter()
        .filter(|f| matches!(f, rubato_protocol::Finding::ApprovalStale { .. }))
        .count();
    app.metrics.plans_approval_stale.record(stale as u64, &[]);
    Ok(())
}
