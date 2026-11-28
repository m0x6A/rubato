//! Which workspaces this server holds (ADR 0044): read from `RUBATO_WORKSPACES_FILE`, or made
//! from the legacy `RUBATO_MAP_*` variables as the one workspace `default`. Every refusal is
//! decided here, at start, with the reason.

use crate::kernel::tenancy::{TenantId, WsKey};
use anyhow::{Result, bail};
use rubato_protocol::{DEFAULT_WORKSPACE, is_workspace_id};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One workspace: one map, read from GitHub or, for local runs and tests, from a file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEntry {
    /// The tenant that holds it (ADR 0087). A workspaces file and the legacy variables know no
    /// tenant: their entries are tenant `default`'s.
    #[serde(skip, default = "TenantId::default_tenant")]
    pub tenant: TenantId,
    /// Unique inside its tenant (ADR 0089).
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
    /// What only the database holds (ADR 0075): the revision and the settings beyond the map
    /// source. Default, revision 0, for an entry read from the file or the environment.
    #[serde(skip)]
    pub settings: StoredSettings,
}

/// A workspace's settings as the registry carries them (ADR 0075). Secrets stay sealed here;
/// the slice that uses them opens them with `kernel::secrets::open`. `Debug` prints no field
/// value and no secret.
#[derive(Clone, Default, PartialEq)]
pub struct StoredSettings {
    /// 0 for an entry not read from `workspace_settings`; else the row's revision.
    pub revision: u64,
    pub traffic: Option<TrafficSetting>,
    pub default_spend_cap_usd: Option<f64>,
    pub default_attempt_limit: Option<u32>,
}

/// One workspace's traffic source: the adapter's name, its text fields and its sealed secrets.
/// Only the adapter knows what the fields mean (ADR 0040).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TrafficSetting {
    pub source: String,
    pub fields: std::collections::BTreeMap<String, String>,
    /// A JSON object of the secrets, sealed with `RUBATO_TOKEN_KEY`.
    pub secrets_sealed: Option<String>,
}

impl std::fmt::Debug for StoredSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSettings")
            .field("revision", &self.revision)
            .field("traffic", &self.traffic)
            .field("default_spend_cap_usd", &self.default_spend_cap_usd)
            .field("default_attempt_limit", &self.default_attempt_limit)
            .finish()
    }
}

impl std::fmt::Debug for TrafficSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrafficSetting")
            .field("source", &self.source)
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets_sealed.as_ref().map(|_| "set"))
            .finish()
    }
}

impl WorkspaceEntry {
    pub fn key(&self) -> WsKey {
        WsKey::new(self.tenant.clone(), self.id.clone())
    }
}

/// The workspaces a server holds right now, in switcher order, and where the list came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    pub entries: Vec<WorkspaceEntry>,
    pub source: Source,
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
    /// Table `workspace_settings`, the source after the first start (ADR 0075).
    Database,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::File => "file",
            Source::LegacyEnv => "legacy_env",
            Source::Database => "database",
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
    check_file(&out)?;
    Ok(out)
}

/// The refusals of ADR 0044, in the order a person fixes them. An id and a map are unique
/// inside one tenant (ADR 0089): two tenants may each hold a workspace `main`.
pub fn check(entries: &[WorkspaceEntry]) -> Result<()> {
    for w in entries {
        if !is_workspace_id(&w.id) {
            bail!("workspace id {:?} is not [a-z0-9-]{{1,32}}", w.id);
        }
    }
    for (i, w) in entries.iter().enumerate() {
        if entries[..i]
            .iter()
            .any(|o| o.id == w.id && o.tenant == w.tenant)
        {
            bail!("workspace id {} is listed twice", w.id);
        }
        if let Some(o) = entries[..i].iter().find(|o| {
            o.tenant == w.tenant
                && o.map_repo.eq_ignore_ascii_case(&w.map_repo)
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
    Ok(())
}

/// A workspaces file, or the legacy variables: tenant `default`'s list as it was before tenants.
/// It keeps its workspace `default`, which holds every row from before workspaces (ADR 0044). No
/// other tenant needs one (plan 345, Decision 12).
pub fn check_file(entries: &[WorkspaceEntry]) -> Result<()> {
    if entries.is_empty() {
        bail!("the workspaces file lists no workspace");
    }
    check(entries)?;
    if !entries.iter().any(|w| w.id == DEFAULT_WORKSPACE) {
        bail!(
            "the workspaces file has no entry with id {DEFAULT_WORKSPACE}; it keeps every row from before workspaces"
        );
    }
    Ok(())
}

/// Tables that hold a tenant's workspace id (ADR 0047, ADR 0089), for the orphan count.
pub const SCOPED_TABLES: [&str; 11] = [
    "map_layout",
    "map_layout_grouped",
    "traffic_dismissals",
    "map_cache",
    "map_proposal",
    "pins",
    "map_line_pins",
    "map_draft_ops",
    "issue_creations",
    "issue_suggestions",
    "launches",
];

/// At start: log which workspaces this server holds (`workspace config`), record how many, and
/// count rows whose workspace is no longer configured. Those rows are kept, never deleted, and
/// logged so a removed or renamed entry is noticed.
pub async fn report(app: &crate::kernel::state::App) -> Result<u64> {
    let (entries, source) = app.workspace_entries();
    let keys: Vec<String> = entries.iter().map(|w| w.key().to_string()).collect();
    let ids: Vec<&str> = entries.iter().map(|w| w.id.as_str()).collect();
    tracing::info!(workspaces = ?keys, source = source.as_str(), "workspace config");
    app.metrics
        .workspaces_configured
        .record(entries.len() as u64, &[]);
    let placeholders = vec!["?"; ids.len()].join(", ");
    let mut orphans = 0u64;
    for t in SCOPED_TABLES {
        // Over every tenant on purpose: this is the server's own count at start, for no tenant.
        let sql = format!(
            "SELECT tenant_id, workspace_id, COUNT(*) AS n FROM {t} WHERE (tenant_id || '/' || workspace_id) NOT IN ({placeholders}) GROUP BY tenant_id, workspace_id"
        );
        let mut q = sqlx::query(&sql);
        for key in &keys {
            q = q.bind(key);
        }
        for r in q.fetch_all(&app.db).await? {
            use sqlx::Row;
            let n = r.get::<i64, _>("n") as u64;
            let tenant: String = r.get("tenant_id");
            let ws: String = r.get("workspace_id");
            tracing::warn!(table = t, rubato.tenant = %tenant, rubato.workspace = %ws, rows = n, "orphan workspace rows");
            orphans += n;
        }
    }
    app.metrics.workspace_orphan_rows.record(orphans, &[]);
    Ok(orphans)
}

/// The first table that still holds a row of workspace `id` (ADR 0075): a deleted workspace's
/// rows are kept, so its id cannot be created again while one remains.
pub async fn id_has_rows(db: &sqlx::SqlitePool, id: &WsKey) -> Result<Option<&'static str>> {
    for t in SCOPED_TABLES {
        let sql =
            format!("SELECT EXISTS(SELECT 1 FROM {t} WHERE tenant_id = ? AND workspace_id = ?)");
        if sqlx::query_scalar::<_, bool>(&sql)
            .bind(id.tenant.as_str())
            .bind(id.ws())
            .fetch_one(db)
            .await?
        {
            return Ok(Some(t));
        }
    }
    Ok(None)
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
