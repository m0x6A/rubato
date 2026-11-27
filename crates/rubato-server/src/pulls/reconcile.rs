//! Merges GitHub did not confirm (ADR 0033, plan 82 Decision 12), after each tick's listings:
//! confirmed by a `pull` read once the pull request leaves a complete listing; dropped when it is
//! still listed open at that head on two complete listings, when its head moved, when it closed
//! without a merge, or after [`OPENING_LIMIT_SECS`] whatever the listings say. Called by
//! `tasks.rs`: `pulls` names `github_poll`, so not the other way round.

use super::facts::ViewCache;
use super::store;
use crate::github_poll;
use crate::kernel::state::App;
use chrono::{Duration, Utc};

/// An `opening` row is dropped after this long.
pub const OPENING_LIMIT_SECS: i64 = 600;
/// Rows `opening` for longer than this count in `rubato.pulls.merges_opening`.
const STUCK_AFTER_SECS: i64 = 120;
/// `merged` rows are kept this many days (design 82 A9).
const MERGED_KEPT_DAYS: i64 = 7;
/// Complete listings that still show the pull request open at the head before the row goes.
const LISTINGS_OPEN: u32 = 2;

/// `complete`: the repos listed and stored in full this tick.
pub async fn reconcile(app: &App, complete: &[String]) {
    let rows = match store::opening(&app.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading opening merges failed");
            return;
        }
    };
    let listings = if rows.is_empty() {
        Vec::new()
    } else {
        github_poll::repo_pulls(&app.db).await.unwrap_or_default()
    };
    let cache = app.slots.get::<ViewCache>();
    let now = Utc::now();
    let mut stuck = 0u64;
    for i in rows {
        let gone = |why: &'static str| tracing::warn!(rubato.repo = %i.repo, rubato.pr = i.number, login = %i.login, why, "merge not confirmed");
        let forget = || store::forget(&app.db, &i.repo, i.number, &i.head_sha);
        if now - i.opened_at > Duration::seconds(OPENING_LIMIT_SECS) {
            gone("ten minutes");
            let _ = forget().await;
            continue;
        }
        if complete.iter().any(|r| r.eq_ignore_ascii_case(&i.repo)) {
            let listed = listings
                .iter()
                .filter(|r| r.repo.eq_ignore_ascii_case(&i.repo))
                .flat_map(|r| &r.pulls)
                .find(|p| p.number == i.number);
            match listed {
                Some(p) if p.head_sha == i.head_sha => match store::seen_open(&app.db, &i).await {
                    Ok(n) if n >= LISTINGS_OPEN => {
                        gone("still open");
                        let _ = forget().await;
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(error = %format!("{e:#}"), "counting a merge's listings failed")
                    }
                },
                Some(_) => {
                    gone("head moved");
                    let _ = forget().await;
                    continue;
                }
                None => match app.github.unscoped(&i.repo).pull(&i.repo, i.number).await {
                    Ok(Some(p)) if p.info.merged_at.is_some() => {
                        if let Err(e) = store::merged(&app.db, &i, None).await {
                            tracing::error!(error = %format!("{e:#}"), "settling a merge failed");
                        } else {
                            tracing::info!(rubato.repo = %i.repo, rubato.pr = i.number, login = %i.login, "merge confirmed by poll");
                            cache.evict(&i.repo, i.number);
                            app.notify("merge");
                        }
                        continue;
                    }
                    Ok(Some(p)) if !p.info.open => {
                        gone("closed");
                        let _ = forget().await;
                        continue;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        gone("not found");
                        let _ = forget().await;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(rubato.repo = %i.repo, rubato.pr = i.number, error = %format!("{e:#}"), "merge reconcile failed")
                    }
                },
            }
        }
        if now - i.opened_at > Duration::seconds(STUCK_AFTER_SECS) {
            stuck += 1;
        }
    }
    app.metrics.pulls_merges_opening.record(stuck, &[]);
    if let Err(e) = store::prune_merged(&app.db, now - Duration::days(MERGED_KEPT_DAYS)).await {
        tracing::error!(error = %format!("{e:#}"), "pruning merged rows failed");
    }
}
