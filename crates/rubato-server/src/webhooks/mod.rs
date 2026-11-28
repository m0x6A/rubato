//! Slice `webhooks`: GitHub App webhook deliveries (ADR 0074). `POST /webhooks/github` verifies
//! each delivery's signature, reduces it to what it names and stores it once; the composition
//! root's poll loop reads the pending rows, refreshes what they name through `github_poll` and
//! `plans`, and settles them here. Names no other slice. Owns table `webhook_deliveries`.

mod event;
mod handler;
mod store;
mod verify;

#[cfg(test)]
mod tests;

pub use event::Target;
pub use handler::{MAX_BODY_BYTES, PATH, receive};
pub use verify::sign;

use crate::kernel::state::App;

/// Pending rows read per pass (design 208 D4).
pub const PASS_MAX_ROWS: i64 = 200;

/// One pending delivery and what it asks the poll loop to read.
#[derive(Debug, Clone)]
pub struct Pending {
    pub delivery_id: String,
    pub target: Target,
}

/// The pending deliveries, oldest first, at most [`PASS_MAX_ROWS`]. A pass marks only these
/// (D9), so a delivery stored while it runs waits for the next one. Empty on a store error,
/// which is logged.
pub async fn pending(app: &App) -> Vec<Pending> {
    match store::pending(&app.db, PASS_MAX_ROWS).await {
        Ok(rows) => rows
            .into_iter()
            .filter(|r| !event::is_installation_event(&r.event))
            .filter_map(|r| {
                let target = event::target(
                    &r.event,
                    r.repo.as_deref(),
                    r.issue_number,
                    r.branch.as_deref(),
                )?;
                Some(Pending {
                    delivery_id: r.delivery_id,
                    target,
                })
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading pending webhook deliveries failed");
            Vec::new()
        }
    }
}

/// What an accepted `installation` or `installation_repositories` delivery says (design §5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Suspended,
    Unsuspended,
    Deleted,
    Repositories,
}

/// One pending delivery about an installation itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationChange {
    pub delivery_id: String,
    pub installation: u64,
    pub change: Change,
}

/// The pending deliveries about installations, oldest first. The poll loop applies each to the
/// installation's tenant and settles it with [`settle`]. A row that names no installation, or an
/// action this server does not act on, is settled here.
pub async fn installation_changes(app: &App) -> Vec<InstallationChange> {
    let rows = match store::pending(&app.db, PASS_MAX_ROWS).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading pending webhook deliveries failed");
            return Vec::new();
        }
    };
    let (mut out, mut stray) = (Vec::new(), Vec::new());
    for r in rows
        .into_iter()
        .filter(|r| event::is_installation_event(&r.event))
    {
        let change = match (r.event.as_str(), r.action.as_deref()) {
            ("installation", Some("suspend")) => Some(Change::Suspended),
            ("installation", Some("unsuspend")) => Some(Change::Unsuspended),
            ("installation", Some("deleted")) => Some(Change::Deleted),
            ("installation_repositories", _) => Some(Change::Repositories),
            _ => None,
        };
        match (r.installation, change) {
            (Some(installation), Some(change)) => out.push(InstallationChange {
                delivery_id: r.delivery_id,
                installation,
                change,
            }),
            _ => stray.push(r.delivery_id),
        }
    }
    if !stray.is_empty()
        && let Err(e) = store::mark(&app.db, &stray, app.clock.now().timestamp()).await
    {
        tracing::error!(error = %format!("{e:#}"), "marking webhook deliveries failed");
    }
    out
}

/// Settle a pass (D9): `done` rows are marked; each `failed` row gets one more attempt, and at
/// the third it is marked and left to the catch-up. One `info` line per pass.
pub async fn settle(app: &App, done: &[String], failed: &[(String, String)], issues: usize) {
    let now = app.clock.now().timestamp();
    if let Err(e) = store::mark(&app.db, done, now).await {
        tracing::error!(error = %format!("{e:#}"), "marking webhook deliveries failed");
    }
    for (id, issue) in failed {
        match store::failed(&app.db, id, now).await {
            Ok(true) => tracing::warn!(
                delivery = %id,
                rubato.issue = %issue,
                attempts = store::MAX_ATTEMPTS,
                "webhook refresh gave up; the catch-up reads it"
            ),
            Ok(false) => {}
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "counting a failed webhook refresh failed")
            }
        }
    }
    tracing::info!(
        deliveries = done.len() + failed.len(),
        issues,
        failed = failed.len(),
        marked = done.len(),
        "webhook refresh"
    );
}

/// On every poll tick while webhooks are on: rows older than 7 days go, and the two webhook
/// gauges are recorded (D12). Absent while webhooks are off, so their alerts stay silent.
pub async fn record(app: &App) {
    if !app.config.webhooks_on() {
        return;
    }
    let now = app.clock.now().timestamp();
    match store::prune(&app.db, now - store::KEEP_SECS).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(pruned = n, "webhook deliveries pruned"),
        Err(e) => tracing::error!(error = %format!("{e:#}"), "pruning webhook deliveries failed"),
    }
    let last = store::last_accepted_at(&app.db).await.unwrap_or_else(|e| {
        tracing::error!(error = %format!("{e:#}"), "reading the last webhook delivery failed");
        None
    });
    app.metrics
        .webhook_last_accepted_at
        .record(last.and_then(|t| u64::try_from(t).ok()).unwrap_or(0), &[]);
    let oldest = store::oldest_pending_at(&app.db).await.unwrap_or_else(|e| {
        tracing::error!(error = %format!("{e:#}"), "reading pending webhook deliveries failed");
        None
    });
    let age = oldest.map_or(0, |t| u64::try_from(now - t).unwrap_or(0));
    app.metrics.webhook_pending_oldest.record(age, &[]);
}
