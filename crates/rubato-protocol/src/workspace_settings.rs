//! Workspace settings set at runtime (ADR 0075): what the Configure workspace view reads and
//! writes. A workspace's name, map source, traffic source and launch defaults; never an
//! application setting. The traffic source is backend-neutral (ADR 0040): an adapter's name, its
//! fields as text, and its secrets only as set or not set. The view draws the form from the
//! field list the adapter publishes.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Whether a secret is stored. A secret's value is never returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretState {
    pub set: bool,
}

/// One workspace's traffic source as the view reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficSourceSettings {
    /// The adapter's name, one of `SourceFields.source`.
    pub source: String,
    /// The adapter's text fields, by `SourceField.name`. Only the adapter knows what they mean.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    /// Each secret field of the adapter, by name: set or not set.
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretState>,
}

/// `GET` `routes::SETTINGS`: one workspace's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceSettings {
    pub id: String,
    /// The compare-and-set token: an update carries the revision it read.
    pub revision: u64,
    /// What people call it; absent: the map's own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `owner/name` of the repo holding the map. Fixed once the workspace exists.
    pub map_repo: String,
    /// Fixed once the workspace exists.
    pub map_path: String,
    pub map_ref: String,
    /// Absent: the workspace has no traffic source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<TrafficSourceSettings>,
    /// Absent: the built-in 25 USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_spend_cap_usd: Option<f64>,
    /// Absent: the built-in 3 attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_attempt_limit: Option<u32>,
    /// The login of whoever changed it last; `seed` for a row the first start wrote.
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

/// How a text field is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Url,
    Number,
}

/// One field of a traffic adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceField {
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    /// A secret is write-only: sent in `TrafficSourceUpdate.secrets`, never in `fields`.
    pub secret: bool,
    /// What the adapter uses when the field is left empty, as text for the form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// `GET` `routes::TRAFFIC_SOURCES`: one adapter's field list, published by the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceFields {
    /// The adapter's name, as `TrafficSourceSettings.source` carries it.
    pub source: String,
    pub label: String,
    pub fields: Vec<SourceField>,
}

/// What an update does to one secret. A secret an update does not name is kept.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SecretUpdate {
    /// Keep what is stored. A changed URL origin clears a kept secret (ADR 0075).
    Keep,
    Set {
        value: String,
    },
    Clear,
}

/// The action only: a secret's value is never printed.
impl std::fmt::Debug for SecretUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SecretUpdate::Keep => "Keep",
            SecretUpdate::Set { .. } => "Set",
            SecretUpdate::Clear => "Clear",
        })
    }
}

/// The traffic source an update asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TrafficSourceUpdate {
    pub source: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretUpdate>,
}

/// `PUT` `routes::SETTINGS`: the whole settings as they should be. `map_repo` and `map_path`
/// are carried so a change to them is refused by name instead of ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceSettingsUpdate {
    /// The revision this edit read. Another: `409 changed_meanwhile`.
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub map_repo: String,
    pub map_path: String,
    pub map_ref: String,
    /// Absent: no traffic source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<TrafficSourceUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_spend_cap_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_attempt_limit: Option<u32>,
}

/// `POST` `workspace::routes::WORKSPACES`: a new workspace. Its traffic source and launch
/// defaults are set afterwards, with an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewWorkspace {
    /// `[a-z0-9-]{1,32}`.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub map_repo: String,
    pub map_path: String,
    #[serde(default = "main")]
    pub map_ref: String,
}

fn main() -> String {
    "main".into()
}

/// The body of a refused settings write: why, in words for the person, and for a stale edit the
/// settings as they are now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SettingsRefusal {
    /// One of [`reason`].
    pub reason: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<WorkspaceSettings>,
}

/// Why a settings write is refused: `SettingsRefusal.reason`, and the `reason` of
/// `rubato.web.rejected`.
pub mod reason {
    pub const BAD_ID: &str = "bad_id";
    pub const DUPLICATE: &str = "duplicate";
    pub const SAME_MAP: &str = "same_map";
    pub const DEFAULT_KEPT: &str = "default_kept";
    /// Rows of a deleted workspace with this id are still kept.
    pub const ID_HAS_ROWS: &str = "id_has_rows";
    pub const MAP_SOURCE_FIXED: &str = "map_source_fixed";
    /// The workspace's map is a file on the server: its ref cannot move.
    pub const MAP_FILE: &str = "map_file";
    pub const DRAFT_OPEN: &str = "draft_open";
    pub const MAP_INVALID: &str = "map_invalid";
    pub const REPO_UNREADABLE: &str = "repo_unreadable";
    pub const URL_REFUSED: &str = "url_refused";
    pub const NO_TOKEN_KEY: &str = "no_token_key";
    pub const DEV_TRAFFIC_SET: &str = "dev_traffic_set";
    pub const CHANGED_MEANWHILE: &str = "changed_meanwhile";
    pub const OPEN_LOGIN: &str = "open_login";
    /// A value that does not read: a name too long, a cap that is not a positive number, a
    /// field the adapter does not have or cannot parse.
    pub const BAD_VALUE: &str = "bad_value";

    pub const ALL: [&str; 16] = [
        BAD_ID,
        DUPLICATE,
        SAME_MAP,
        DEFAULT_KEPT,
        ID_HAS_ROWS,
        MAP_SOURCE_FIXED,
        MAP_FILE,
        DRAFT_OPEN,
        MAP_INVALID,
        REPO_UNREADABLE,
        URL_REFUSED,
        NO_TOKEN_KEY,
        DEV_TRAFFIC_SET,
        CHANGED_MEANWHILE,
        OPEN_LOGIN,
        BAD_VALUE,
    ];
}

/// The `reason` of the `WebEvent::Changed` a settings change sends.
pub mod changed {
    /// One workspace's settings changed; carries the workspace.
    pub const SETTINGS: &str = "settings";
    /// The list of workspaces changed (a create, a delete, a new name); unscoped.
    pub const WORKSPACES: &str = "workspaces";
}

/// A workspace name's longest length, in characters.
pub const MAX_NAME_CHARS: usize = 80;

pub mod routes {
    /// `GET` reads, `PUT` updates one workspace's settings. Map-scoped (ADR 0049).
    pub const SETTINGS: &str = "/api/settings";
    /// `DELETE`: remove a workspace of a tenant; its rows are kept (ADR 0089). The full path:
    /// mounted as it is.
    pub const WORKSPACE: &str = "/api/t/{tenant}/w/{ws}";
    /// `GET`: every traffic adapter's field list. A tenant route.
    pub const TRAFFIC_SOURCES: &str = "/api/traffic/sources";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_update_never_prints_its_value() {
        let u = TrafficSourceUpdate {
            source: "prometheus".into(),
            fields: BTreeMap::new(),
            secrets: [(
                "headers".to_owned(),
                SecretUpdate::Set {
                    value: "Authorization=Bearer s3cret".into(),
                },
            )]
            .into(),
        };
        let printed = format!("{u:?}");
        assert!(!printed.contains("s3cret"), "{printed}");
        assert!(printed.contains("Set"), "{printed}");
    }

    #[test]
    fn an_update_reads_its_wire_form() {
        let u: WorkspaceSettingsUpdate = serde_json::from_str(
            r#"{"revision":4,"map_repo":"a/b","map_path":"m.yml","map_ref":"main",
                "traffic":{"source":"prometheus","fields":{"url":"http://prom.lan:9090"},
                           "secrets":{"headers":{"action":"keep"}}}}"#,
        )
        .unwrap();
        assert_eq!(u.revision, 4);
        assert_eq!(
            u.traffic.unwrap().secrets["headers"],
            SecretUpdate::Keep,
            "keep, set or clear"
        );
        let n: NewWorkspace =
            serde_json::from_str(r#"{"id":"shop","map_repo":"a/b","map_path":"m.yml"}"#).unwrap();
        assert_eq!(n.map_ref, "main");
        assert_eq!(reason::ALL.len(), 16);
    }
}
