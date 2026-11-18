//! At start (ADR 0075): seed `workspace_settings` once, load it into the kernel's registry, and
//! warn for every workspace-scoped variable or file entry that is still set.

use super::store::{self, Row};
use crate::kernel::secrets;
use crate::kernel::state::App;
use crate::kernel::workspaces::{
    Registry, Source, StoredSettings, TrafficSetting, WorkspaceEntry, check,
};
use crate::traffic::{PROMETHEUS_FIELDS, PROMETHEUS_SECRETS, seed_from_env};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

/// A workspace-scoped variable or workspaces-file entry still set at start, and whether its
/// value differs from what the database holds. It is ignored either way: the database owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub setting: String,
    pub differs: bool,
}

/// What `start` did.
#[derive(Debug)]
pub struct Started {
    /// Rows the seed wrote: every workspace on the first start, 0 after.
    pub seeded: usize,
    pub drift: Vec<Drift>,
}

/// Seed once, load the registry, and warn about what is still set. The composition root calls
/// it before anything reads the registry.
pub async fn start(app: &App) -> Result<Started> {
    let seeded = seed(app).await?;
    let rows = store::all(&app.db).await?;
    let registry = registry_of(app, &rows)?;
    let ids: Vec<&str> = registry.entries.iter().map(|w| w.id.as_str()).collect();
    tracing::info!(workspaces = ?ids, seeded, "workspace settings loaded");
    app.set_workspaces(registry);
    let drift = drift(app, &rows);
    for d in &drift {
        tracing::warn!(
            setting = d.setting.as_str(),
            differs = d.differs,
            "workspace setting still set outside the database; the database holds it (ADR 0075)"
        );
    }
    Ok(Started { seeded, drift })
}

/// The first start's seed, in one transaction: a row per workspace of the file (or `default`
/// from `RUBATO_MAP_*`), each with the traffic source and launch defaults the environment sets
/// (a one-time copy, plan 189 Decision 7). Writes nothing once the table has a row.
async fn seed(app: &App) -> Result<usize> {
    let mut tx = app.db.begin().await?;
    if store::count(&mut tx).await? > 0 {
        return Ok(0);
    }
    let (entries, _) = app.config.workspace_entries();
    check(&entries)?;
    let seed_env = &app.config.seed;
    let (source, fields, sealed) = match seed_from_env(|k| app.config.traffic_env.get(k)) {
        None => (None, None, None),
        Some(s) => {
            let mut fields = s.fields;
            if let Some(f) = seed_env.traffic_fetch_secs {
                fields.insert("fetch_secs".into(), f.to_string());
            }
            let sealed = if s.secrets.is_empty() {
                None
            } else {
                let Some(key) = app.config.token_key.as_ref() else {
                    let set: Vec<&str> = PROMETHEUS_SECRETS
                        .iter()
                        .filter(|(field, _)| s.secrets.contains_key(*field))
                        .map(|(_, var)| *var)
                        .collect();
                    bail!(
                        "{} set and RUBATO_TOKEN_KEY unset: a workspace's traffic secrets are stored sealed with it (ADR 0075). Set RUBATO_TOKEN_KEY, or unset {}",
                        set.join(", "),
                        set.join(", ")
                    );
                };
                Some(secrets::seal(key, &serde_json::to_string(&s.secrets)?)?)
            };
            (
                Some(s.source),
                Some(serde_json::to_string(&fields)?),
                sealed,
            )
        }
    };
    let now = crate::kernel::db::ts(app.clock.now());
    for (i, w) in entries.iter().enumerate() {
        store::insert(
            &mut tx,
            &Row {
                workspace_id: w.id.clone(),
                position: i as i64,
                revision: 1,
                name: w.name.clone(),
                map_repo: w.map_repo.clone(),
                map_path: w.map_path.clone(),
                map_ref: w.map_ref.clone(),
                traffic_source: source.clone(),
                traffic_fields: fields.clone(),
                traffic_secrets_sealed: sealed.clone(),
                default_spend_cap_usd: seed_env.default_spend_cap_usd,
                default_attempt_limit: seed_env.default_attempt_limit.map(i64::from),
                updated_by: "seed".into(),
                updated_at: now.clone(),
            },
        )
        .await
        .with_context(|| format!("seeding workspace {}", w.id))?;
    }
    tx.commit().await?;
    tracing::info!(
        rows = entries.len(),
        traffic = source.is_some(),
        "workspace settings seeded from the environment"
    );
    Ok(entries.len())
}

/// One registry entry from its row. `map_file` is never stored: it is read by id from the file
/// or the environment (plan 189 Decision 10).
pub(super) fn entry_of(app: &App, r: &Row) -> Result<WorkspaceEntry> {
    let (from_config, _) = app.config.workspace_entries();
    let fields: BTreeMap<String, String> = match &r.traffic_fields {
        Some(j) => serde_json::from_str(j)
            .with_context(|| format!("workspace {}: traffic fields", r.workspace_id))?,
        None => BTreeMap::new(),
    };
    Ok(WorkspaceEntry {
        id: r.workspace_id.clone(),
        name: r.name.clone(),
        map_repo: r.map_repo.clone(),
        map_path: r.map_path.clone(),
        map_ref: r.map_ref.clone(),
        map_file: from_config
            .iter()
            .find(|w| w.id == r.workspace_id)
            .and_then(|w| w.map_file.clone()),
        settings: StoredSettings {
            revision: r.revision as u64,
            traffic: r.traffic_source.as_ref().map(|s| TrafficSetting {
                source: s.clone(),
                fields,
                secrets_sealed: r.traffic_secrets_sealed.clone(),
            }),
            default_spend_cap_usd: r.default_spend_cap_usd,
            default_attempt_limit: r.default_attempt_limit.map(|n| n as u32),
        },
    })
}

/// The registry from the table, checked as at every start (ADR 0044).
pub(super) fn registry_of(app: &App, rows: &[Row]) -> Result<Registry> {
    let entries = rows
        .iter()
        .map(|r| entry_of(app, r))
        .collect::<Result<Vec<_>>>()?;
    check(&entries)?;
    Ok(Registry {
        entries,
        source: Source::Database,
    })
}

/// Every workspace-scoped variable or file entry still set, and whether it differs from the
/// database: a file entry from its own row, `RUBATO_MAP_*` from `default`'s row (they name that
/// one workspace), a traffic or launch-default variable from any row (issue #281). Values are
/// compared, never logged.
fn drift(app: &App, rows: &[Row]) -> Vec<Drift> {
    let cfg = &app.config;
    let row = |id: &str| rows.iter().find(|r| r.workspace_id == id);
    let default = row(rubato_protocol::DEFAULT_WORKSPACE);
    let mut out = Vec::new();
    for w in &cfg.workspaces {
        let differs = row(&w.id).is_none_or(|r| {
            r.name != w.name
                || !r.map_repo.eq_ignore_ascii_case(&w.map_repo)
                || r.map_path != w.map_path
                || r.map_ref != w.map_ref
        });
        out.push(Drift {
            setting: format!("RUBATO_WORKSPACES_FILE entry {}", w.id),
            differs,
        });
    }
    for var in &cfg.seed.map_vars {
        let differs = default.is_none_or(|r| match var.as_str() {
            "RUBATO_MAP_REPO" => !r.map_repo.eq_ignore_ascii_case(&cfg.map_repo),
            "RUBATO_MAP_PATH" => r.map_path != cfg.map_path,
            _ => r.map_ref != cfg.map_ref,
        });
        out.push(Drift {
            setting: var.clone(),
            differs,
        });
    }
    // The traffic source and the launch defaults were copied to every row, and each workspace
    // has had its own since (ADR 0075): a variable differs when any workspace's row does.
    type Values = BTreeMap<String, String>;
    let stored: Vec<(Values, Option<Values>)> = rows
        .iter()
        .map(|r| {
            let fields = r
                .traffic_fields
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default();
            let secrets = r
                .traffic_secrets_sealed
                .as_deref()
                .and_then(|s| secrets::open(cfg.token_key.as_ref()?, s).ok())
                .and_then(|j| serde_json::from_str(&j).ok());
            (fields, secrets)
        })
        .collect();
    let any_row = |differs: &dyn Fn(usize) -> bool| rows.is_empty() || (0..rows.len()).any(differs);
    for (var, value) in &cfg.traffic_env.0 {
        let field = PROMETHEUS_FIELDS
            .iter()
            .find(|(_, v)| v == var)
            .map(|(f, _)| (*f, false))
            .or_else(|| {
                PROMETHEUS_SECRETS
                    .iter()
                    .find(|(_, v)| v == var)
                    .map(|(f, _)| (*f, true))
            });
        let differs = any_row(&|i| match field {
            Some((f, false)) => stored[i].0.get(f) != Some(value),
            Some((f, true)) => stored[i].1.as_ref().is_none_or(|s| s.get(f) != Some(value)),
            None => true,
        });
        out.push(Drift {
            setting: var.clone(),
            differs,
        });
    }
    if let Some(f) = cfg.seed.traffic_fetch_secs {
        let f = f.to_string();
        out.push(Drift {
            setting: "RUBATO_TRAFFIC_FETCH_SECS".into(),
            differs: any_row(&|i| stored[i].0.get("fetch_secs") != Some(&f)),
        });
    }
    if let Some(c) = cfg.seed.default_spend_cap_usd {
        out.push(Drift {
            setting: "RUBATO_DEFAULT_SPEND_CAP_USD".into(),
            differs: any_row(&|i| rows[i].default_spend_cap_usd != Some(c)),
        });
    }
    if let Some(n) = cfg.seed.default_attempt_limit {
        out.push(Drift {
            setting: "RUBATO_DEFAULT_ATTEMPT_LIMIT".into(),
            differs: any_row(&|i| rows[i].default_attempt_limit != Some(i64::from(n))),
        });
    }
    out
}
