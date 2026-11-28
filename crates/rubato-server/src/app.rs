//! Composition root: the router, shared by the binary and the integration tests.
//!
//! Unknown paths under `/api`, `/ws` and `/auth` answer 404 JSON and log a `warn!`, so a route
//! lost from this list cannot look like a page (design D13). Every other unknown GET serves
//! `index.html` for the single-page app's deep links.

use crate::kernel::state::App;
use crate::{
    admin, approve, auth, companion_link, import, launch, layout, map, map_draft, new_issue,
    overview, pairing, pins, plan_drafts, pulls, queue, rooms, session_log, session_messages,
    telemetry_relay, tenants, traffic, voice, web_link, webhooks, workspace_settings,
};
use axum::Router;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post, put};
use rubato_protocol::approve::routes as approve_routes;
use rubato_protocol::auth::routes as auth_routes;
use rubato_protocol::issue_new::routes as issue_routes;
use rubato_protocol::launch::routes as launch_routes;
use rubato_protocol::layout::routes as layout_routes;
use rubato_protocol::map::routes as map_routes;
use rubato_protocol::map_draft::routes as draft_routes;
use rubato_protocol::pairing::routes as pr;
use rubato_protocol::pins::routes as pin_routes;
use rubato_protocol::plan_draft::routes as plan_draft_routes;
use rubato_protocol::progress::routes as overview_routes;
use rubato_protocol::pulls::routes as pull_routes;
use rubato_protocol::queue::routes as queue_routes;
use rubato_protocol::tenant::routes as tenant_routes_of;
use rubato_protocol::tenant::routes as tenant_routes;
use rubato_protocol::traffic::routes as traffic_routes;
use rubato_protocol::web_link::routes as web_routes;
use rubato_protocol::workspace::routes as workspace_routes;
use rubato_protocol::workspace_settings::routes as settings_routes;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

/// The body of the 404 answer for an unknown API path.
pub const UNKNOWN_API_PATH: &str = r#"{"error":"unknown api path"}"#;

/// One tenant route (ADR 0089): the path as the protocol names it, whether it is map-scoped (and
/// so mounted under a workspace), and its handler for each method. The router is built from
/// this table and from nothing else, and the cross-tenant sweep reads the same table
/// ([`tenant_route_table`]), so a tenant route cannot be added without being swept (design §11.2).
struct TenantRoute {
    path: String,
    map_scoped: bool,
    on: Vec<(Method, axum::routing::MethodRouter<App>)>,
}

impl TenantRoute {
    /// Where the route is mounted: under the tenant, and under its workspace when map-scoped.
    fn mounted(&self) -> String {
        // Already a full tenant path (`/api/t/{tenant}/…`); `/api/traffic/…` is not one.
        if self
            .path
            .starts_with(&format!("{}/", tenant_routes::TENANT_PREFIX))
        {
            self.path.clone()
        } else if self.map_scoped {
            workspace_routes::scoped("{tenant}", "{ws}", &self.path)
        } else {
            tenant_routes::in_tenant("{tenant}", &self.path)
        }
    }
}

fn tenant_routes() -> Vec<TenantRoute> {
    let ws = |path: &str, on: Vec<(Method, axum::routing::MethodRouter<App>)>| TenantRoute {
        path: path.to_owned(),
        map_scoped: true,
        on,
    };
    let t = |path: &str, on: Vec<(Method, axum::routing::MethodRouter<App>)>| TenantRoute {
        path: path.to_owned(),
        map_scoped: false,
        on,
    };
    use Method as M;
    use rubato_protocol::plan_draft::routes as drafts;
    use rubato_protocol::room::routes as room;
    use rubato_protocol::session_log::routes as log;
    use rubato_protocol::session_message::routes as messages;
    use rubato_protocol::voice::routes as speech;
    vec![
        // Map-scoped (ADR 0049): each under `/api/t/{tenant}/w/{ws}`.
        ws(
            overview_routes::OVERVIEW,
            vec![(M::GET, get(overview::overview))],
        ),
        ws(map_routes::MAP, vec![(M::GET, get(map::map))]),
        ws(
            map_routes::MAP_RELOAD,
            vec![(M::POST, post(map::map_reload))],
        ),
        ws(
            draft_routes::DRAFT_OPS,
            vec![(M::POST, post(map_draft::add_op))],
        ),
        ws(
            &format!("{}/{{seq}}", draft_routes::DRAFT_OPS),
            vec![(M::DELETE, delete(map_draft::remove_op))],
        ),
        ws(
            draft_routes::DRAFT_DISCARD,
            vec![(M::POST, post(map_draft::discard))],
        ),
        ws(
            draft_routes::DRAFT_RESTORE,
            vec![(M::POST, post(map_draft::restore))],
        ),
        ws(
            draft_routes::DRAFT_YAML,
            vec![(M::GET, get(map_draft::proposed_yaml))],
        ),
        ws(
            draft_routes::DRAFT_PROPOSE,
            vec![(M::POST, post(map_draft::propose))],
        ),
        ws(
            draft_routes::DRAFT_WITHDRAW,
            vec![(M::POST, post(map_draft::withdraw))],
        ),
        ws(
            pin_routes::PINS,
            vec![
                (M::POST, post(pins::add_pin)),
                (M::DELETE, delete(pins::remove_pin)),
            ],
        ),
        ws(launch_routes::LAUNCH, vec![(M::POST, post(launch::launch))]),
        ws(
            plan_draft_routes::PLAN_IMPORT,
            vec![(M::POST, post(import::import))],
        ),
        ws(
            issue_routes::ISSUES,
            vec![(M::POST, post(new_issue::create))],
        ),
        ws(
            approve_routes::PLAN_APPROVE,
            vec![(M::POST, post(approve::approve))],
        ),
        ws(
            approve_routes::PLAN_DRAFT_APPROVE,
            vec![(M::POST, post(approve::approve_draft))],
        ),
        ws(
            layout_routes::LAYOUT,
            vec![
                (M::PUT, put(layout::put_layout)),
                (M::DELETE, delete(layout::clear_layout)),
            ],
        ),
        ws(
            layout_routes::LAYOUT_GROUPED,
            vec![(M::DELETE, delete(layout::clear_grouped_layout))],
        ),
        ws(
            traffic_routes::TRAFFIC_DISMISSALS,
            vec![(M::POST, post(traffic::dismiss))],
        ),
        ws(
            &format!("{}/{{id}}", traffic_routes::TRAFFIC_DISMISSALS),
            vec![(M::DELETE, delete(traffic::undismiss))],
        ),
        ws(pull_routes::PULL, vec![(M::GET, get(pulls::pull_view))]),
        ws(
            pull_routes::PULLS_MERGE,
            vec![(M::POST, post(pulls::merge))],
        ),
        ws(
            settings_routes::SETTINGS,
            vec![
                (M::GET, get(workspace_settings::settings)),
                (M::PUT, put(workspace_settings::update)),
            ],
        ),
        // ADR 0075: a workspace is deleted by its own id. Its path is already the full one.
        ws(
            settings_routes::WORKSPACE,
            vec![(M::DELETE, delete(workspace_settings::delete))],
        ),
        // Tenant routes that name no workspace: each under `/api/t/{tenant}`.
        t(
            workspace_routes::WORKSPACES,
            vec![
                (M::GET, get(map::workspaces)),
                (M::POST, post(workspace_settings::create)),
            ],
        ),
        t(
            settings_routes::TRAFFIC_SOURCES,
            vec![(M::GET, get(workspace_settings::sources))],
        ),
        // A tenant's members, managed by its admins (ADR 0088): `tenant::routes::MEMBERS` and
        // `MEMBER`, written here without their tenant prefix like every other row.
        t(
            "/api/members",
            vec![
                (M::GET, get(admin::members::list)),
                (M::POST, post(admin::members::invite)),
            ],
        ),
        t(
            "/api/members/{github_id}",
            vec![
                (M::PUT, put(admin::members::set_role)),
                (M::DELETE, delete(admin::members::remove)),
            ],
        ),
        t(
            queue_routes::QUEUE,
            vec![
                (M::POST, post(queue::add)),
                (M::PUT, put(queue::reorder)),
                (M::DELETE, delete(queue::remove)),
            ],
        ),
        t(
            messages::LAUNCH_MESSAGES,
            vec![
                (M::GET, get(session_messages::list)),
                (M::POST, post(session_messages::send)),
            ],
        ),
        t(
            messages::LAUNCH_STOP,
            vec![(M::POST, post(session_messages::stop))],
        ),
        t(log::LAUNCH_LOG, vec![(M::GET, get(session_log::log))]),
        t(drafts::DRAFT, vec![(M::GET, get(plan_drafts::draft))]),
        t(
            drafts::DRAFT_REVISION,
            vec![(M::GET, get(plan_drafts::revision))],
        ),
        t(room::ROOM, vec![(M::GET, get(rooms::room))]),
        t(
            room::ROOM_AGENTS,
            vec![
                (M::POST, post(rooms::call_in)),
                (M::DELETE, delete(rooms::dismiss)),
            ],
        ),
        t(room::ROOM_REQUEST, vec![(M::POST, post(rooms::answer))]),
        t(room::ROOM_MESSAGES, vec![(M::POST, post(rooms::post))]),
        // ADR 0073: a plan room's voice channel. Every route answers 404 `voice_off` while
        // `RUBATO_VOICE` is off; the handlers cap their own bodies.
        t(
            speech::VOICE_TOKEN,
            vec![(M::POST, post(voice::voice_token))],
        ),
        t(
            speech::VOICE_UTTERANCE,
            vec![(M::POST, post(voice::utterance))],
        ),
        t(speech::VOICE_SPEECH, vec![(M::GET, get(voice::speech))]),
        t(
            speech::VOICE_JOIN_FAILED,
            vec![(M::POST, post(voice::join_failed))],
        ),
    ]
}

/// Every tenant route as it is mounted: its method and its path template (`{tenant}`, `{ws}`
/// and its own ids as placeholders). What the cross-tenant sweep calls (design §11.2).
#[doc(hidden)]
pub fn tenant_route_table() -> Vec<(Method, String)> {
    tenant_routes()
        .iter()
        .flat_map(|r| {
            let at = r.mounted();
            r.on.iter().map(move |(m, _)| (m.clone(), at.clone()))
        })
        .collect()
}

pub fn router(app: App) -> Router {
    let dist = app.config.web_dist.clone();
    let index = dist.join("index.html");
    // Built from the one table (design §11.2). A map-scoped route gets the workspace layer; all
    // get the membership layer, which runs first (ADR 0089).
    let (mut scoped, mut plain) = (Router::new(), Router::new());
    for r in tenant_routes() {
        let at = r.mounted();
        let on =
            r.on.into_iter()
                .fold(axum::routing::MethodRouter::new(), |all, (_, one)| {
                    all.merge(one)
                });
        if r.map_scoped {
            scoped = scoped.route(&at, on);
        } else {
            plain = plain.route(&at, on);
        }
    }
    let scoped = scoped.route_layer(axum::middleware::from_fn_with_state(
        app.clone(),
        crate::kernel::http::check_workspace,
    ));
    // Outermost first: the membership layer, then the repositories the request names.
    let tenant = plain
        .merge(scoped)
        .route_layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::kernel::http::check_repos,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            app.clone(),
            tenants::check_tenant,
        ));
    let api = Router::new()
        .route("/api/health", get(health))
        .route(auth_routes::ME, get(auth::me))
        .merge(tenant)
        .route(pr::PAIRINGS, get(pairing::list))
        .route(
            &format!("{}/{{companion_id}}", pr::PAIRINGS),
            delete(pairing::revoke),
        )
        // The operator's routes (ADR 0087, 0088): 404 for anyone else.
        .route(
            tenant_routes_of::ADMIN_INSTALLATIONS,
            get(admin::operator::installations),
        )
        .route(
            tenant_routes_of::ADMIN_TENANTS,
            get(admin::operator::tenants_list).post(admin::operator::create_tenant),
        )
        .route(
            tenant_routes_of::ADMIN_TENANT,
            delete(admin::operator::delete_tenant),
        )
        .route(
            tenant_routes_of::ADMIN_ATTACH,
            post(admin::operator::attach).delete(admin::operator::detach),
        )
        .route(tenant_routes_of::ADMIN_PURGE, post(admin::operator::purge))
        .route(
            tenant_routes_of::ADMIN_TENANT_ADMINS,
            post(admin::operator::name_admin),
        )
        .route(pr::PAIR_APPROVE, post(pairing::approve))
        .route(pr::PAIR_START, post(pairing::start))
        .route(pr::PAIR_POLL, post(pairing::poll))
        .route(pr::COMPANION_WS, get(companion_link::upgrade))
        // One context graph source for a companion, by its pairing token (ADR 0086).
        .route(
            rubato_protocol::graph::routes::GRAPH_SNAPSHOT,
            get(crate::context_graph::snapshot),
        )
        .route(web_routes::WEB_WS, get(web_link::upgrade))
        .route(auth_routes::LOGIN, get(auth::login))
        .route(auth_routes::CALLBACK, get(auth::callback))
        .route(auth_routes::LOGOUT, post(auth::logout))
        // ADR 0082: the web's own client spans, rewritten and forwarded; 404 while the server
        // exports no traces. The body cap is the relay's own.
        .route(
            rubato_protocol::telemetry_relay::routes::TELEMETRY_TRACES,
            post(telemetry_relay::relay).layer(axum::extract::DefaultBodyLimit::max(
                rubato_protocol::telemetry_relay::MAX_BODY_BYTES,
            )),
        )
        .route("/api/{*rest}", any(unknown_api_path))
        .route("/ws/{*rest}", any(unknown_api_path))
        .route("/auth/{*rest}", any(unknown_api_path))
        // Every write and websocket upgrade, tenant routes included (ADR 0066).
        .route_layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::kernel::http::same_origin,
        ));
    // After the same-origin layer on purpose (ADR 0074): GitHub is not a browser, and the
    // signature is this route's authentication. Without the secret the path is unknown.
    let api = if app.config.webhooks_on() {
        api.route(
            webhooks::PATH,
            post(webhooks::receive).layer(axum::extract::DefaultBodyLimit::max(
                webhooks::MAX_BODY_BYTES,
            )),
        )
    } else {
        api.route(webhooks::PATH, any(unknown_api_path))
    }
    .route("/webhooks/{*rest}", any(unknown_api_path));
    let csp = axum::http::HeaderValue::from_str(&crate::kernel::http::content_security_policy(
        &app.config.public_url,
        app.config.livekit_url(),
    ))
    .expect("a header-safe policy");
    api.fallback_service(ServeDir::new(dist).not_found_service(ServeFile::new(index)))
        .layer(axum::middleware::map_response(move |mut res: Response| {
            let csp = csp.clone();
            async move {
                crate::kernel::http::add_security_headers(&csp, &mut res);
                res
            }
        }))
        .layer({
            // ADR 0082: a request with the browser's `traceparent` is a server span under it.
            let span_app = app.clone();
            TraceLayer::new_for_http()
                .make_span_with(move |req: &axum::http::Request<axum::body::Body>| {
                    crate::kernel::http::request_span(&span_app, req)
                })
                .on_response(
                    |res: &Response, _latency: std::time::Duration, span: &tracing::Span| {
                        crate::kernel::http::record_status(res, span)
                    },
                )
        })
        .with_state(app)
}

/// For this crate's tests that were written before tenants (ADR 0089), and for nothing else:
/// [`router`], with a request to a path of before the cut sent on to the same route in tenant
/// `default` (and workspace `default` when the path named none). The binary never calls it, so a
/// real server answers 404 for those paths; `tests/tenant_paths.rs` checks that against
/// [`router`] itself.
#[doc(hidden)]
pub fn router_before_tenants(app: App) -> Router {
    async fn moved(mut req: axum::extract::Request) -> axum::extract::Request {
        if let Some(path) = path_in_default_tenant(req.uri().path()) {
            let query = req
                .uri()
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default();
            if let Ok(uri) = format!("{path}{query}").parse::<Uri>() {
                *req.uri_mut() = uri;
            }
        }
        req
    }
    // The layer runs before the outer router hands the request to its fallback, which is the
    // real router: it routes on the path as rewritten.
    Router::new()
        .fallback_service(router(app))
        .layer(axum::middleware::map_request(moved))
}

/// Where a path of before ADR 0089 lives now, in tenant `default`; `None` for a path that did
/// not move (a person or server route, or one already under `/api/t`).
#[doc(hidden)]
pub fn path_in_default_tenant(path: &str) -> Option<String> {
    const PERSON: [&str; 7] = [
        "/api/me",
        "/api/health",
        "/api/pair",
        "/api/graph",
        "/api/telemetry",
        "/api/admin",
        "/api/t/",
    ];
    const TENANT: [&str; 6] = [
        "/api/workspaces",
        "/api/traffic/sources",
        "/api/queue",
        "/api/launches/",
        "/api/drafts/",
        "/api/room",
    ];
    let t = crate::kernel::tenancy::DEFAULT_TENANT;
    let rest = path.strip_prefix("/api/")?;
    if PERSON.iter().any(|p| path.starts_with(p)) {
        return None;
    }
    if let Some(scoped) = rest.strip_prefix("w/") {
        return Some(format!("/api/t/{t}/w/{scoped}"));
    }
    if TENANT.iter().any(|p| path.starts_with(p)) {
        return Some(tenant_routes::in_tenant(t, path));
    }
    Some(workspace_routes::scoped(
        t,
        rubato_protocol::DEFAULT_WORKSPACE,
        path,
    ))
}

async fn health() -> &'static str {
    "ok"
}

async fn unknown_api_path(method: Method, uri: Uri) -> Response {
    tracing::warn!(%method, path = %uri.path(), "unknown api path");
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        UNKNOWN_API_PATH,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use std::sync::Arc;

    async fn serve(web_dist: Option<std::path::PathBuf>) -> (String, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let mut cfg = Config::for_tests(url, "m0x6A");
        if let Some(d) = web_dist {
            cfg.web_dist = d;
        }
        let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router_before_tenants(app))
                .await
                .unwrap()
        });
        (format!("http://{addr}"), tmp)
    }

    /// ADR 0082: a request carrying the browser's `traceparent` is `rubato.http.request` of
    /// kind server, in the browser's trace and under its span, with the route and status. One
    /// without, or with one that does not parse, makes no such span. Each is counted. Fails on
    /// purpose when `set_parent` is taken out: the span then starts its own trace.
    #[tokio::test(flavor = "current_thread")]
    async fn a_request_with_the_browsers_traceparent_is_a_server_span_under_it() {
        use crate::kernel::telemetry::metric_reader;
        use opentelemetry::trace::TracerProvider as _;
        use rubato_protocol::otel::{attr, metric, span};
        use tracing_subscriber::layer::SubscriberExt as _;
        drop(crate::kernel::telemetry::capture::start());
        let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry()
                .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test"))),
        );
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            Arc::new(FakeGitHub::new()),
        );
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
        let http = reqwest::Client::new();
        let health = format!("http://{addr}/api/health");
        const SENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        for tp in [Some(SENT), None, Some("00-not-a-trace-01")] {
            let mut req = http.get(&health);
            if let Some(tp) = tp {
                req = req.header("traceparent", tp);
            }
            assert_eq!(req.send().await.unwrap().status(), 200);
        }
        let mut spans = Vec::new();
        for _ in 0..200 {
            spans = exporter
                .get_finished_spans()
                .unwrap()
                .into_iter()
                .filter(|s| s.name == span::HTTP_REQUEST)
                .collect();
            if !spans.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(
            spans.len(),
            1,
            "only the request with a valid traceparent: {spans:?}"
        );
        let s = &spans[0];
        assert_eq!(s.span_kind, opentelemetry::trace::SpanKind::Server);
        assert_eq!(
            format!("{:032x}", s.span_context.trace_id()),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(format!("{:016x}", s.parent_span_id), "00f067aa0ba902b7");
        let attr_of = |k: &str| {
            s.attributes
                .iter()
                .find(|kv| kv.key.as_str() == k)
                .map(|kv| kv.value.to_string())
        };
        assert_eq!(attr_of("http.route").as_deref(), Some("/api/health"));
        assert_eq!(attr_of("http.response.status_code").as_deref(), Some("200"));
        let count = |outcome: &str| -> u64 {
            reader
                .points(metric::HTTP_TRACEPARENT)
                .iter()
                .filter(|p| p.attrs[attr::OUTCOME] == outcome)
                .map(|p| p.value)
                .sum()
        };
        assert_eq!(
            (count("linked"), count("absent"), count("invalid")),
            (1, 1, 1)
        );
    }

    /// Every route the server registers, with its method. Kept by hand: the probe fails when a
    /// line above goes missing, because the path then reaches the 404 fallback.
    fn known_routes() -> Vec<(Method, String)> {
        let revoke = format!("{}/{}", pr::PAIRINGS, uuid::Uuid::new_v4());
        let mut routes = vec![
            (Method::GET, "/api/health".into()),
            (Method::GET, auth_routes::ME.into()),
            (Method::GET, overview_routes::OVERVIEW.into()),
            (Method::GET, map_routes::MAP.into()),
            (Method::POST, map_routes::MAP_RELOAD.into()),
            (Method::POST, draft_routes::DRAFT_OPS.into()),
            (Method::DELETE, format!("{}/1", draft_routes::DRAFT_OPS)),
            (Method::POST, draft_routes::DRAFT_DISCARD.into()),
            (Method::POST, draft_routes::DRAFT_RESTORE.into()),
            (Method::GET, draft_routes::DRAFT_YAML.into()),
            (Method::POST, draft_routes::DRAFT_PROPOSE.into()),
            (Method::POST, draft_routes::DRAFT_WITHDRAW.into()),
            (Method::POST, pin_routes::PINS.into()),
            (Method::DELETE, pin_routes::PINS.into()),
            (Method::POST, launch_routes::LAUNCH.into()),
            (Method::POST, plan_draft_routes::PLAN_IMPORT.into()),
            (Method::POST, issue_routes::ISSUES.into()),
            (Method::POST, approve_routes::PLAN_APPROVE.into()),
            (Method::POST, approve_routes::PLAN_DRAFT_APPROVE.into()),
            (Method::PUT, layout_routes::LAYOUT.into()),
            (Method::DELETE, layout_routes::LAYOUT.into()),
            (Method::DELETE, layout_routes::LAYOUT_GROUPED.into()),
            (Method::POST, pull_routes::PULLS_MERGE.into()),
            (
                Method::GET,
                pull_routes::PULL
                    .replace("{owner}", "m0x6A")
                    .replace("{repo}", "rubato")
                    .replace("{number}", "1"),
            ),
            (Method::POST, traffic_routes::TRAFFIC_DISMISSALS.into()),
            (
                Method::POST,
                rubato_protocol::telemetry_relay::routes::TELEMETRY_TRACES.into(),
            ),
            (
                Method::DELETE,
                format!("{}/1", traffic_routes::TRAFFIC_DISMISSALS),
            ),
            (Method::POST, queue_routes::QUEUE.into()),
            (Method::PUT, queue_routes::QUEUE.into()),
            (Method::DELETE, queue_routes::QUEUE.into()),
            (Method::GET, pr::PAIRINGS.into()),
            (Method::DELETE, revoke),
            (Method::POST, pr::PAIR_APPROVE.into()),
            (Method::POST, pr::PAIR_START.into()),
            (Method::POST, pr::PAIR_POLL.into()),
            (
                Method::GET,
                rubato_protocol::graph::routes::GRAPH_SNAPSHOT.into(),
            ),
            (Method::GET, pr::COMPANION_WS.into()),
            (Method::GET, web_routes::WEB_WS.into()),
            (Method::GET, auth_routes::LOGIN.into()),
            (Method::GET, auth_routes::CALLBACK.into()),
            (Method::POST, auth_routes::LOGOUT.into()),
            (Method::GET, workspace_routes::WORKSPACES.into()),
            (Method::POST, workspace_routes::WORKSPACES.into()),
            (Method::GET, settings_routes::SETTINGS.into()),
            (Method::PUT, settings_routes::SETTINGS.into()),
            (Method::GET, settings_routes::TRAFFIC_SOURCES.into()),
            (
                Method::DELETE,
                settings_routes::WORKSPACE.replace("{ws}", "default"),
            ),
            (
                Method::GET,
                format!(
                    "{}?repo=m0x6A/rubato&number=1&branch=issue/1-x",
                    rubato_protocol::room::routes::ROOM
                ),
            ),
            (
                Method::POST,
                rubato_protocol::room::routes::ROOM_MESSAGES.into(),
            ),
            (
                Method::POST,
                rubato_protocol::room::routes::ROOM_AGENTS.into(),
            ),
            (
                Method::DELETE,
                rubato_protocol::room::routes::ROOM_AGENTS.into(),
            ),
            (
                Method::POST,
                rubato_protocol::room::routes::ROOM_REQUEST
                    .replace("{request_id}", &uuid::Uuid::new_v4().to_string()),
            ),
        ];
        // Every map-scoped route again under `/api/w/{ws}` for a configured workspace (ADR 0049).
        let scoped: Vec<(Method, String)> = routes
            .iter()
            // The adapters' field lists are team-wide, though under `/api/traffic`.
            .filter(|(_, p)| p != settings_routes::TRAFFIC_SOURCES)
            .filter(|(_, p)| MAP_SCOPED.iter().any(|m| p.starts_with(m)))
            .map(|(m, p)| (m.clone(), workspace_routes::scoped("default", "default", p)))
            .collect();
        assert!(scoped.len() >= 20, "{scoped:?}");
        routes.extend(scoped);
        routes
    }

    /// The paths served per workspace: map-scoped, never team-scoped.
    const MAP_SCOPED: [&str; 12] = [
        "/api/settings",
        "/api/pulls",
        "/api/traffic",
        "/api/overview",
        "/api/map",
        "/api/pins",
        "/api/launch",
        "/api/layout",
        "/api/issues",
        "/api/plans/approve",
        "/api/plans/drafts/approve",
        "/api/plans/import",
    ];

    /// Refusals are counted by route and reason, and logged with who (design O3, O12).
    #[tokio::test(flavor = "current_thread")]
    async fn refusals_are_counted_by_route_and_reason() {
        use crate::kernel::config::GitHubApp;
        use crate::kernel::telemetry::{capture, metric_reader};
        use rubato_protocol::otel::{attr, metric};
        use rubato_protocol::{IssueRef, MapRef, PinRequest, SystemMap};

        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            Arc::new(FakeGitHub::new()),
        );
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let map: SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../docs/design/rubato-map.yml")).unwrap();
        app.set_map(
            &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
            crate::kernel::state::LoadedMap {
                map,
                version: "v".into(),
                loaded_at: chrono::Utc::now(),
                yaml: String::new(),
            },
        )
        .await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = app.clone();
        tokio::spawn(async move {
            axum::serve(listener, router_before_tenants(served))
                .await
                .unwrap()
        });
        let http = reqwest::Client::new();
        let (events, _guard) = capture::start();
        let count = |reason: &str| {
            reader
                .points(metric::WEB_REJECTED)
                .into_iter()
                .find(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason))
                .map(|p| {
                    (
                        p.value,
                        p.attrs.get(attr::ROUTE).cloned().unwrap_or_default(),
                    )
                })
        };

        // A part's id pinned as a seam, then an id the map does not have.
        for (target, _) in [
            (MapRef::seam("rubato-server"), "kind_mismatch"),
            (MapRef::node("nope"), "unknown_id"),
        ] {
            let r = http
                .post(format!("{base}{}", pin_routes::PINS))
                .json(&PinRequest {
                    issue: IssueRef::new("m0x6A/rubato", 48),
                    target,
                })
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 422);
        }
        assert_eq!(
            count("kind_mismatch"),
            Some((1, pin_routes::PINS.to_owned()))
        );
        assert_eq!(count("unknown_id"), Some((1, pin_routes::PINS.to_owned())));
        // Issue #163: an issue of a repo the map does not name is refused at the pin and at
        // the launch, each counted on its own route.
        let elsewhere = IssueRef::new("elsewhere/secret", 1);
        let r = http
            .post(format!("{base}{}", pin_routes::PINS))
            .json(&PinRequest {
                issue: elsewhere.clone(),
                target: MapRef::node("web"),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422);
        assert!(
            r.text()
                .await
                .unwrap()
                .contains("elsewhere/secret is not a repo the map names")
        );
        let r = http
            .post(format!("{base}{}", launch_routes::LAUNCH))
            .json(&serde_json::json!({ "issue": elsewhere }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422);
        let unknown_repo = |route: &str| {
            reader
                .points(metric::WEB_REJECTED)
                .into_iter()
                .find(|p| {
                    p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("unknown_repo")
                        && p.attrs.get(attr::ROUTE).map(String::as_str) == Some(route)
                })
                .map(|p| p.value)
        };
        assert_eq!(unknown_repo(pin_routes::PINS), Some(1));
        assert_eq!(unknown_repo(launch_routes::LAUNCH), Some(1));
        {
            let events = events.lock().unwrap();
            let line = events
                .iter()
                .find(|e| {
                    e.message == "web write rejected"
                        && e.fields.get("rubato.reason").map(String::as_str)
                            == Some("kind_mismatch")
                })
                .expect("an info line per refusal");
            assert_eq!(line.level, tracing::Level::INFO);
            assert_eq!(
                line.fields.get("rubato.user").map(String::as_str),
                Some("m0x6A")
            );
            assert_eq!(
                line.fields.get("rubato.route").map(String::as_str),
                Some(pin_routes::PINS)
            );
        }

        // The layout: a part the map lacks and a position off the canvas are refused and
        // counted; an accepted write and a clear each move `rubato.layout.writes`.
        use rubato_protocol::layout::{MapLayout, Position};
        let layout = |id: &str, x: f64| MapLayout {
            positions: [(id.to_owned(), Position { x, y: 0.0 })].into(),
            ..Default::default()
        };
        let put = |l: MapLayout| {
            http.put(format!("{base}{}", layout_routes::LAYOUT))
                .json(&l)
                .send()
        };
        assert_eq!(put(layout("nope", 1.0)).await.unwrap().status(), 422);
        assert_eq!(put(layout("web", 1.0e9)).await.unwrap().status(), 422);
        assert!(
            reader.points(metric::LAYOUT_WRITES).is_empty(),
            "a refused write is not a write"
        );
        let by_route = |reason: &str| {
            reader
                .points(metric::WEB_REJECTED)
                .into_iter()
                .find(|p| {
                    p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason)
                        && p.attrs.get(attr::ROUTE).map(String::as_str)
                            == Some(layout_routes::LAYOUT)
                })
                .map(|p| p.value)
        };
        assert_eq!(by_route("unknown_id"), Some(1));
        assert_eq!(by_route("bad_position"), Some(1));
        assert_eq!(put(layout("web", 10.0)).await.unwrap().status(), 204);
        let cleared = http
            .delete(format!("{base}{}", layout_routes::LAYOUT))
            .send()
            .await
            .unwrap();
        assert_eq!(cleared.status(), 200);
        let writes = |which: &str| {
            reader
                .points(metric::LAYOUT_WRITES)
                .into_iter()
                .find(|p| p.attrs.get(attr::LAYOUT_ARRANGEMENT).map(String::as_str) == Some(which))
                .map(|p| p.value)
        };
        assert_eq!(writes("flat"), Some(2));
        // The By repo grouping (ADR 0068) counts apart from the flat map.
        let grouped = MapLayout {
            grouped: [("web".to_owned(), Position { x: 30.0, y: 60.0 })].into(),
            boxes: [("m0x6A/rubato".to_owned(), Position { x: 0.0, y: 0.0 })].into(),
            ..Default::default()
        };
        assert_eq!(put(grouped).await.unwrap().status(), 204);
        let cleared = http
            .delete(format!("{base}{}", layout_routes::LAYOUT_GROUPED))
            .send()
            .await
            .unwrap();
        assert_eq!(cleared.status(), 200);
        assert_eq!(writes("grouped"), Some(2));
        assert_eq!(writes("flat"), Some(2), "a grouped write is not a flat one");
        // One body with both counts once in each arrangement.
        let both = MapLayout {
            grouped: [("web".to_owned(), Position { x: 1.0, y: 1.0 })].into(),
            ..layout("web", 20.0)
        };
        assert_eq!(put(both).await.unwrap().status(), 204);
        assert_eq!((writes("flat"), writes("grouped")), (Some(3), Some(3)));
        {
            let events = events.lock().unwrap();
            assert!(
                events.iter().any(|e| e.message == "layout written"
                    && e.level == tracing::Level::INFO
                    && e.fields
                        .get("rubato.layout.arrangement")
                        .map(String::as_str)
                        == Some("grouped")),
                "a write is logged at info with its arrangement"
            );
        }
        {
            let events = events.lock().unwrap();
            assert!(
                events.iter().any(|e| e.message == "layout cleared"
                    && e.level == tracing::Level::INFO
                    && e.fields.get("rubato.user").map(String::as_str) == Some("m0x6A")),
                "clearing the team's layout is logged with who did it; max level {:?}, captured: {:?}",
                tracing::level_filters::LevelFilter::current(),
                events
                    .iter()
                    .map(|e| format!("{} {} {:?}", e.level, e.message, e.fields))
                    .collect::<Vec<_>>()
            );
        }

        // The map draft: an operation the rules refuse is counted with its reason.
        let bad_op = serde_json::json!({ "op": "add_node", "op_id": uuid::Uuid::new_v4(), "node": { "id": "web", "kind": "context" } });
        let r = http
            .post(format!("{base}{}", draft_routes::DRAFT_OPS))
            .json(&bad_op)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422);
        assert!(r.text().await.unwrap().contains("already exists"));
        let refused = reader
            .points(metric::WEB_REJECTED)
            .into_iter()
            .find(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("invalid_op"))
            .expect("counted");
        assert_eq!(refused.attrs[attr::ROUTE], draft_routes::DRAFT_OPS);
        // Proposing through a connection that cannot write is refused and counted too.
        let ok_op = serde_json::json!({ "op": "add_node", "op_id": uuid::Uuid::new_v4(), "node": { "id": "billing", "kind": "context" } });
        assert_eq!(
            http.post(format!("{base}{}", draft_routes::DRAFT_OPS))
                .json(&ok_op)
                .send()
                .await
                .unwrap()
                .status(),
            201
        );
        assert_eq!(
            http.post(format!("{base}{}", draft_routes::DRAFT_PROPOSE))
                .send()
                .await
                .unwrap()
                .status(),
            409
        );
        assert!(reader.points(metric::WEB_REJECTED).iter().any(|p| {
            p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("cannot_write")
                && p.attrs[attr::ROUTE] == draft_routes::DRAFT_PROPOSE
        }));

        // The queue: a lane that is not the caller's (or does not exist) refuses all three
        // writes, each counted.
        use rubato_protocol::queue::{QueueOrder, QueueRequest};
        let nobody = uuid::Uuid::new_v4();
        let entry = QueueRequest {
            issue: IssueRef::new("m0x6A/rubato", 48),
            companion_id: nobody,
        };
        let url = format!("{base}{}", queue_routes::QUEUE);
        assert_eq!(
            http.post(&url).json(&entry).send().await.unwrap().status(),
            403
        );
        assert_eq!(
            http.delete(&url)
                .json(&entry)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        let order = QueueOrder {
            companion_id: nobody,
            issues: vec![],
        };
        assert_eq!(
            http.put(&url).json(&order).send().await.unwrap().status(),
            403
        );
        let refused = reader
            .points(metric::WEB_REJECTED)
            .into_iter()
            .find(|p| {
                p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("not_own_companion")
            })
            .expect("counted");
        assert_eq!(refused.value, 3);
        assert_eq!(refused.attrs[attr::ROUTE], queue_routes::QUEUE);

        // Pairing (#160): approving a code for a companion another login owns is refused.
        use rubato_protocol::pairing::{PAIR_FIELD_MAX_CHARS, PairStartRequest, PairStartResponse};
        let owned = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at) VALUES (?, 'bob', 'h', 'h', '2026-11-01T00:00:00Z')")
            .bind(owned.to_string())
            .execute(&app.db)
            .await
            .unwrap();
        let start: PairStartResponse = http
            .post(format!("{base}{}", pr::PAIR_START))
            .json(&PairStartRequest {
                companion_id: owned,
                hostname: "h".into(),
                companion_version: "t".into(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let r = http
            .post(format!("{base}{}", pr::PAIR_APPROVE))
            .json(&rubato_protocol::PairApproveRequest {
                user_code: start.user_code,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert_eq!(
            count("companion_taken"),
            Some((1, pr::PAIR_APPROVE.to_owned()))
        );

        // Pairing starts (#168): a field too long, then the pending cap.
        let start = |hostname: String| PairStartRequest {
            companion_id: uuid::Uuid::new_v4(),
            hostname,
            companion_version: "t".into(),
        };
        let r = http
            .post(format!("{base}{}", pr::PAIR_START))
            .json(&start("h".repeat(PAIR_FIELD_MAX_CHARS + 1)))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422);
        assert_eq!(
            count("field_too_long"),
            Some((1, pr::PAIR_START.to_owned()))
        );
        for _ in 0..app.config.pairing_max_pending {
            sqlx::query("INSERT INTO pairing_codes (device_code, user_code, companion_id, hostname, companion_version, status, created_at, expires_at) VALUES (?, ?, 'x', 'h', 't', 'pending', '2026-01-01T00:00:00+00:00', '2999-01-01T00:00:00+00:00')")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(uuid::Uuid::new_v4().to_string())
                .execute(&app.db)
                .await
                .unwrap();
        }
        let r = http
            .post(format!("{base}{}", pr::PAIR_START))
            .json(&start("h".into()))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 429);
        assert_eq!(count("pending_cap"), Some((1, pr::PAIR_START.to_owned())));

        // ADR 0088: a session whose person is in no tenant reaches no tenant route. The answer
        // is the one for a tenant that does not exist, counted by the route's template.
        let tmp2 = tempfile::tempdir().unwrap();
        let url2 = format!("sqlite://{}/t.db?mode=rwc", tmp2.path().display());
        let pool2 = crate::kernel::db::connect(&url2).await.unwrap();
        let mut cfg = Config::for_tests(url2, "unused");
        cfg.dev_user = None;
        cfg.github_app = Some(GitHubApp {
            app_id: 1,
            private_key: "unused".into(),
            installation_id: 2,
        });
        let (reader2, metrics2) = metric_reader::start();
        let mut gated = AppState::new(pool2, cfg, Arc::new(FakeGitHub::new()));
        Arc::get_mut(&mut gated).unwrap().metrics = metrics2;
        sqlx::query("INSERT INTO web_sessions (id, login, created_at) VALUES ('s1', 'old-timer', '2026-11-01T00:00:00Z')")
            .execute(&gated.db)
            .await
            .unwrap();
        let cookie = format!("rubato_session={}", gated.signer.sign("s1"));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base2 = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(gated)).await.unwrap() });
        let r = http
            .get(format!(
                "{base2}{}",
                workspace_routes::scoped("default", "default", overview_routes::OVERVIEW)
            ))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert_eq!(
            r.text().await.unwrap(),
            r#"{"reason":"unknown_tenant"}"#,
            "the same answer as for a tenant that does not exist"
        );
        let stale = reader2.points(metric::WEB_REJECTED);
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert_eq!(stale[0].attrs[attr::DROP_REASON], "unknown_tenant");
        assert_eq!(
            stale[0].attrs[attr::ROUTE],
            workspace_routes::scoped("{tenant}", "{ws}", overview_routes::OVERVIEW),
            "the route's template: a tenant id is never a label"
        );
        println!("checked 11 refusal reasons");
    }

    #[tokio::test]
    async fn every_known_route_answers() {
        let (url, _tmp) = serve(None).await;
        let http = reqwest::Client::new();
        let routes = known_routes();
        for (method, path) in &routes {
            let r = http
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            let status = r.status();
            let body = r.text().await.unwrap();
            assert_ne!(status, 405, "{method} {path} is not routed for its method");
            assert_ne!(
                body, UNKNOWN_API_PATH,
                "{method} {path} reached the fallback"
            );
        }
        println!("probed {} routes", routes.len());
    }

    #[tokio::test]
    async fn team_routes_are_not_scoped_and_an_unknown_workspace_is_404() {
        let (url, _tmp) = serve(None).await;
        let http = reqwest::Client::new();
        // Team routes exist only unprefixed: under a workspace they are unknown paths.
        for path in [
            "/api/w/default/queue",
            "/api/w/default/pair/start",
            "/api/w/default/me",
            "/api/w/default/workspaces",
            "/api/w/default/room/messages",
        ] {
            let r = http.post(format!("{url}{path}")).send().await.unwrap();
            assert_eq!(r.status(), 404, "{path}");
            assert_eq!(r.text().await.unwrap(), UNKNOWN_API_PATH, "{path}");
        }
        // A workspace this server does not hold: 404, never a fallback to `default`.
        for (method, path) in [
            (Method::GET, "/api/w/nope/overview"),
            (Method::PUT, "/api/w/nope/layout"),
            (Method::POST, "/api/w/Default/pins"),
        ] {
            let r = http
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 404, "{method} {path}");
            assert_eq!(
                r.text().await.unwrap(),
                crate::kernel::http::UNKNOWN_WORKSPACE,
                "{method} {path}"
            );
        }
    }

    #[tokio::test]
    async fn unknown_api_paths_are_404_json() {
        let (url, _tmp) = serve(None).await;
        let http = reqwest::Client::new();
        for (method, path) in [
            (Method::GET, "/api/nope"),
            (Method::POST, "/api/nope"),
            (Method::GET, "/ws/nope"),
            (Method::GET, "/auth/nope"),
            // ADR 0074: without RUBATO_GITHUB_WEBHOOK_SECRET the webhook path is unknown.
            (Method::POST, "/webhooks/github"),
            (Method::GET, "/webhooks/github"),
            (Method::POST, "/webhooks/nope"),
        ] {
            let r = http
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 404, "{method} {path}");
            assert_eq!(
                r.headers()["content-type"],
                "application/json",
                "{method} {path}"
            );
            assert_eq!(r.text().await.unwrap(), UNKNOWN_API_PATH);
        }
    }

    #[tokio::test]
    async fn deep_links_still_serve_the_app() {
        let dist = tempfile::tempdir().unwrap();
        std::fs::write(
            dist.path().join("index.html"),
            "<!doctype html><title>r</title>",
        )
        .unwrap();
        let (url, _tmp) = serve(Some(dist.path().to_owned())).await;
        let r = reqwest::get(format!("{url}/some/page")).await.unwrap();
        // As on main: `not_found_service` serves index.html with status 404 (design N17 said
        // 200; the status is kept, not changed, by this move).
        assert_eq!(r.status(), 404);
        assert!(
            r.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(r.text().await.unwrap().contains("<title>r</title>"));
    }

    // Issue #193 (ADR 0066): same-origin writes, headers, companion caps, the login's text.

    /// A served router with in-memory metrics, the map loaded, and `configure` applied.
    async fn hardened(
        configure: impl FnOnce(&mut Config),
    ) -> (
        String,
        App,
        crate::kernel::telemetry::metric_reader::Reader,
        tempfile::TempDir,
    ) {
        use crate::kernel::telemetry::metric_reader;
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut cfg = Config::for_tests(url, "m0x6A");
        configure(&mut cfg);
        let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let map: rubato_protocol::SystemMap =
            serde_yaml_ng::from_str(include_str!("../../../docs/design/rubato-map.yml")).unwrap();
        app.set_map(
            &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
            crate::kernel::state::LoadedMap {
                map,
                version: "v".into(),
                loaded_at: chrono::Utc::now(),
                yaml: String::new(),
            },
        )
        .await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = app.clone();
        tokio::spawn(async move {
            axum::serve(listener, router_before_tenants(served))
                .await
                .unwrap()
        });
        (base, app, reader, tmp)
    }

    fn dropped(
        reader: &crate::kernel::telemetry::metric_reader::Reader,
        metric: &str,
        reason: &str,
    ) -> u64 {
        use rubato_protocol::otel::attr;
        reader
            .points(metric)
            .into_iter()
            .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(reason))
            .map(|p| p.value)
            .sum()
    }

    /// Pair a companion through the device-code flow as the dev user; its id and token.
    async fn pair_companion(base: &str) -> (uuid::Uuid, String) {
        use rubato_protocol::pairing::{
            PairApproveRequest, PairPollRequest, PairPollResponse, PairStartRequest,
            PairStartResponse,
        };
        let http = reqwest::Client::new();
        let companion_id = uuid::Uuid::new_v4();
        let start: PairStartResponse = http
            .post(format!("{base}{}", pr::PAIR_START))
            .json(&PairStartRequest {
                companion_id,
                hostname: "h".into(),
                companion_version: "t".into(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let approved = http
            .post(format!("{base}{}", pr::PAIR_APPROVE))
            .json(&PairApproveRequest {
                user_code: start.user_code.clone(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(approved.status(), 200);
        let poll: PairPollResponse = http
            .post(format!("{base}{}", pr::PAIR_POLL))
            .json(&PairPollRequest {
                device_code: start.device_code,
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let PairPollResponse::Approved { token, .. } = poll else {
            panic!("{poll:?}")
        };
        (companion_id, token)
    }

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// Open a websocket at `path` with `headers`; the socket, or the refusal's status.
    async fn open_ws(base: &str, path: &str, headers: &[(&str, &str)]) -> Result<Socket, u16> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = format!("ws://{}{path}", base.trim_start_matches("http://"))
            .into_client_request()
            .unwrap();
        for (k, v) in headers {
            req.headers_mut().insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        match tokio_tungstenite::connect_async(req).await {
            Ok((ws, _)) => Ok(ws),
            Err(tokio_tungstenite::tungstenite::Error::Http(r)) => Err(r.status().as_u16()),
            Err(e) => panic!("{e}"),
        }
    }

    async fn send_env(ws: &mut Socket, seq: u64, message: serde_json::Value) {
        use futures_util::SinkExt;
        let env =
            serde_json::json!({"seq": seq, "sent_at": "2026-11-15T12:00:00Z", "message": message});
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            env.to_string().into(),
        ))
        .await
        .unwrap();
    }

    /// The next server message, or `None` when the socket ended.
    async fn next_msg(ws: &mut Socket) -> Option<rubato_protocol::ServerMessage> {
        use futures_util::StreamExt;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
                .await
                .expect("an answer in time")
            {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => {
                    let env: rubato_protocol::ServerEnvelope = serde_json::from_str(&t).unwrap();
                    return Some(env.message);
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
                | None
                | Some(Err(_)) => {
                    return None;
                }
                Some(Ok(_)) => {}
            }
        }
    }

    fn hello(companion_id: uuid::Uuid) -> serde_json::Value {
        serde_json::json!({"type": "hello", "companion_id": companion_id, "user": "m0x6A",
            "companion_version": "t", "protocol_version": rubato_protocol::PROTOCOL_VERSION,
            "last_seen_server_seq": 0})
    }

    /// #69: this companion's `rubato.sessions.by_state` points, by state.
    fn by_state(
        reader: &crate::kernel::telemetry::metric_reader::Reader,
        companion_id: uuid::Uuid,
    ) -> std::collections::BTreeMap<String, u64> {
        use rubato_protocol::otel::{attr, metric};
        reader
            .points(metric::SESSIONS_BY_STATE)
            .into_iter()
            .filter(|p| p.attrs.get(attr::COMPANION_ID) == Some(&companion_id.to_string()))
            .map(|p| (p.attrs[attr::SESSION_STATE].clone(), p.value))
            .collect()
    }

    /// Poll the reader until this companion's counts are `want` (states not named are 0): a
    /// close is handled asynchronously, so an assertion right after it would prove nothing.
    async fn counts_become(
        reader: &crate::kernel::telemetry::metric_reader::Reader,
        companion_id: uuid::Uuid,
        want: &[(&str, u64)],
    ) -> usize {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let got = by_state(reader, companion_id);
            let ok = got.len() == 5
                && got
                    .iter()
                    .all(|(k, v)| *v == want.iter().find(|(s, _)| s == k).map_or(0, |(_, n)| *n));
            if ok {
                return got.len();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "counts {got:?}, want {want:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// One working and one ended session of this companion, from the protocol fixture.
    fn working_and_ended(companion_id: uuid::Uuid) -> serde_json::Value {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../rubato-protocol/fixtures/companion_sessions.json"
        ))
        .unwrap();
        let mut working = fixture["message"]["sessions"][0].clone();
        working["companion_id"] = serde_json::json!(companion_id);
        working["status"] = serde_json::json!("working");
        let mut ended = working.clone();
        ended["status"] = serde_json::json!("ended");
        ended["session_id"] = serde_json::json!(uuid::Uuid::new_v4());
        ended["tmux"]["pane"] = serde_json::json!("%99");
        serde_json::json!({"type": "sessions", "sessions": [working, ended]})
    }

    /// A paired companion's socket that said `Hello` at `seq` (a new one: a recorded sequence
    /// number is a replay) and was welcomed.
    async fn hello_socket(base: &str, companion_id: uuid::Uuid, token: &str, seq: u64) -> Socket {
        let bearer = format!("Bearer {token}");
        let mut ws = open_ws(
            base,
            pr::COMPANION_WS,
            &[("authorization", bearer.as_str())],
        )
        .await
        .unwrap();
        send_env(&mut ws, seq, hello(companion_id)).await;
        let first = next_msg(&mut ws).await;
        assert!(
            matches!(first, Some(rubato_protocol::ServerMessage::Welcome { .. })),
            "{first:?}"
        );
        ws
    }

    /// #69: a companion whose socket ends counts its sessions that had not ended as unknown,
    /// and its `Hello` on a new socket records them again from the stored snapshot.
    #[tokio::test(flavor = "current_thread")]
    async fn a_disconnected_companion_counts_its_sessions_as_unknown() {
        let (base, app, reader, _tmp) = hardened(|_| {}).await;
        let (companion_id, token) = pair_companion(&base).await;
        let mut ws = hello_socket(&base, companion_id, &token, 1).await;
        send_env(&mut ws, 2, working_and_ended(companion_id)).await;
        assert_eq!(
            next_msg(&mut ws).await,
            Some(rubato_protocol::ServerMessage::Ack { seq: 2 })
        );
        let mut checked =
            counts_become(&reader, companion_id, &[("working", 1), ("ended", 1)]).await;
        let stored = || async {
            let row: (String,) = sqlx::query_as("SELECT sessions FROM session_snapshots")
                .fetch_one(&app.db)
                .await
                .unwrap();
            row.0
        };
        let (row_before, cache_before) = (stored().await, app.latest_sessions());

        futures_util::SinkExt::close(&mut ws).await.unwrap();
        checked += counts_become(&reader, companion_id, &[("unknown", 1), ("ended", 1)]).await;
        assert_eq!(
            stored().await,
            row_before,
            "the stored snapshot is not changed"
        );
        assert_eq!(app.latest_sessions(), cache_before, "nor the cache");

        // Back, with no new snapshot: the stored states again.
        let _ws = hello_socket(&base, companion_id, &token, 3).await;
        checked += counts_become(&reader, companion_id, &[("working", 1), ("ended", 1)]).await;
        println!("checked {checked} points over a snapshot, a close and a Hello");
    }

    /// #69: a socket replaced by a newer one leaves the counts to the newer one when it ends.
    #[tokio::test(flavor = "current_thread")]
    async fn a_replaced_socket_leaves_the_counts_to_the_newer_one() {
        let (events, _guard) = crate::kernel::telemetry::capture::start();
        let (base, _app, reader, _tmp) = hardened(|_| {}).await;
        let (companion_id, token) = pair_companion(&base).await;
        let mut first = hello_socket(&base, companion_id, &token, 1).await;
        send_env(&mut first, 2, working_and_ended(companion_id)).await;
        assert!(next_msg(&mut first).await.is_some());
        counts_become(&reader, companion_id, &[("working", 1), ("ended", 1)]).await;

        let _second = hello_socket(&base, companion_id, &token, 3).await;
        assert_eq!(
            next_msg(&mut first).await,
            None,
            "the server ends the older socket"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !events.lock().unwrap().iter().any(|e| {
            e.message == "companion disconnected"
                && e.fields.get("sessions").map(String::as_str) == Some("left_to_newer")
        }) {
            assert!(
                std::time::Instant::now() < deadline,
                "the older socket's end was not logged"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        counts_become(&reader, companion_id, &[("working", 1), ("ended", 1)]).await;
    }

    /// #69: a revoked companion is gone: every count is 0, not unknown.
    #[tokio::test(flavor = "current_thread")]
    async fn a_revoked_companion_counts_zero() {
        let (base, _app, reader, _tmp) = hardened(|_| {}).await;
        let (companion_id, token) = pair_companion(&base).await;
        let mut ws = hello_socket(&base, companion_id, &token, 1).await;
        send_env(&mut ws, 2, working_and_ended(companion_id)).await;
        assert!(next_msg(&mut ws).await.is_some());
        counts_become(&reader, companion_id, &[("working", 1), ("ended", 1)]).await;
        let revoked = reqwest::Client::new()
            .delete(format!("{base}{}/{companion_id}", pr::PAIRINGS))
            .send()
            .await
            .unwrap();
        assert_eq!(revoked.status(), 204);
        counts_become(&reader, companion_id, &[]).await;
    }

    /// Plan 189 PR 1: every reader takes the kernel's registry, not `Config`. A workspace added
    /// to the registry is listed and passes `check_workspace`; one never added is still unknown.
    #[tokio::test(flavor = "current_thread")]
    async fn readers_follow_the_workspace_registry() {
        use crate::kernel::workspaces::{Registry, Source};
        let (base, app, _reader, _tmp) = hardened(|_| {}).await;
        let http = reqwest::Client::new();
        let get = |path: String| {
            let http = http.clone();
            async move { http.get(path).send().await.unwrap() }
        };
        let scoped_map = |ws: &str| format!("{base}/api/w/{ws}/map");
        // Before: shop is not held.
        let before = get(scoped_map("shop")).await;
        assert_eq!(before.status(), 404);
        assert!(before.text().await.unwrap().contains("unknown_workspace"));

        let (mut entries, _) = app.workspace_entries();
        let mut shop = entries[0].clone();
        shop.id = "shop".into();
        shop.map_path = "docs/design/shop-map.yml".into();
        entries.push(shop);
        app.set_workspaces(Registry {
            entries,
            source: Source::Database,
        });

        let listed: Vec<rubato_protocol::WorkspaceInfo> =
            get(format!("{base}{}", workspace_routes::WORKSPACES))
                .await
                .json()
                .await
                .unwrap();
        let ids: Vec<&str> = listed.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["default", "shop"]);
        let after = get(scoped_map("shop")).await;
        assert!(
            !after.text().await.unwrap().contains("unknown_workspace"),
            "shop passes check_workspace"
        );
        let never = get(scoped_map("nope")).await;
        assert_eq!(
            never.status(),
            404,
            "a workspace never added is still unknown"
        );
        println!("checked 2 listed workspaces and 3 scoped reads");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_signed_delivery_passes_the_same_origin_check() {
        let secret = "s3cret";
        let (base, _app, _reader, _tmp) = hardened(|c| {
            c.github_webhook_secret =
                crate::kernel::config::WebhookSecret::from_env_value(Some(secret.into()));
        })
        .await;
        let http = reqwest::Client::new();
        let body = r#"{"action":"closed","pull_request":{"head":{"ref":"issue/5-x"}},"repository":{"full_name":"m0x6A/rubato"}}"#;
        let evil = [
            ("sec-fetch-site", "cross-site"),
            ("origin", "https://evil.example"),
        ];
        let send = |id: &'static str, sig: String| {
            let mut r = http
                .post(format!("{base}{}", webhooks::PATH))
                .header("x-github-delivery", id)
                .header("x-github-event", "pull_request")
                .header("x-hub-signature-256", sig)
                .body(body);
            for (k, v) in evil {
                r = r.header(k, v);
            }
            r.send()
        };
        let good = send("d-1", webhooks::sign(secret.as_bytes(), body.as_bytes()))
            .await
            .unwrap();
        assert_eq!(
            good.status(),
            202,
            "signed: accepted whatever the browser headers say"
        );
        let bad = send("d-2", webhooks::sign(b"wrong", body.as_bytes()))
            .await
            .unwrap();
        assert_eq!(
            bad.status(),
            401,
            "refused by the signature, not by the origin check"
        );
        // The same headers on an ordinary write are still refused (ADR 0066).
        let mut w = http.post(format!("{base}{}", map_routes::MAP_RELOAD));
        for (k, v) in evil {
            w = w.header(k, v);
        }
        assert_eq!(w.send().await.unwrap().status(), 403);
        let get = http
            .get(format!("{base}{}", webhooks::PATH))
            .send()
            .await
            .unwrap();
        assert_eq!(get.status(), 405, "only POST is routed");
    }

    #[tokio::test]
    async fn cross_origin_writes_are_refused_and_counted() {
        use rubato_protocol::otel::{attr, metric};
        let (base, _app, reader, _tmp) = hardened(|_| {}).await;
        let http = reqwest::Client::new();
        let host = base.trim_start_matches("http://").to_owned();
        let post = |path: &str, headers: &[(&str, &str)]| {
            let mut r = http.post(format!("{base}{path}"));
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.send()
        };
        let refused: &[(&str, &[(&str, &str)])] = &[
            (
                map_routes::MAP_RELOAD,
                &[("origin", "https://evil.example")],
            ),
            (
                draft_routes::DRAFT_DISCARD,
                &[("origin", "https://evil.example")],
            ),
            (
                draft_routes::DRAFT_DISCARD,
                &[("sec-fetch-site", "same-site")],
            ),
            (
                "/api/w/default/map/draft/discard",
                &[("sec-fetch-site", "cross-site")],
            ),
            // ADR 0075: a new workspace is a write like any other.
            (
                workspace_routes::WORKSPACES,
                &[("origin", "https://evil.example")],
            ),
        ];
        for (path, headers) in refused {
            let r = post(path, headers).await.unwrap();
            assert_eq!(r.status(), 403, "{path} {headers:?}");
            assert_eq!(r.text().await.unwrap(), crate::kernel::http::CROSS_ORIGIN);
        }
        let own = format!("http://{host}");
        let allowed: &[&[(&str, &str)]] = &[
            &[("sec-fetch-site", "same-origin")],
            &[("origin", own.as_str())],
            &[],
        ];
        for headers in allowed {
            let r = post(draft_routes::DRAFT_DISCARD, headers).await.unwrap();
            assert_ne!(r.status(), 403, "{headers:?}");
        }
        let points: Vec<_> = reader
            .points(metric::WEB_REJECTED)
            .into_iter()
            .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some("cross_origin"))
            .collect();
        assert_eq!(points.iter().map(|p| p.value).sum::<u64>(), 5, "{points:?}");
        assert!(points.iter().any(|p| p.attrs[attr::ROUTE]
            == tenant_routes::in_tenant("{tenant}", workspace_routes::WORKSPACES)));
        assert!(points.iter().any(|p| p.attrs[attr::ROUTE]
            == workspace_routes::scoped("{tenant}", "{ws}", map_routes::MAP_RELOAD)));
        assert!(
            points
                .iter()
                .any(|p| p.attrs[attr::ROUTE] == "/api/t/{tenant}/w/{ws}/map/draft/discard")
        );
        println!(
            "checked {} refused and {} allowed writes",
            refused.len(),
            allowed.len()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn websocket_upgrades_check_the_origin() {
        let (base, _app, _reader, _tmp) = hardened(|_| {}).await;
        let host = base.trim_start_matches("http://").to_owned();
        let evil = [("origin", "https://evil.example")];
        assert_eq!(
            open_ws(&base, "/ws/web?t=default", &evil).await.err(),
            Some(403)
        );
        let own = format!("http://{host}");
        assert!(
            open_ws(&base, "/ws/web?t=default", &[("origin", own.as_str())])
                .await
                .is_ok()
        );
        let (companion_id, token) = pair_companion(&base).await;
        let bearer = format!("Bearer {token}");
        let auth = ("authorization", bearer.as_str());
        assert_eq!(
            open_ws(&base, pr::COMPANION_WS, &[auth, evil[0]])
                .await
                .err(),
            Some(403)
        );
        // The companion sends no Origin: it upgrades and is welcomed as before.
        let mut ws = open_ws(&base, pr::COMPANION_WS, &[auth])
            .await
            .expect("upgrade");
        send_env(&mut ws, 1, hello(companion_id)).await;
        assert!(matches!(
            next_msg(&mut ws).await,
            Some(rubato_protocol::ServerMessage::Welcome { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn every_response_carries_the_security_headers() {
        let dist = tempfile::tempdir().unwrap();
        std::fs::write(
            dist.path().join("index.html"),
            "<!doctype html><title>r</title>",
        )
        .unwrap();
        let d = dist.path().to_owned();
        let (base, _app, _reader, _tmp) = hardened(|c| {
            c.web_dist = d;
            c.public_url = "http://192.168.1.20:8081".into();
        })
        .await;
        for path in ["/api/health", "/api/nope", "/", "/some/page"] {
            let r = reqwest::get(format!("{base}{path}")).await.unwrap();
            assert_eq!(r.headers()["x-content-type-options"], "nosniff", "{path}");
            let csp = r.headers()["content-security-policy"].to_str().unwrap();
            assert!(csp.starts_with("default-src 'self';"), "{path}: {csp}");
            assert!(csp.contains("ws://192.168.1.20:8081"), "{path}: {csp}");
            assert!(csp.contains("frame-ancestors 'none'"), "{path}: {csp}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_oversized_companion_message_ends_the_socket() {
        use rubato_protocol::otel::metric;
        let (base, _app, reader, _tmp) = hardened(|_| {}).await;
        let (companion_id, token) = pair_companion(&base).await;
        let bearer = format!("Bearer {token}");
        let mut ws = open_ws(
            &base,
            pr::COMPANION_WS,
            &[("authorization", bearer.as_str())],
        )
        .await
        .unwrap();
        send_env(&mut ws, 1, hello(companion_id)).await;
        assert!(next_msg(&mut ws).await.is_some());
        // A 3 MiB hook is an honest size (the companion takes 2 MiB hook bodies): acknowledged.
        let big = "x".repeat(3 * 1024 * 1024);
        let hook = serde_json::json!({"type": "hook", "session_id": "s", "hook_event_name": "PostToolUse",
            "cwd": "/w", "received_at": "2026-11-15T12:00:00Z", "payload": {"tool_input": big}});
        send_env(&mut ws, 2, hook).await;
        assert_eq!(
            next_msg(&mut ws).await,
            Some(rubato_protocol::ServerMessage::Ack { seq: 2 })
        );
        assert_eq!(dropped(&reader, metric::EVENTS_DROPPED, "too_large"), 0);
        // Over 4 MiB: the socket ends, counted once.
        let huge = "x".repeat(5 * 1024 * 1024);
        // The server stops reading at the cap, so the send itself may see the reset.
        let env = serde_json::json!({"seq": 3, "sent_at": "2026-11-15T12:00:00Z",
            "message": {"type": "hook", "session_id": huge}});
        let sent = futures_util::SinkExt::send(
            &mut ws,
            tokio_tungstenite::tungstenite::Message::Text(env.to_string().into()),
        )
        .await;
        if sent.is_ok() {
            assert_eq!(next_msg(&mut ws).await, None);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(dropped(&reader, metric::EVENTS_DROPPED, "too_large"), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_snapshot_over_the_cap_is_not_stored() {
        use rubato_protocol::otel::metric;
        let (base, app, reader, _tmp) = hardened(|_| {}).await;
        let (companion_id, token) = pair_companion(&base).await;
        let bearer = format!("Bearer {token}");
        let mut ws = open_ws(
            &base,
            pr::COMPANION_WS,
            &[("authorization", bearer.as_str())],
        )
        .await
        .unwrap();
        send_env(&mut ws, 1, hello(companion_id)).await;
        assert!(next_msg(&mut ws).await.is_some());
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../rubato-protocol/fixtures/companion_sessions.json"
        ))
        .unwrap();
        let one = fixture["message"]["sessions"][1].clone();
        let sessions = |n: usize| -> serde_json::Value {
            (0..n)
                .map(|i| {
                    let mut s = one.clone();
                    s["companion_id"] = serde_json::json!(companion_id);
                    s["tmux"]["pane"] = serde_json::json!(format!("%{i}"));
                    s
                })
                .collect()
        };
        let stored = || async {
            let row: (String,) = sqlx::query_as("SELECT sessions FROM session_snapshots")
                .fetch_one(&app.db)
                .await
                .unwrap();
            serde_json::from_str::<Vec<serde_json::Value>>(&row.0)
                .unwrap()
                .len()
        };
        let seqs = [(2u64, 1usize), (3, 257), (4, 256)];
        for (seq, n) in seqs {
            send_env(
                &mut ws,
                seq,
                serde_json::json!({"type": "sessions", "sessions": sessions(n)}),
            )
            .await;
            assert_eq!(
                next_msg(&mut ws).await,
                Some(rubato_protocol::ServerMessage::Ack { seq }),
                "{n} sessions"
            );
            let want = if n > 256 { 1 } else { n };
            assert_eq!(stored().await, want, "after {n} sessions");
            // #69: a refused snapshot records nothing; the gauge keeps the stored one's counts.
            assert_eq!(
                by_state(&reader, companion_id).get("blocked").copied(),
                Some(want as u64),
                "the gauge after {n} sessions"
            );
        }
        assert_eq!(
            dropped(&reader, metric::EVENTS_DROPPED, "too_many_sessions"),
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_callback_does_not_echo_upstream_errors() {
        use rubato_protocol::otel::metric;
        let stub = axum::Router::new().route(
            "/login/oauth/access_token",
            axum::routing::post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({"error": "bad_verification_code",
                        "error_description": "UPSTREAM-SECRET-TEXT"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let stub_url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
        let s = stub_url.clone();
        let (base, _app, reader, _tmp) = hardened(move |c| {
            c.github_client_id = Some("Iv1.test".into());
            c.github_client_secret = Some("secret".into());
            c.github_web_url = Some(s.clone());
            c.github_api_url = Some(s);
            c.dev_user = None;
        })
        .await;
        let r = reqwest::Client::new()
            .get(format!("{base}/auth/github/callback?code=c1&state=s1"))
            .header("cookie", "rubato_oauth_state=s1")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 502);
        let body = r.text().await.unwrap();
        assert!(!body.contains("UPSTREAM-SECRET-TEXT"), "{body}");
        assert!(!body.contains("bad_verification_code"), "{body}");
        assert_eq!(body, "GitHub did not complete the login. Try again.");
        assert_eq!(
            dropped(&reader, metric::WEB_REJECTED, "token_exchange_failed"),
            1
        );
    }
}
