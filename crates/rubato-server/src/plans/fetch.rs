//! Fetch the plan file on an issue branch (ADR 0005, 0009) and store what it says. A parse
//! failure is recorded, counted and does not stop the caller; a missing file deletes the plan.

use super::hash::approval_hash;
use super::parse::parse_plan;
use super::store;
use crate::kernel::state::{App, LoadedMap};
use anyhow::Result;
use opentelemetry::KeyValue;
use rubato_protocol::IssueRef;
use rubato_protocol::otel::attr;

/// Slug from the branch `issue/<n>-<slug>`.
pub fn slug_of(branch: &str, number: u64) -> String {
    branch
        .strip_prefix(&format!("issue/{number}-"))
        .unwrap_or("")
        .to_owned()
}

/// Where the plan of `issue` lives on `branch`, by the map's rule.
pub fn plan_path(map: &LoadedMap, issue: &IssueRef, branch: &str) -> String {
    map.map
        .plan_path_for(&issue.repo, issue.number, &slug_of(branch, issue.number))
}

pub async fn fetch_and_store(
    app: &App,
    map: &LoadedMap,
    issue: &IssueRef,
    branch: &str,
) -> Result<()> {
    let path = plan_path(map, issue, branch);
    match app.github.file(&issue.repo, &path, branch).await? {
        Some(text) => match parse_plan(&text, &path) {
            Ok(mut plan) => {
                plan.git_ref = Some(branch.to_owned());
                let sha = approval_hash(&text);
                store::upsert_plan(&app.db, issue, branch, &path, Some(&plan), None, &sha).await?;
            }
            Err(e) => {
                app.metrics.plan_parse_failures.add(
                    1,
                    &[
                        KeyValue::new(attr::PLAN_PATH, path.clone()),
                        KeyValue::new(attr::REPO, issue.repo.clone()),
                    ],
                );
                let sha = approval_hash(&text);
                store::upsert_plan(
                    &app.db,
                    issue,
                    branch,
                    &path,
                    None,
                    Some(&e.to_string()),
                    &sha,
                )
                .await?;
            }
        },
        None => store::delete_plan(&app.db, issue).await?,
    }
    Ok(())
}

/// The approval hash of the plan on `branch`, which is not the branch the plan is read from:
/// one file read, for an approval that names that branch (ADR 0027). `None` when that branch
/// has no plan file. Nothing is stored here.
pub async fn hash_on_branch(
    app: &App,
    map: &LoadedMap,
    issue: &IssueRef,
    branch: &str,
) -> Result<Option<String>> {
    let path = plan_path(map, issue, branch);
    Ok(app
        .github
        .file(&issue.repo, &path, branch)
        .await?
        .map(|text| approval_hash(&text)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn slug_comes_from_the_branch() {
        assert_eq!(
            super::slug_of("issue/20-vertical-slices", 20),
            "vertical-slices"
        );
        assert_eq!(super::slug_of("main", 20), "");
    }
}
