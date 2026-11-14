//! Once per poll tick: follow the proposal's pull request (found, merged, closed), empty the
//! draft once a merged proposal is in the loaded map, and record the draft's gauges.
//!
//! The composition root (`tasks.rs`) calls these in order and reloads the map in between when
//! a merge was seen, so this slice does not name the `map` slice.

use super::propose::{count, log_state};
use super::store::{self, ProposalRow, state_name};
use crate::kernel::state::App;
use crate::kernel::workspaces::WorkspaceEntry;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::map_draft::ProposalState;
use rubato_protocol::otel::attr;

const STATES: [ProposalState; 3] = [
    ProposalState::Opening,
    ProposalState::Proposed,
    ProposalState::Merged,
];

/// Both gauges on every tick, zeros included: a gauge recorded only on change keeps its last
/// value, so a past state would linger and a restart would go silent.
pub async fn record_gauges(app: &App) {
    for w in app.config.workspace_entries().0 {
        record_gauges_ws(app, &w).await;
    }
}

async fn record_gauges_ws(app: &App, w: &WorkspaceEntry) {
    match store::live_count(&app.db, &w.id).await {
        Ok(n) => app
            .metrics
            .map_draft_ops
            .record(n, &[KeyValue::new(attr::WORKSPACE, w.id.clone())]),
        Err(e) => tracing::error!(error = %e, "reading the draft size failed"),
    }
    let now = match store::proposal(&app.db, &w.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "reading the proposal failed");
            return;
        }
    };
    for s in STATES {
        let since = now
            .as_ref()
            .filter(|p| p.state == s)
            .and_then(|p| u64::try_from(p.state_since.timestamp()).ok())
            .unwrap_or(0);
        app.metrics.map_proposal_state_since.record(
            since,
            &[
                KeyValue::new(attr::OUTCOME, state_name(s)),
                KeyValue::new(attr::WORKSPACE, w.id.clone()),
            ],
        );
    }
}

/// Empty the draft when its merged proposal is in the loaded map. Only a map load that follows
/// the merge shows the merged map: then the draft's operations are in it, and they and the
/// proposal go. Until such a load the draft stays as it is.
pub async fn clear_merged(app: &App) {
    for w in app.config.workspace_entries().0 {
        clear_merged_ws(app, &w).await;
    }
}

async fn clear_merged_ws(app: &App, w: &WorkspaceEntry) {
    let row = match store::proposal(&app.db, &w.id).await {
        Ok(Some(row)) if row.state == ProposalState::Merged => row,
        Ok(_) => return,
        Err(e) => {
            tracing::error!(error = %e, "reading the proposal failed");
            return;
        }
    };
    let loaded_at: Option<DateTime<Utc>> = app.map_of(&w.id).await.as_ref().map(|m| m.loaded_at);
    if !loaded_at.is_some_and(|at| at > row.state_since) {
        return;
    }
    if let Err(e) = store::clear_merged(&app.db, &w.id, row.last_seq).await {
        tracing::error!(error = %e, "clearing the merged draft failed");
        return;
    }
    log_state("merged", "empty", &row, None);
    if let Err(e) = app.github.delete_ref(&w.map_repo, &row.branch).await {
        tracing::warn!(branch = %row.branch, error = %format!("{e:#}"), "the merged proposal's branch was not deleted");
    }
    app.notify_ws("draft", &w.id);
}

async fn follow(app: &App, w: &WorkspaceEntry, row: &ProposalRow) -> anyhow::Result<()> {
    let repo = &w.map_repo;
    let now = Utc::now();
    if row.state == ProposalState::Merged {
        return Ok(());
    }
    let Some(pr) = app.github.pull_for_branch(repo, &row.branch).await? else {
        return Ok(());
    };
    let from = state_name(row.state);
    let known = Some((pr.number, pr.url.as_str()));
    if pr.merged_at.is_some() {
        store::set_state(&app.db, &w.id, ProposalState::Merged, now, known).await?;
        log_state(from, "merged", row, Some(pr.number));
        count(app, "merged");
        app.notify_ws("draft", &w.id);
    } else if !pr.open {
        // Closed without merging: the draft is editable again, every operation kept.
        store::drop_proposal(&app.db, &w.id).await?;
        log_state(from, "drafting", row, Some(pr.number));
        count(app, "closed");
        if let Err(e) = app.github.delete_ref(repo, &row.branch).await {
            tracing::warn!(branch = %row.branch, error = %format!("{e:#}"), "the closed proposal's branch was not deleted");
        }
        app.notify_ws("draft", &w.id);
    } else if row.state == ProposalState::Opening {
        // The pull request exists though the row never learned it: a call that timed out.
        store::set_state(&app.db, &w.id, ProposalState::Proposed, now, known).await?;
        log_state(from, "proposed", row, Some(pr.number));
        count(app, "opened");
        app.notify_ws("draft", &w.id);
    }
    Ok(())
}

/// Called from the poll tick, after the per-issue reads and before the rate-limit read. True
/// when the proposal is merged and the draft waits for a map load that follows the merge: the
/// caller reloads the map and then calls `clear_merged`.
/// Over every workspace: true when any proposal is merged and waits for a map load.
pub async fn check_proposal(app: &App) -> bool {
    let mut merged = false;
    for w in app.config.workspace_entries().0 {
        merged |= check_proposal_ws(app, &w).await;
    }
    merged
}

async fn check_proposal_ws(app: &App, w: &WorkspaceEntry) -> bool {
    match store::proposal(&app.db, &w.id).await {
        Ok(Some(row)) => {
            if let Err(e) = follow(app, w, &row).await {
                tracing::warn!(error = %format!("{e:#}"), "following the map proposal failed");
            }
        }
        Ok(None) => return false,
        Err(e) => {
            tracing::error!(error = %e, "reading the proposal failed");
            return false;
        }
    }
    matches!(
        store::proposal(&app.db, &w.id).await,
        Ok(Some(row)) if row.state == ProposalState::Merged
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::otel::metric;
    use std::sync::Arc;

    /// The dashboard's "Map proposal age" is `time() - state_since > 0` by state, yellow past
    /// 900 s for opening and merged. So a stuck opening must show its time, and must stop
    /// showing it once the proposal has moved on.
    #[tokio::test]
    async fn state_since_shows_the_current_state_and_zero_for_the_others() {
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
        let since = |state: &str| {
            reader
                .points(metric::MAP_PROPOSAL_STATE_SINCE)
                .into_iter()
                .find(|p| p.attrs.get(attr::OUTCOME).map(String::as_str) == Some(state))
                .map(|p| (p.value, p.unit))
        };

        // No proposal: every state is recorded as 0, and the draft is empty.
        record_gauges(&app).await;
        for s in ["opening", "proposed", "merged"] {
            assert_eq!(since(s), Some((0, String::new())), "{s}: 0 and no unit");
        }
        assert_eq!(reader.points(metric::MAP_DRAFT_OPS)[0].value, 0);

        // Stuck in opening since 20 minutes ago: its time shows, so the panel turns yellow.
        let started = Utc::now() - chrono::Duration::minutes(20);
        store::start_proposal(
            &app.db,
            rubato_protocol::DEFAULT_WORKSPACE,
            "map/draft-1",
            "base",
            "hash",
            "m0x6A",
            started,
        )
        .await
        .unwrap()
        .unwrap();
        record_gauges(&app).await;
        let opening = since("opening").unwrap().0;
        assert_eq!(opening, u64::try_from(started.timestamp()).unwrap());
        assert!(u64::try_from(Utc::now().timestamp()).unwrap() - opening > 900);
        assert_eq!(since("proposed").unwrap().0, 0);

        // Proposed: opening goes quiet at once, and proposed may be as old as a review takes.
        store::set_state(
            &app.db,
            rubato_protocol::DEFAULT_WORKSPACE,
            ProposalState::Proposed,
            Utc::now(),
            Some((7, "u")),
        )
        .await
        .unwrap();
        record_gauges(&app).await;
        assert_eq!(since("opening").unwrap().0, 0);
        assert!(since("proposed").unwrap().0 > 0);
        assert_eq!(since("merged").unwrap().0, 0);

        // Gone: all three back to 0.
        store::drop_proposal(&app.db, rubato_protocol::DEFAULT_WORKSPACE)
            .await
            .unwrap();
        record_gauges(&app).await;
        assert!(
            ["opening", "proposed", "merged"]
                .iter()
                .all(|s| since(s).unwrap().0 == 0)
        );
        println!("checked 3 states over 4 ticks");
    }
}
