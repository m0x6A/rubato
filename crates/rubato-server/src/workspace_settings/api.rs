//! The settings routes (ADR 0075): read a workspace's settings, change them, create a workspace,
//! delete one, and the traffic adapters' field lists.
//!
//! Every write runs under one lock, from its first read to the registry swap, so two writers
//! cannot reorder their swaps and two creates cannot both pass the checks. A change is a
//! compare-and-set on the row's revision. GitHub is read under the lock but outside any SQLite
//! transaction. A refusal changes nothing, answers with its reason and is counted on
//! `rubato.web.rejected`.

use super::seed::{entry_of, registry_of};
use super::store::{self, Row};
use crate::auth::CurrentUser;
use crate::kernel::config::is_local;
use crate::kernel::db::{parse_ts, ts};
use crate::kernel::http::{Ws, internal};
use crate::kernel::secrets;
use crate::kernel::state::{App, LoadedMap};
use crate::kernel::tenancy::WsKey;
use crate::kernel::tenancy::{AdminScope, TenantId};
use crate::kernel::workspaces::{self, WorkspaceEntry};
use crate::{map, map_draft, traffic};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rubato_protocol::otel::span;
use rubato_protocol::workspace::routes::WORKSPACES;
use rubato_protocol::workspace_settings::{MAX_NAME_CHARS, changed, reason, routes};
use rubato_protocol::{
    DEFAULT_WORKSPACE, NewWorkspace, SecretState, SecretUpdate, SettingsRefusal, SourceFields,
    TrafficSourceSettings, TrafficSourceUpdate, WorkspaceSettings, WorkspaceSettingsUpdate,
    is_workspace_id,
};
use std::collections::{BTreeMap, BTreeSet};

const SLICE: &str = "workspace_settings";

/// The one write lock of the slice.
#[derive(Default)]
struct WriteLock(tokio::sync::Mutex<()>);

/// A refusal before it is counted: the status, the reason and words for the person.
struct Refusal {
    status: StatusCode,
    reason: &'static str,
    message: String,
    current: Option<Box<WorkspaceSettings>>,
}

/// A handler's early answer, boxed so a `Result` carrying it stays small.
pub struct Stop(Box<Response>);

impl IntoResponse for Stop {
    fn into_response(self) -> Response {
        *self.0
    }
}

impl From<StatusCode> for Stop {
    fn from(status: StatusCode) -> Self {
        Stop(Box::new(status.into_response()))
    }
}

impl Refusal {
    fn new(status: StatusCode, reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            reason,
            message: message.into(),
            current: None,
        }
    }

    fn unprocessable(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, reason, message)
    }

    fn conflict(reason: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, reason, message)
    }

    /// Count it and make it the answer.
    fn answer(self, app: &App, route: &str, user: &str) -> Stop {
        app.reject(route, self.reason, user);
        let response = (
            self.status,
            Json(SettingsRefusal {
                reason: self.reason.to_owned(),
                message: self.message,
                current: self.current.map(|c| *c),
            }),
        )
            .into_response();
        Stop(Box::new(response))
    }
}

fn failed(e: impl std::fmt::Display) -> Stop {
    internal(SLICE, e).into()
}

/// A server open to any GitHub account takes no settings write (ADR 0075, ADR 0064): a write
/// can make the server send a request.
fn open_login(app: &App) -> Option<Refusal> {
    let open = app.config.oauth_without_app() && !is_local(app.config.bind, &app.config.public_url);
    open.then(|| {
        Refusal::new(
            StatusCode::FORBIDDEN,
            reason::OPEN_LOGIN,
            "this server lets any GitHub account log in, so workspace settings cannot be changed here; configure the GitHub App",
        )
    })
}

fn fields_of(row: &Row) -> BTreeMap<String, String> {
    row.traffic_fields
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default()
}

/// The names of the secrets a row holds. When they cannot be opened (no key, or another key)
/// every secret of the adapter counts as set: something is stored.
fn stored_secret_names(app: &App, row: &Row) -> BTreeSet<String> {
    let Some(sealed) = &row.traffic_secrets_sealed else {
        return BTreeSet::new();
    };
    opened(app, sealed)
        .map(|m| m.into_keys().collect())
        .unwrap_or_else(|| {
            traffic::secret_names(row.traffic_source.as_deref().unwrap_or_default())
                .into_iter()
                .map(str::to_owned)
                .collect()
        })
}

fn opened(app: &App, sealed: &str) -> Option<BTreeMap<String, String>> {
    let key = app.config.token_key.as_ref()?;
    serde_json::from_str(&secrets::open(key, sealed).ok()?).ok()
}

/// A row as the view reads it: secrets only as set or not set.
fn wire(app: &App, row: &Row) -> WorkspaceSettings {
    let traffic = row.traffic_source.as_ref().map(|source| {
        let set = stored_secret_names(app, row);
        TrafficSourceSettings {
            source: source.clone(),
            fields: fields_of(row),
            secrets: traffic::secret_names(source)
                .into_iter()
                .map(|n| {
                    (
                        n.to_owned(),
                        SecretState {
                            set: set.contains(n),
                        },
                    )
                })
                .collect(),
        }
    });
    WorkspaceSettings {
        id: row.workspace_id.clone(),
        revision: row.revision as u64,
        name: row.name.clone(),
        map_repo: row.map_repo.clone(),
        map_path: row.map_path.clone(),
        map_ref: row.map_ref.clone(),
        traffic,
        default_spend_cap_usd: row.default_spend_cap_usd,
        default_attempt_limit: row.default_attempt_limit.map(|n| n as u32),
        updated_by: row.updated_by.clone(),
        updated_at: parse_ts(&row.updated_at),
    }
}

/// `GET /api/settings`, per workspace.
pub async fn settings(
    State(app): State<App>,
    ws: Ws,
    _user: CurrentUser,
) -> Result<Json<WorkspaceSettings>, StatusCode> {
    let row = store::get(&app.db, ws.key())
        .await
        .map_err(|e| internal(SLICE, e))?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(wire(&app, &row)))
}

/// `GET /api/traffic/sources`: the field list of every adapter, as the adapters publish them.
pub async fn sources(_user: CurrentUser) -> Json<Vec<SourceFields>> {
    Json(traffic::sources())
}

fn clean_name(name: Option<&str>) -> Result<Option<String>, Refusal> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(Refusal::unprocessable(
            reason::BAD_VALUE,
            format!("a name is at most {MAX_NAME_CHARS} characters on one line"),
        ));
    }
    Ok(Some(name.to_owned()))
}

fn clean_ref(map_ref: &str) -> Result<String, Refusal> {
    let r = map_ref.trim();
    if r.is_empty() || r.len() > 200 || r.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Refusal::unprocessable(
            reason::BAD_VALUE,
            "the map ref is a branch name",
        ));
    }
    Ok(r.to_owned())
}

/// What the stored traffic columns become: source, fields as JSON, sealed secrets.
type TrafficColumns = (Option<String>, Option<String>, Option<String>);

/// Work out the traffic source an update asks for. Adds the names of what changes to `keys`;
/// with none added the stored columns come back untouched and nothing is checked, so a source
/// stored before a rule tightened does not block a rename.
fn traffic_change(
    app: &App,
    row: &Row,
    want: Option<&TrafficSourceUpdate>,
    keys: &mut Vec<String>,
) -> Result<TrafficColumns, Refusal> {
    let stored: TrafficColumns = (
        row.traffic_source.clone(),
        row.traffic_fields.clone(),
        row.traffic_secrets_sealed.clone(),
    );
    let Some(want) = want else {
        if row.traffic_source.is_some() {
            keys.push("traffic.source".into());
        }
        return Ok((None, None, None));
    };
    let bad = |m: String| Refusal::unprocessable(reason::BAD_VALUE, m);
    let secret_names = traffic::secret_names(&want.source);
    if let Some(unknown) = want
        .secrets
        .keys()
        .find(|n| !secret_names.contains(&n.as_str()))
    {
        return Err(bad(format!(
            "{unknown:?} is not a secret of the {} source",
            want.source
        )));
    }
    let fields: BTreeMap<String, String> = want
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), v.trim().to_owned()))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let stored_fields = fields_of(row);
    let same_source = row.traffic_source.as_deref() == Some(want.source.as_str());
    // A stored secret belongs to the origin it was set for: a new origin drops it unless this
    // request sets it, so a URL cannot be repointed to collect it.
    let same_origin =
        same_source && traffic::source_origin(&stored_fields) == traffic::source_origin(&fields);
    let had = stored_secret_names(app, row);
    let mut changed: Vec<String> = Vec::new();
    if !same_source {
        changed.push("traffic.source".into());
    }
    let names: BTreeSet<&String> = stored_fields.keys().chain(fields.keys()).collect();
    for name in names {
        if !same_source || stored_fields.get(name) != fields.get(name) {
            changed.push(format!("traffic.{name}"));
        }
    }
    let mut touched = false;
    for name in &secret_names {
        let update = want.secrets.get(*name).unwrap_or(&SecretUpdate::Keep);
        let differs = match update {
            SecretUpdate::Keep => !same_origin && had.contains(*name),
            SecretUpdate::Clear => had.contains(*name),
            SecretUpdate::Set { value } => !value.trim().is_empty() || had.contains(*name),
        };
        touched |= !matches!(update, SecretUpdate::Keep);
        if differs {
            changed.push(format!("traffic.secrets.{name}"));
        }
    }
    if changed.is_empty() {
        return Ok(stored);
    }
    if app.config.dev_traffic.is_some() {
        return Err(Refusal::conflict(
            reason::DEV_TRAFFIC_SET,
            "RUBATO_DEV_TRAFFIC is set on this server: every workspace reads that file, and a stored source would be ignored",
        ));
    }
    let key = app.config.token_key.as_ref();
    let mut plain: BTreeMap<String, String> = match (&row.traffic_secrets_sealed, same_origin) {
        (Some(sealed), true) if touched => opened(app, sealed).unwrap_or_default(),
        _ => BTreeMap::new(),
    };
    let mut sets = false;
    for (name, update) in &want.secrets {
        match update {
            SecretUpdate::Keep => {}
            SecretUpdate::Set { value } if !value.trim().is_empty() => {
                sets = true;
                plain.insert(name.clone(), value.trim().to_owned());
            }
            SecretUpdate::Set { .. } | SecretUpdate::Clear => {
                plain.remove(name);
            }
        }
    }
    if sets && key.is_none() {
        return Err(Refusal::unprocessable(
            reason::NO_TOKEN_KEY,
            "RUBATO_TOKEN_KEY is not set on this server, so it cannot keep a secret; set the key, or leave the secret out",
        ));
    }
    let local = is_local(app.config.bind, &app.config.public_url);
    traffic::check_source(&want.source, &fields, &plain, local).map_err(|e| {
        let reason = match e.url {
            true => reason::URL_REFUSED,
            false => reason::BAD_VALUE,
        };
        Refusal::unprocessable(reason, e.message)
    })?;
    let sealed = match (touched, same_origin) {
        // Nothing set or cleared: what is stored stays as it is, for the same origin only.
        (false, true) => row.traffic_secrets_sealed.clone(),
        (false, false) => None,
        (true, _) if plain.is_empty() => None,
        (true, _) => {
            let key = key.ok_or_else(|| {
                Refusal::unprocessable(
                    reason::NO_TOKEN_KEY,
                    "RUBATO_TOKEN_KEY is not set on this server, so the stored secrets cannot be kept; clear them all, or set the key",
                )
            })?;
            let json = serde_json::to_string(&plain).map_err(|e| bad(e.to_string()))?;
            Some(secrets::seal(key, &json).map_err(|e| bad(e.to_string()))?)
        }
    };
    let fields_json = serde_json::to_string(&fields).map_err(|e| bad(e.to_string()))?;
    keys.extend(changed);
    Ok((Some(want.source.clone()), Some(fields_json), sealed))
}

/// A repository a workspace of `tenant` may use: one that an installation attached to that
/// tenant covers (ADR 0087, superseding ADR 0075's read with the member's own token). The same
/// answer for a repository of another tenant and one nobody covers.
fn covered(app: &App, tenant: &TenantId, user: &str, repo: &str) -> Result<(), Refusal> {
    if app.github.tenant_of(repo).as_ref() == Some(tenant) {
        return Ok(());
    }
    tracing::info!(login = user, rubato.tenant = %tenant, rubato.repo = repo, "the repository is not the tenant's");
    Err(Refusal::unprocessable(
        reason::REPO_UNREADABLE,
        format!(
            "{repo} is not covered by a GitHub App installation of this tenant, so a workspace cannot use it"
        ),
    ))
}

/// The three checks of a new map source or a moved ref (ADR 0075, ADR 0087), in an order that
/// tells an admin nothing about a repository outside their tenant: an installation of the
/// tenant covers the map repo; the map loads and validates fresh at that ref; an installation
/// of the tenant covers every repo the map names.
async fn vet(
    app: &App,
    tenant: &TenantId,
    user: &str,
    entry: &WorkspaceEntry,
) -> Result<LoadedMap, Refusal> {
    if entry.map_file.is_none() {
        covered(app, tenant, user, &entry.map_repo)?;
    }
    let loaded = map::fetch_and_validate(app, entry).await.map_err(|e| {
        Refusal::unprocessable(
            reason::MAP_INVALID,
            format!(
                "the map does not load from {} {} at {}: {e:#}",
                entry.map_repo, entry.map_path, entry.map_ref
            ),
        )
    })?;
    for repo in loaded.map.repos_named() {
        if !repo.eq_ignore_ascii_case(&entry.map_repo) || entry.map_file.is_some() {
            covered(app, tenant, user, repo)?;
        }
    }
    Ok(loaded)
}

fn same_map(a: &WorkspaceEntry, repo: &str, path: &str, map_ref: &str) -> bool {
    a.map_repo.eq_ignore_ascii_case(repo) && a.map_path == path && a.map_ref == map_ref
}

/// The change record (ADR 0075): who, which workspace, which keys. Never a value.
fn record(ws: &WsKey, user: &str, keys: &[String]) {
    let keys = keys.join(",");
    let span = tracing::info_span!(
        span::WORKSPACE_CONFIG_CHANGED,
        rubato.tenant = %ws.tenant,
        rubato.workspace = ws.ws(),
        rubato.user = user,
        rubato.workspace.changed_keys = %keys
    );
    let _entered = span.enter();
    tracing::info!(
        rubato.tenant = %ws.tenant,
        rubato.workspace = ws.ws(),
        rubato.user = user,
        keys = %keys,
        "workspace config changed"
    );
}

/// Replace the registry with what the table holds now. Every reader sees it at once and the
/// traffic supervisor is woken.
async fn swap(app: &App) -> anyhow::Result<()> {
    let rows = store::all(&app.db).await?;
    app.set_workspaces(registry_of(app, &rows)?);
    Ok(())
}

/// The orphan count and `rubato.workspaces.configured`, again: the list changed.
async fn report_again(app: &App) {
    if let Err(e) = workspaces::report(app).await {
        tracing::error!(slice = SLICE, error = %format!("{e:#}"), "workspace report failed");
    }
}

/// `PUT /api/settings`, per workspace: the settings as they should be, at the revision read.
/// An admin's to change (ADR 0088).
pub async fn update(
    State(app): State<App>,
    ws: Ws,
    _admin: AdminScope,
    CurrentUser(user): CurrentUser,
    Json(req): Json<WorkspaceSettingsUpdate>,
) -> Result<Json<WorkspaceSettings>, Stop> {
    let route = routes::SETTINGS;
    let refuse = |r: Refusal| r.answer(&app, route, &user);
    if let Some(r) = open_login(&app) {
        return Err(refuse(r));
    }
    let lock = app.slots.get::<WriteLock>();
    let _held = lock.0.lock().await;
    let row = store::get(&app.db, ws.key())
        .await
        .map_err(failed)?
        .ok_or(Stop::from(StatusCode::NOT_FOUND))?;
    let stale = |row: &Row| Refusal {
        current: Some(Box::new(wire(&app, row))),
        ..Refusal::conflict(
            reason::CHANGED_MEANWHILE,
            "someone changed these settings meanwhile; read them again",
        )
    };
    if row.revision as u64 != req.revision {
        return Err(refuse(stale(&row)));
    }
    if !row.map_repo.eq_ignore_ascii_case(req.map_repo.trim())
        || row.map_path != req.map_path.trim()
    {
        return Err(refuse(Refusal::unprocessable(
            reason::MAP_SOURCE_FIXED,
            "a workspace's map repo and path are fixed: its pins, layout and drafts belong to that map. Create another workspace for another map",
        )));
    }
    let mut next = row.clone();
    let mut keys: Vec<String> = Vec::new();
    next.name = clean_name(req.name.as_deref()).map_err(refuse)?;
    if next.name != row.name {
        keys.push("name".into());
    }
    if let Some(cap) = req.default_spend_cap_usd
        && !(cap.is_finite() && cap > 0.0)
    {
        return Err(refuse(Refusal::unprocessable(
            reason::BAD_VALUE,
            "the spend cap is a number of USD above 0",
        )));
    }
    if req.default_attempt_limit == Some(0) {
        return Err(refuse(Refusal::unprocessable(
            reason::BAD_VALUE,
            "the attempt limit is at least 1",
        )));
    }
    next.default_spend_cap_usd = req.default_spend_cap_usd;
    if next.default_spend_cap_usd != row.default_spend_cap_usd {
        keys.push("default_spend_cap_usd".into());
    }
    next.default_attempt_limit = req.default_attempt_limit.map(i64::from);
    if next.default_attempt_limit != row.default_attempt_limit {
        keys.push("default_attempt_limit".into());
    }
    (
        next.traffic_source,
        next.traffic_fields,
        next.traffic_secrets_sealed,
    ) = traffic_change(&app, &row, req.traffic.as_ref(), &mut keys).map_err(refuse)?;
    next.map_ref = clean_ref(&req.map_ref).map_err(refuse)?;
    let mut loaded = None;
    if next.map_ref != row.map_ref {
        keys.push("map_ref".into());
        let entry = entry_of(&app, &next).map_err(failed)?;
        if entry.map_file.is_some() {
            return Err(refuse(Refusal::conflict(
                reason::MAP_FILE,
                "this workspace's map is a file on the server, so its ref cannot move",
            )));
        }
        if map_draft::is_open(&app.db, ws.key())
            .await
            .map_err(failed)?
        {
            return Err(refuse(Refusal::conflict(
                reason::DRAFT_OPEN,
                "the map draft is open: discard it, or merge or withdraw its proposal, before the map's ref moves",
            )));
        }
        let entries = app.workspaces_of(ws.key().tenant());
        if let Some(other) = entries.iter().find(|o| {
            o.id != row.workspace_id && same_map(o, &next.map_repo, &next.map_path, &next.map_ref)
        }) {
            return Err(refuse(Refusal::conflict(
                reason::SAME_MAP,
                format!("workspace {} already shows that map at that ref", other.id),
            )));
        }
        loaded = Some(
            vet(&app, ws.key().tenant(), &user, &entry)
                .await
                .map_err(refuse)?,
        );
    }
    if keys.is_empty() {
        return Ok(Json(wire(&app, &row)));
    }
    next.updated_by = user.clone();
    next.updated_at = ts(app.clock.now());
    if !store::update(&app.db, &next, row.revision)
        .await
        .map_err(failed)?
    {
        let now = store::get(&app.db, ws.key())
            .await
            .map_err(failed)?
            .unwrap_or(row);
        return Err(refuse(stale(&now)));
    }
    next.revision = row.revision + 1;
    record(ws.key(), &user, &keys);
    swap(&app).await.map_err(failed)?;
    if let Some(loaded) = loaded {
        // The registry holds the new revision now, so a poll's load of the old ref is dropped
        // and this one applies.
        let entry = entry_of(&app, &next).map_err(failed)?;
        map::apply(&app, &entry, loaded).await.map_err(failed)?;
        app.notify_ws("map", ws.key());
    }
    app.notify_ws(changed::SETTINGS, ws.key());
    if next.name != row.name {
        app.notify_in(ws.key().tenant(), changed::WORKSPACES);
    }
    Ok(Json(wire(&app, &next)))
}

fn clean_source(req: &NewWorkspace) -> Result<(String, String, String), Refusal> {
    let bad = |m: &str| Refusal::unprocessable(reason::BAD_VALUE, m);
    let repo = req.map_repo.trim();
    let part = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if !repo
        .split_once('/')
        .is_some_and(|(o, n)| part(o) && part(n))
    {
        return Err(bad("the map repo is owner/name"));
    }
    let path = req.map_path.trim();
    if path.is_empty()
        || path.len() > 300
        || path.starts_with('/')
        || path
            .split('/')
            .any(|s| s.is_empty() || s == ".." || s == ".")
        || path.chars().any(char::is_control)
    {
        return Err(bad("the map path is a file path inside the repo"));
    }
    Ok((repo.to_owned(), path.to_owned(), clean_ref(&req.map_ref)?))
}

/// `POST /api/workspaces`, per tenant: a new workspace on a map in one of the tenant's
/// repositories. An admin's to create (ADR 0088).
pub async fn create(
    State(app): State<App>,
    AdminScope(scope): AdminScope,
    CurrentUser(user): CurrentUser,
    Json(req): Json<NewWorkspace>,
) -> Result<(StatusCode, Json<WorkspaceSettings>), Stop> {
    let refuse = |r: Refusal| r.answer(&app, WORKSPACES, &user);
    if let Some(r) = open_login(&app) {
        return Err(refuse(r));
    }
    let lock = app.slots.get::<WriteLock>();
    let _held = lock.0.lock().await;
    if !is_workspace_id(&req.id) {
        return Err(refuse(Refusal::unprocessable(
            reason::BAD_ID,
            "a workspace id is 1 to 32 of a-z, 0-9 and -",
        )));
    }
    let key = scope.ws(req.id.clone());
    let mut entries = app.workspaces_of(scope.tenant());
    if entries.iter().any(|w| w.id == req.id) {
        return Err(refuse(Refusal::conflict(
            reason::DUPLICATE,
            format!("there is a workspace {} already", req.id),
        )));
    }
    if let Some(table) = workspaces::id_has_rows(&app.db, &key)
        .await
        .map_err(failed)?
    {
        tracing::info!(rubato.workspace = %req.id, table, "create refused: kept rows");
        return Err(refuse(Refusal::conflict(
            reason::ID_HAS_ROWS,
            format!(
                "a workspace {} was deleted and its rows are kept, so the id cannot be used again; choose another id",
                req.id
            ),
        )));
    }
    let (map_repo, map_path, map_ref) = clean_source(&req).map_err(refuse)?;
    let name = clean_name(req.name.as_deref()).map_err(refuse)?;
    if let Some(other) = entries
        .iter()
        .find(|o| same_map(o, &map_repo, &map_path, &map_ref))
    {
        return Err(refuse(Refusal::conflict(
            reason::SAME_MAP,
            format!("workspace {} already shows that map at that ref", other.id),
        )));
    }
    let row = Row {
        tenant_id: scope.tenant().as_str().to_owned(),
        workspace_id: req.id.clone(),
        position: 0,
        revision: 1,
        name,
        map_repo,
        map_path,
        map_ref,
        traffic_source: None,
        traffic_fields: None,
        traffic_secrets_sealed: None,
        default_spend_cap_usd: None,
        default_attempt_limit: None,
        updated_by: user.clone(),
        updated_at: ts(app.clock.now()),
    };
    let entry = entry_of(&app, &row).map_err(failed)?;
    // The start-up checks over the list as it would be (ADR 0044), after the named refusals.
    entries.push(entry.clone());
    if let Err(e) = workspaces::check(&entries) {
        return Err(refuse(Refusal::unprocessable(
            reason::BAD_VALUE,
            e.to_string(),
        )));
    }
    let loaded = vet(&app, scope.tenant(), &user, &entry)
        .await
        .map_err(refuse)?;
    let row = store::create(&app.db, &row).await.map_err(failed)?;
    record(
        &row.key(),
        &user,
        &["created", "name", "map_repo", "map_path", "map_ref"].map(String::from),
    );
    swap(&app).await.map_err(failed)?;
    map::apply(&app, &entry, loaded).await.map_err(failed)?;
    app.notify_in(scope.tenant(), changed::WORKSPACES);
    report_again(&app).await;
    Ok((StatusCode::CREATED, Json(wire(&app, &row))))
}

/// `DELETE /api/t/{tenant}/w/{ws}`: the workspace leaves the list. Every row of it is kept and
/// counted as an orphan. Workspace `default` of tenant `default` stays: it keeps every row from
/// before workspaces. An admin's to delete (ADR 0088).
pub async fn delete(
    State(app): State<App>,
    ws: Ws,
    _admin: AdminScope,
    CurrentUser(user): CurrentUser,
) -> Result<StatusCode, Stop> {
    let refuse = |r: Refusal| r.answer(&app, routes::WORKSPACE, &user);
    if let Some(r) = open_login(&app) {
        return Err(refuse(r));
    }
    let lock = app.slots.get::<WriteLock>();
    let _held = lock.0.lock().await;
    if ws.key() == &WsKey::of_default(DEFAULT_WORKSPACE) {
        return Err(refuse(Refusal::conflict(
            reason::DEFAULT_KEPT,
            "the workspace default cannot be deleted: it keeps every row from before workspaces",
        )));
    }
    if !store::delete(&app.db, ws.key()).await.map_err(failed)? {
        return Err(StatusCode::NOT_FOUND.into());
    }
    record(ws.key(), &user, &["deleted".to_owned()]);
    swap(&app).await.map_err(failed)?;
    map::forget(&app, ws.key()).await;
    app.notify_in(ws.key().tenant(), changed::WORKSPACES);
    report_again(&app).await;
    Ok(StatusCode::NO_CONTENT)
}
