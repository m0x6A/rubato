//! Approvals GitHub did not confirm: found by their comment (author, issue, sha) on the next
//! comments pass, or dropped after `OPENING_LIMIT_SECS` so the person can press again.

use super::store;
use crate::github_poll;
use crate::kernel::state::App;
use chrono::{Duration, Utc};

/// An `opening` row with no comment found after this long is dropped.
pub const OPENING_LIMIT_SECS: i64 = 600;
/// Rows `opening` for longer than this count in `rubato.plans.approvals_opening`.
const STUCK_AFTER_SECS: i64 = 120;

pub async fn reconcile(app: &App) {
    let rows = match store::opening(&app.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading opening approvals failed");
            return;
        }
    };
    let approvals = github_poll::all_approvals(&app.db)
        .await
        .unwrap_or_default();
    let now = Utc::now();
    let mut stuck = 0u64;
    for i in rows {
        let found = approvals.iter().map(|(a, _)| a).find(|a| {
            a.issue == i.issue
                && a.author == i.login
                && a.sha.as_deref().is_some_and(|s| i.sha.starts_with(s))
        });
        if let Some(a) = found {
            match store::posted(&app.db, &i, a.comment_id, &a.url).await {
                Ok(()) => {
                    tracing::info!(rubato.issue = %i.issue, login = %i.login, "approval found in the comments")
                }
                Err(e) => tracing::error!(error = %format!("{e:#}"), "settling an approval failed"),
            }
        } else if now - i.created_at > Duration::seconds(OPENING_LIMIT_SECS) {
            let _ = store::forget(&app.db, &i.login, &i.issue, &i.sha).await;
            tracing::warn!(rubato.issue = %i.issue, login = %i.login, "approval not found after ten minutes; dropped");
        } else if now - i.created_at > Duration::seconds(STUCK_AFTER_SECS) {
            stuck += 1;
        }
    }
    app.metrics.plans_approvals_opening.record(stuck, &[]);
}
