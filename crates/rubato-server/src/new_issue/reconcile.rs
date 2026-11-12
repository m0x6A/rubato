//! Every poll tick, after the open-issues listing: finish the creations GitHub did not confirm.
//! An issue whose body carries a `Rubato-Request:` line of an `opening` row is that row's issue;
//! a row unseen for [`OPENING_LIMIT`] in a repo listed whole is `failed`. The gauge
//! `rubato.issues.opening` is recorded every tick, zeros included.

use super::handlers::count;
use super::store;
use crate::kernel::state::App;
use crate::pins;
use chrono::{Duration, Utc};
use rubato_protocol::IssueRef;
use std::collections::{HashMap, HashSet};

/// How long a create may stay unconfirmed before it is given up.
pub const OPENING_LIMIT: Duration = Duration::minutes(10);

/// An open issue whose body names a request id.
#[derive(Debug, Clone)]
pub struct Found {
    pub issue: IssueRef,
    pub url: String,
    pub request_id: String,
}

/// `fresh`: the repos this tick listed whole. Only there does "not seen" mean "not created".
pub async fn reconcile(app: &App, found: &[Found], fresh: &HashSet<String>) {
    let rows = match store::opening(&app.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "reading the opening issue creations failed");
            return;
        }
    };
    let by_id: HashMap<&str, &Found> = found.iter().map(|f| (f.request_id.as_str(), f)).collect();
    let mut changed = false;
    let mut still = 0u64;
    for c in &rows {
        let done = if let Some(f) = by_id
            .get(c.request_id.as_str())
            .filter(|f| f.issue.repo == c.repo)
        {
            finish(app, c, f).await
        } else if fresh.contains(&c.repo) && Utc::now() - c.created_at > OPENING_LIMIT {
            give_up(app, c).await
        } else {
            false
        };
        if done {
            changed = true;
        } else {
            still += 1;
        }
    }
    app.metrics.issues_opening.record(still, &[]);
    if changed {
        app.notify("issue");
    }
}

async fn finish(app: &App, c: &store::Creation, f: &Found) -> bool {
    match store::created(&app.db, &c.request_id, f.issue.number, &f.url).await {
        Ok(true) => {}
        Ok(false) => return true,
        Err(e) => {
            tracing::error!(request_id = %c.request_id, error = %e, "storing a found issue failed");
            return false;
        }
    }
    if let Err(e) = pins::pin_created(app, &f.issue, &c.target, &c.created_by).await {
        tracing::error!(rubato.issue = %f.issue, error = %e, "pinning a found issue failed");
    }
    count(app, "found");
    tracing::info!(
        rubato.issue = %f.issue,
        request_id = %c.request_id,
        "issue found by its Rubato-Request line"
    );
    true
}

async fn give_up(app: &App, c: &store::Creation) -> bool {
    let why = "GitHub did not confirm it and no listing showed it for ten minutes";
    match store::failed(&app.db, &c.request_id, why).await {
        Ok(changed) => {
            if changed {
                count(app, "failed");
                tracing::warn!(
                    step = "unseen",
                    rubato.repo = %c.repo,
                    request_id = %c.request_id,
                    "issue creation failed"
                );
            }
            true
        }
        Err(e) => {
            tracing::error!(request_id = %c.request_id, error = %e, "failing an issue creation failed");
            false
        }
    }
}
