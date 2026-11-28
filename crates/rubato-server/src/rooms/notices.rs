//! Seam notices (ADR 0057, Q6): when this plan and another open plan or settled draft touch the
//! same seam, Rubato posts one system notice in each room, once per pair of issue and seam.
//! Agents never post uninvited. Run on every poll tick by the composition root.
//!
//! Which plans: the same as the overview's `SeamCollision`, grouped by the same `seam_groups`.
//! Each issue's plan on its branch, or, with none, its newest draft whose plan parses; none of an
//! issue whose pull request merged.

use super::store;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use crate::{github_poll, plan_drafts, plans};
use anyhow::Result;
use rubato_protocol::{
    IssueRef, Plan, RoomAuthorKind, RoomKey, RoomMessage, SeamNotice, WebResource, seam_groups,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The author of Rubato's own messages.
pub const SYSTEM_AUTHOR: &str = "rubato";

fn notice_text(other: &IssueRef, other_branch: &str, seam: &str) -> String {
    format!("This plan and {other} on {other_branch} both touch {seam}.")
}

/// Post the notices still missing. Returns how many were posted.
pub async fn post_seam_notices(app: &App, t: &TenantId) -> Result<usize> {
    // Merged, or closed on GitHub (#265): neither plan is open any more. A notice is a message
    // posted once, so a closed issue gets none even while a session still works on it.
    let mut merged: HashSet<IssueRef> = github_poll::all_prs(&app.db, t)
        .await?
        .into_iter()
        .filter(|(_, pr)| pr.merged_at.is_some())
        .map(|(issue, _)| issue)
        .collect();
    merged.extend(
        github_poll::all_issue_meta(&app.db, t)
            .await?
            .into_iter()
            .filter(|(_, m)| m.github_state.as_deref() == Some("closed"))
            .map(|(issue, _)| issue),
    );
    let mut open: HashMap<IssueRef, (String, Plan)> = HashMap::new();
    for row in plans::all(&app.db, t).await? {
        if let Some(plan) = row.plan
            && !merged.contains(&row.issue)
        {
            open.insert(row.issue, (row.branch, plan));
        }
    }
    let mut newest_draft: HashMap<IssueRef, plan_drafts::LatestDraft> = HashMap::new();
    for d in plan_drafts::latest_drafts(app, t).await? {
        if d.plan.is_none() || merged.contains(&d.issue) || open.contains_key(&d.issue) {
            continue;
        }
        if newest_draft
            .get(&d.issue)
            .is_none_or(|n| n.summary.at < d.summary.at)
        {
            newest_draft.insert(d.issue.clone(), d);
        }
    }
    for (issue, d) in newest_draft {
        if let Some(plan) = d.plan {
            open.insert(issue, (d.summary.branch, plan));
        }
    }
    let mut keys: Vec<(&IssueRef, &String, &Plan)> =
        open.iter().map(|(i, (b, p))| (i, b, p)).collect();
    keys.sort_by(|a, b| a.0.cmp(b.0));
    let groups = seam_groups(keys.iter().map(|(i, b, p)| ((*i, *b), *p)));
    let now = app.clock.now();
    let mut posted = 0;
    let mut pairs = 0;
    for (seam, members) in groups {
        for (issue, branch) in &members {
            for (other, other_branch) in members.iter().filter(|(o, _)| o != issue) {
                pairs += 1;
                let room = RoomKey {
                    repo: issue.repo.clone(),
                    number: issue.number,
                    branch: (*branch).clone(),
                };
                let m = RoomMessage {
                    id: Uuid::new_v4(),
                    author_kind: RoomAuthorKind::System,
                    author: SYSTEM_AUTHOR.into(),
                    text: notice_text(other, other_branch, seam),
                    at: now,
                    notice: Some(SeamNotice {
                        seam: seam.to_owned(),
                        other: (*other).clone(),
                        other_branch: (*other_branch).clone(),
                    }),
                    addressed: vec![],
                    turn_id: None,
                    spoken: false,
                };
                if store::insert(&app.db, t, &room, &m).await? {
                    posted += 1;
                    tracing::info!(
                        rubato.issue = %issue,
                        rubato.branch = %branch,
                        rubato.seam = seam,
                        other = %other,
                        other_branch = %other_branch,
                        "seam notice posted"
                    );
                    app.notify_resource(
                        t,
                        WebResource::Room {
                            repo: room.repo,
                            number: room.number,
                            branch: room.branch,
                        },
                    );
                }
            }
        }
    }
    tracing::debug!(plans = open.len(), pairs, posted, "seam notices checked");
    Ok(posted)
}
