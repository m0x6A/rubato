//! Which workspaces this server holds (ADR 0044): read from `RUBATO_WORKSPACES_FILE`, or made
//! from the legacy `RUBATO_MAP_*` variables as the one workspace `default`. Every refusal is
//! decided here, at start, with the reason.

use anyhow::{Result, bail};
use rubato_protocol::{DEFAULT_WORKSPACE, is_workspace_id};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One workspace: one map, read from GitHub or, for local runs and tests, from a file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEntry {
    pub id: String,
    /// What people call it; else the map's own `name`.
    #[serde(default)]
    pub name: Option<String>,
    pub map_repo: String,
    pub map_path: String,
    #[serde(default = "main")]
    pub map_ref: String,
    #[serde(default)]
    pub map_file: Option<PathBuf>,
}

fn main() -> String {
    "main".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspacesFile {
    workspaces: Vec<WorkspaceEntry>,
}

/// Where the workspaces came from, for the start-up log `workspace config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    File,
    LegacyEnv,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::File => "file",
            Source::LegacyEnv => "legacy_env",
        }
    }
}

/// Parse and check a workspaces file. `file` is its path: a relative `map_file` is relative to
/// it. `legacy_set` names the `RUBATO_MAP_*` variables that are set, which must be none.
pub fn from_file(text: &str, file: &Path, legacy_set: &[&str]) -> Result<Vec<WorkspaceEntry>> {
    if !legacy_set.is_empty() {
        bail!(
            "RUBATO_WORKSPACES_FILE and {} are both set; put every map in the workspaces file",
            legacy_set.join(", ")
        );
    }
    let parsed: WorkspacesFile = serde_yaml_ng::from_str(text)
        .map_err(|e| anyhow::anyhow!("workspaces file {}: {e}", file.display()))?;
    let mut out = parsed.workspaces;
    let base = file.parent().unwrap_or(Path::new("."));
    for w in &mut out {
        if let Some(f) = &w.map_file
            && f.is_relative()
        {
            w.map_file = Some(base.join(f));
        }
    }
    check(&out)?;
    Ok(out)
}

/// The refusals of ADR 0044, in the order a person fixes them.
pub fn check(entries: &[WorkspaceEntry]) -> Result<()> {
    if entries.is_empty() {
        bail!("the workspaces file lists no workspace");
    }
    for w in entries {
        if !is_workspace_id(&w.id) {
            bail!("workspace id {:?} is not [a-z0-9-]{{1,32}}", w.id);
        }
    }
    for (i, w) in entries.iter().enumerate() {
        if entries[..i].iter().any(|o| o.id == w.id) {
            bail!("workspace id {} is listed twice", w.id);
        }
        if let Some(o) = entries[..i].iter().find(|o| {
            o.map_repo.eq_ignore_ascii_case(&w.map_repo)
                && o.map_path == w.map_path
                && o.map_ref == w.map_ref
        }) {
            bail!(
                "workspaces {} and {} name the same map ({} {} at {})",
                o.id,
                w.id,
                w.map_repo,
                w.map_path,
                w.map_ref
            );
        }
    }
    if !entries.iter().any(|w| w.id == DEFAULT_WORKSPACE) {
        bail!(
            "the workspaces file has no entry with id {DEFAULT_WORKSPACE}; it keeps every row from before workspaces and serves unprefixed routes"
        );
    }
    Ok(())
}

/// Tables that hold a workspace id (ADR 0047), for the orphan count.
pub const SCOPED_TABLES: [&str; 9] = [
    "map_layout",
    "map_cache",
    "map_proposal",
    "pins",
    "map_line_pins",
    "map_draft_ops",
    "issue_creations",
    "launches",
    "issue_suggestions",
];

/// At start: log which workspaces this server holds (`workspace config`), record how many, and
/// count rows whose workspace is no longer configured. Those rows are kept, never deleted, and
/// logged so a removed or renamed entry is noticed.
pub async fn report(app: &crate::kernel::state::App) -> Result<u64> {
    let (entries, source) = app.config.workspace_entries();
    let ids: Vec<&str> = entries.iter().map(|w| w.id.as_str()).collect();
    tracing::info!(workspaces = ?ids, source = source.as_str(), "workspace config");
    app.metrics
        .workspaces_configured
        .record(entries.len() as u64, &[]);
    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut orphans = 0u64;
    for t in SCOPED_TABLES {
        let sql = format!(
            "SELECT workspace_id, COUNT(*) AS n FROM {t} WHERE workspace_id NOT IN ({placeholders}) GROUP BY workspace_id"
        );
        let mut q = sqlx::query(&sql);
        for id in &ids {
            q = q.bind(*id);
        }
        for r in q.fetch_all(&app.db).await? {
            use sqlx::Row;
            let n = r.get::<i64, _>("n") as u64;
            let ws: String = r.get("workspace_id");
            tracing::warn!(table = t, rubato.workspace = %ws, rows = n, "orphan workspace rows");
            orphans += n;
        }
    }
    app.metrics.workspace_orphan_rows.record(orphans, &[]);
    Ok(orphans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(body: &str) -> Result<Vec<WorkspaceEntry>> {
        from_file(body, Path::new("/etc/rubato/workspaces.yml"), &[])
    }

    #[test]
    fn a_file_with_default_reads_and_resolves_map_file_against_itself() {
        let w = file("workspaces:\n  - id: default\n    map_repo: m0x6A/rubato\n    map_path: docs/design/rubato-map.yml\n    map_file: maps/rubato.yml\n").unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].map_ref, "main");
        assert_eq!(
            w[0].map_file.as_deref(),
            Some(Path::new("/etc/rubato/maps/rubato.yml"))
        );
    }

    #[test]
    fn several_workspaces_with_default_read() {
        let w = file("workspaces:\n  - id: default\n    map_repo: a/b\n    map_path: m.yml\n  - id: shop\n    map_repo: a/c\n    map_path: m.yml\n").unwrap();
        assert_eq!(
            w.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["default", "shop"]
        );
    }

    #[test]
    fn every_refusal_says_why() {
        let entry = |id: &str, repo: &str, path: &str| {
            format!("  - id: {id}\n    map_repo: {repo}\n    map_path: {path}\n")
        };
        let cases: Vec<(String, &str)> = vec![
            ("workspaces: []\n".into(), "lists no workspace"),
            (format!("workspaces:\n{}", entry("Shop", "a/b", "m.yml")), "is not [a-z0-9-]"),
            (format!("workspaces:\n{}{}", entry("default", "a/b", "m.yml"), entry("default", "a/c", "m.yml")), "listed twice"),
            (format!("workspaces:\n{}{}", entry("default", "a/b", "m.yml"), entry("shop", "A/B", "m.yml")), "name the same map"),
            (format!("workspaces:\n{}", entry("shop", "a/b", "m.yml")), "no entry with id default"),
            ("workspaces:\n  - id: default\n    map_repo: a/b\n    map_path: m\n    colour: red\n".into(), "unknown field"),
        ];
        for (body, want) in &cases {
            let err = file(body).unwrap_err().to_string();
            assert!(err.contains(want), "{body}\n→ {err}");
        }
        let both =
            from_file("workspaces: []\n", Path::new("w.yml"), &["RUBATO_MAP_REPO"]).unwrap_err();
        assert!(both.to_string().contains("both set"), "{both}");
        println!("checked {} refusals", cases.len() + 1);
    }
}
