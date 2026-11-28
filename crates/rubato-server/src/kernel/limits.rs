//! What bounds a request before any handler runs (ADR 0090, design 345 §7): whose address it
//! comes from, token buckets on the anonymous paths and on writes, a timeout, and a bound on how
//! many requests run at once. One layer, [`guard`], outside every route.
//!
//! - **Client address.** The socket's peer, unless the peer is a proxy named in
//!   `RUBATO_TRUSTED_PROXY`: then the last address in `X-Forwarded-For` that is not such a proxy.
//!   A header from anyone else is not read, so a client cannot choose its own bucket.
//! - **Anonymous paths** (`/auth/*`, `/api/pair/start`, `/api/pair/poll`): a bucket per address.
//! - **Writes**: a bucket per web session. Not in local development (`RUBATO_DEV_USER`, loopback
//!   only), where there is no session and nobody to protect from.
//! - A refusal is 429 with `Retry-After`, counted on `rubato.web.rejected` with reason
//!   `rate_limited` and the route's template.
//! - **Timeout**: 30 s, 120 s for the routes that chain GitHub writes; never a websocket.

use crate::kernel::config::Net;
use crate::kernel::state::App;
use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// An address may make this many anonymous requests at once, and one more each
/// [`ANONYMOUS_REFILL_MS`]. A companion that waits for its pairing polls every 3 s; a login is
/// three requests.
pub const ANONYMOUS_BURST: u32 = 30;
pub const ANONYMOUS_REFILL_MS: u32 = 1_000;
/// A session may make this many writes at once, and one more each [`WRITE_REFILL_MS`].
pub const WRITE_BURST: u32 = 60;
pub const WRITE_REFILL_MS: u32 = 500;
/// How long a request may take; a websocket is not a request in this sense.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// For the routes that chain GitHub writes, above their worst case of chained 15 s calls.
pub const LONG_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// How many requests run at once; the next waits, inside its own timeout.
pub const MAX_CONCURRENT_REQUESTS: usize = 256;
/// The most keys a set of buckets remembers; past it, the one idle longest is forgotten.
pub const MAX_BUCKETS: usize = 10_000;
/// Reasons on `rubato.web.rejected`.
pub mod reason {
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const TIMEOUT: &str = "timeout";
    pub const BUSY: &str = "busy";
}

/// Token buckets by key: `burst` tokens at most, one back every `refill_ms`.
pub struct Buckets {
    burst: f64,
    refill_ms: f64,
    max: usize,
    held: Mutex<HashMap<String, (f64, Instant)>>,
}

impl Buckets {
    pub fn new(burst: u32, refill_ms: u32) -> Self {
        Self::with_max(burst, refill_ms, MAX_BUCKETS)
    }

    pub fn with_max(burst: u32, refill_ms: u32, max: usize) -> Self {
        Self {
            burst: f64::from(burst),
            refill_ms: f64::from(refill_ms.max(1)),
            max,
            held: Mutex::new(HashMap::new()),
        }
    }

    /// Take one token for `key`. `Err` says how long until there is one.
    pub fn take(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let mut held = self.held.lock().unwrap();
        if held.len() >= self.max
            && !held.contains_key(key)
            && let Some(oldest) = held
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone())
        {
            held.remove(&oldest);
        }
        let (tokens, at) = held.entry(key.to_owned()).or_insert((self.burst, now));
        let refilled = now.saturating_duration_since(*at).as_secs_f64() * 1000.0 / self.refill_ms;
        *tokens = (*tokens + refilled).min(self.burst);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64(
                (1.0 - *tokens) * self.refill_ms / 1000.0,
            ))
        }
    }

    pub fn keys(&self) -> usize {
        self.held.lock().unwrap().len()
    }
}

/// What the guard holds for one server.
pub struct Limiter {
    pub anonymous: Buckets,
    pub writes: Buckets,
    pub running: tokio::sync::Semaphore,
    /// [`REQUEST_TIMEOUT`] and [`LONG_REQUEST_TIMEOUT`]; a test shortens them.
    pub request_timeout: Duration,
    pub long_request_timeout: Duration,
}

impl Default for Limiter {
    fn default() -> Self {
        Self {
            anonymous: Buckets::new(ANONYMOUS_BURST, ANONYMOUS_REFILL_MS),
            writes: Buckets::new(WRITE_BURST, WRITE_REFILL_MS),
            running: tokio::sync::Semaphore::new(MAX_CONCURRENT_REQUESTS),
            request_timeout: REQUEST_TIMEOUT,
            long_request_timeout: LONG_REQUEST_TIMEOUT,
        }
    }
}

/// The address a request came from, as [`guard`] worked it out; on every request's extensions
/// when the server knows its peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr(pub IpAddr);

/// The client's address: `peer`, or, when `peer` is a trusted proxy, the last address in
/// `X-Forwarded-For` that is not one. A header that does not read leaves the peer.
pub fn client_addr(peer: IpAddr, forwarded: Option<&str>, trusted: &[Net]) -> IpAddr {
    let is_proxy = |ip: IpAddr| trusted.iter().any(|n| n.contains(ip));
    if !is_proxy(peer) {
        return peer;
    }
    forwarded
        .into_iter()
        .flat_map(|h| h.rsplit(','))
        .map(|part| part.trim().parse::<IpAddr>())
        .take_while(Result::is_ok)
        .flatten()
        .find(|ip| !is_proxy(*ip))
        .unwrap_or(peer)
}

/// Whether a path is answered without a session: login and the companion's pairing.
fn anonymous(path: &str) -> bool {
    use rubato_protocol::pairing::routes as pr;
    path.starts_with("/auth/") || path == pr::PAIR_START || path == pr::PAIR_POLL
}

/// The routes that chain GitHub writes (ADR 0090): by the end of their template.
fn long_running(route: &str) -> bool {
    [
        rubato_protocol::map_draft::routes::DRAFT_PROPOSE,
        rubato_protocol::approve::routes::PLAN_APPROVE,
        rubato_protocol::approve::routes::PLAN_DRAFT_APPROVE,
        rubato_protocol::pulls::routes::PULLS_MERGE,
        rubato_protocol::plan_draft::routes::PLAN_IMPORT,
    ]
    .iter()
    .any(|r| route.ends_with(r.trim_start_matches("/api")))
}

fn too_many(app: &App, route: &str, who: &str, wait: Duration) -> Response {
    app.reject(route, reason::RATE_LIMITED, who);
    let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
    let mut res = (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"reason":"rate_limited"}"#,
    )
        .into_response();
    res.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(secs));
    res
}

fn unavailable(app: &App, route: &str, why: &'static str) -> Response {
    app.reject(route, why, "-");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"reason":"{why}"}}"#),
    )
        .into_response()
}

/// The layer outside every route. See the module.
pub async fn guard(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    let client = peer.map(|p| client_addr(p, forwarded, &app.config.trusted_proxies));
    if let Some(ip) = client {
        req.extensions_mut().insert(ClientAddr(ip));
    }
    if !app.config.request_limits {
        return next.run(req).await;
    }
    // The template, never the concrete path: a probe's path does not become a label.
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", |m| m.as_str())
        .to_owned();
    let path = req.uri().path();
    let now = Instant::now();
    if anonymous(path) {
        let key = client.map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
        if let Err(wait) = app.limiter.anonymous.take(&key, now) {
            return too_many(&app, &route, &key, wait);
        }
    } else if !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS)
        && app.config.dev_user.is_none()
        && let Some(session) =
            crate::kernel::http::cookie(req.headers(), app.config.session_cookie())
                .and_then(|v| app.signer.verify(&v))
        && let Err(wait) = app.limiter.writes.take(&session, now)
    {
        return too_many(&app, &route, "session", wait);
    }
    // A websocket lives as long as its page: no timeout, and it holds no place among requests.
    if path.starts_with("/ws/") {
        return next.run(req).await;
    }
    let limit = if long_running(&route) {
        app.limiter.long_request_timeout
    } else {
        app.limiter.request_timeout
    };
    let run = async {
        let Ok(_permit) = app.limiter.running.acquire().await else {
            return unavailable(&app, &route, reason::BUSY);
        };
        next.run(req).await
    };
    match tokio::time::timeout(limit, run).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!(rubato.route = %route, secs = limit.as_secs(), "request timed out");
            unavailable(&app, &route, reason::TIMEOUT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_gives_its_burst_then_one_per_refill_and_says_how_long_to_wait() {
        let b = Buckets::new(3, 1_000);
        let t0 = Instant::now();
        for _ in 0..3 {
            assert_eq!(b.take("a", t0), Ok(()));
        }
        let wait = b.take("a", t0).unwrap_err();
        assert!(
            wait > Duration::from_millis(900) && wait <= Duration::from_secs(1),
            "{wait:?}"
        );
        // Another key has its own tokens.
        assert_eq!(b.take("b", t0), Ok(()));
        // One token back after one refill, and never more than the burst.
        assert_eq!(b.take("a", t0 + Duration::from_millis(1_000)), Ok(()));
        assert!(b.take("a", t0 + Duration::from_millis(1_000)).is_err());
        let later = t0 + Duration::from_secs(3_600);
        for _ in 0..3 {
            assert_eq!(b.take("a", later), Ok(()));
        }
        assert!(b.take("a", later).is_err());
    }

    #[test]
    fn the_buckets_forget_the_key_idle_longest_past_their_cap() {
        let b = Buckets::with_max(1, 1_000, 2);
        let t0 = Instant::now();
        b.take("old", t0).unwrap();
        b.take("mid", t0 + Duration::from_millis(1)).unwrap();
        b.take("new", t0 + Duration::from_millis(2)).unwrap();
        assert_eq!(b.keys(), 2);
        // "old" was forgotten, so it starts full again; "mid" is still empty.
        assert!(b.take("mid", t0 + Duration::from_millis(3)).is_err());
        assert_eq!(b.take("old", t0 + Duration::from_millis(3)), Ok(()));
    }

    #[test]
    fn the_client_address_is_the_peer_unless_the_peer_is_a_trusted_proxy() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let proxies = [Net::parse("172.18.0.0/16").unwrap()];
        let cases = [
            // A client that talks to the server itself cannot name its own address.
            ("203.0.113.9", Some("1.2.3.4"), "203.0.113.9"),
            ("203.0.113.9", None, "203.0.113.9"),
            // Through the proxy: what the proxy appended, the rightmost address.
            ("172.18.0.2", Some("198.51.100.7"), "198.51.100.7"),
            // A client that sent its own header: its claim is left of the proxy's entry.
            ("172.18.0.2", Some("1.2.3.4, 198.51.100.7"), "198.51.100.7"),
            // Two trusted hops.
            (
                "172.18.0.2",
                Some("198.51.100.7, 172.18.0.3"),
                "198.51.100.7",
            ),
            // Nothing usable: the proxy's own address.
            ("172.18.0.2", None, "172.18.0.2"),
            ("172.18.0.2", Some("not an address"), "172.18.0.2"),
            ("172.18.0.2", Some("1.2.3.4, junk"), "172.18.0.2"),
        ];
        for (peer, header, want) in cases {
            assert_eq!(
                client_addr(ip(peer), header, &proxies),
                ip(want),
                "{peer} {header:?}"
            );
        }
        // With no proxy configured the header is never read.
        assert_eq!(
            client_addr(ip("172.18.0.2"), Some("1.2.3.4"), &[]),
            ip("172.18.0.2")
        );
        println!("checked {} address cases", cases.len() + 1);
    }

    #[test]
    fn only_login_and_pairing_are_anonymous_and_only_chained_github_writes_are_long() {
        for p in [
            "/auth/github/login",
            "/auth/github/callback",
            "/api/pair/start",
            "/api/pair/poll",
        ] {
            assert!(anonymous(p), "{p}");
        }
        for p in [
            "/api/pair/approve",
            "/api/me",
            "/api/t/acme/queue",
            "/authx",
        ] {
            assert!(!anonymous(p), "{p}");
        }
        for r in [
            "/api/t/{tenant}/w/{ws}/map/draft/propose",
            "/api/t/{tenant}/w/{ws}/plans/approve",
            "/api/t/{tenant}/w/{ws}/plans/drafts/approve",
            "/api/t/{tenant}/w/{ws}/pulls/merge",
            "/api/t/{tenant}/w/{ws}/plans/import",
        ] {
            assert!(long_running(r), "{r}");
        }
        for r in [
            "/api/t/{tenant}/w/{ws}/overview",
            "/api/t/{tenant}/queue",
            "unmatched",
        ] {
            assert!(!long_running(r), "{r}");
        }
    }

    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use axum::Router;
    use axum::routing::{get, post};
    use std::sync::Arc;

    /// A served router: requests reach it over a socket, as a proxy on this machine would send
    /// them, naming their client in `X-Forwarded-For`.
    struct Served {
        url: String,
        http: reqwest::Client,
    }

    async fn serve(router: Router) -> Served {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap()
        });
        Served {
            url,
            http: reqwest::Client::new(),
        }
    }

    impl Served {
        /// A request whose client, as the proxy reports it, is `client`; with none, the
        /// proxy's own request.
        fn request(&self, method: &str, path: &str, client: &str) -> reqwest::RequestBuilder {
            let r = self
                .http
                .request(method.parse().unwrap(), format!("{}{path}", self.url));
            if client.is_empty() {
                r
            } else {
                r.header("x-forwarded-for", client)
            }
        }
    }

    /// (status, Retry-After, body).
    async fn answer(req: reqwest::RequestBuilder) -> (u16, Option<String>, String) {
        let res = req.send().await.unwrap();
        let retry = res
            .headers()
            .get("retry-after")
            .map(|v| v.to_str().unwrap().to_owned());
        let code = res.status().as_u16();
        (code, retry, res.text().await.unwrap())
    }

    /// A server with the limits on, behind the guard, which trusts a proxy on this machine: a
    /// quick route, a slow one, a slow one that chains GitHub writes by its template, and a
    /// websocket path.
    async fn guarded(
        set: impl FnOnce(&mut Config, &mut Limiter),
    ) -> (Served, App, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let mut cfg = Config::for_tests(url, "unused");
        cfg.dev_user = None;
        cfg.request_limits = true;
        cfg.trusted_proxies = vec![Net::parse("127.0.0.0/8").unwrap()];
        let mut limiter = Limiter::default();
        set(&mut cfg, &mut limiter);
        let mut app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
        Arc::get_mut(&mut app).unwrap().limiter = limiter;
        let slow = || async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            "done"
        };
        let router = Router::new()
            .route(
                "/api/me",
                get(|| async { "me" }).post(|| async { "written" }),
            )
            .route("/api/pair/poll", post(|| async { "polled" }))
            .route("/api/slow", get(slow))
            .route("/api/t/{tenant}/w/{ws}/pulls/merge", post(slow))
            .route("/ws/web", get(slow))
            .layer(axum::middleware::from_fn_with_state(app.clone(), guard))
            .with_state(app.clone());
        (serve(router).await, app, tmp)
    }

    const A: &str = "203.0.113.9";
    const POLL: &str = "/api/pair/poll";

    #[tokio::test]
    async fn an_address_gets_its_burst_on_the_anonymous_paths_and_then_429_with_retry_after() {
        let (s, app, _tmp) = guarded(|_, _| {}).await;
        for i in 0..ANONYMOUS_BURST {
            let (code, _, _) = answer(s.request("POST", POLL, A)).await;
            assert_eq!(code, 200, "request {i}");
        }
        let (code, retry, body) = answer(s.request("POST", POLL, A)).await;
        assert_eq!((code, body.as_str()), (429, r#"{"reason":"rate_limited"}"#));
        assert_eq!(retry.as_deref(), Some("1"));
        // Another address has its own bucket, and so has the proxy itself.
        assert_eq!(answer(s.request("POST", POLL, "198.51.100.7")).await.0, 200);
        assert_eq!(answer(s.request("POST", POLL, "")).await.0, 200);
        // What the client wrote into the header itself, left of what the proxy appended, opens
        // no new bucket.
        let forged = s.request("POST", POLL, &format!("192.0.2.99, {A}"));
        assert_eq!(answer(forged).await.0, 429);
        // Other paths are not anonymous: the address still reads.
        assert_eq!(answer(s.request("GET", "/api/me", A)).await.0, 200);
        assert_eq!(app.limiter.anonymous.keys(), 3);
        println!(
            "checked {} anonymous requests from 3 addresses",
            ANONYMOUS_BURST + 5
        );
    }

    #[tokio::test]
    async fn a_session_gets_its_burst_of_writes_and_reads_are_not_counted() {
        let (s, app, _tmp) = guarded(|_, _| {}).await;
        let cookie = |id: &str| format!("rubato_session={}", app.signer.sign(id));
        let write = |who: &str| {
            s.request("POST", "/api/me", A)
                .header("cookie", cookie(who))
        };
        for i in 0..WRITE_BURST {
            assert_eq!(answer(write("ada")).await.0, 200, "write {i}");
        }
        let (code, retry, _) = answer(write("ada")).await;
        assert_eq!(code, 429);
        assert_eq!(retry.as_deref(), Some("1"));
        // Another session from the same address writes; the limited one still reads.
        assert_eq!(answer(write("mia")).await.0, 200);
        let read = s
            .request("GET", "/api/me", A)
            .header("cookie", cookie("ada"));
        assert_eq!(answer(read).await.0, 200);
        // A cookie nobody signed is no session: it has no bucket to drain, and the handler
        // behind it is what refuses.
        let forged = s
            .request("POST", "/api/me", A)
            .header("cookie", "rubato_session=ada.forged");
        assert_eq!(answer(forged).await.0, 200);
        assert_eq!(app.limiter.writes.keys(), 2);
    }

    #[tokio::test]
    async fn a_request_ends_at_its_timeout_a_chained_write_gets_longer_and_a_websocket_none() {
        let (s, _app, _tmp) = guarded(|_, l| {
            l.request_timeout = Duration::from_millis(50);
            l.long_request_timeout = Duration::from_millis(5_000);
        })
        .await;
        let (code, _, body) = answer(s.request("GET", "/api/slow", A)).await;
        assert_eq!((code, body.as_str()), (503, r#"{"reason":"timeout"}"#));
        let merge = s.request("POST", "/api/t/acme/w/main/pulls/merge", A);
        let (code, _, body) = answer(merge).await;
        assert_eq!((code, body.as_str()), (200, "done"));
        let (code, _, body) = answer(s.request("GET", "/ws/web", A)).await;
        assert_eq!((code, body.as_str()), (200, "done"));
        assert_eq!(answer(s.request("GET", "/api/me", A)).await.0, 200);
    }

    #[tokio::test]
    async fn with_every_place_taken_the_next_request_waits_for_one_until_its_timeout() {
        let (s, app, _tmp) = guarded(|_, l| {
            l.running = tokio::sync::Semaphore::new(1);
            l.request_timeout = Duration::from_millis(100);
        })
        .await;
        let held = app.limiter.running.acquire().await.unwrap();
        let (code, _, body) = answer(s.request("GET", "/api/me", A)).await;
        assert_eq!((code, body.as_str()), (503, r#"{"reason":"timeout"}"#));
        drop(held);
        assert_eq!(answer(s.request("GET", "/api/me", A)).await.0, 200);
    }

    /// With the limits off (a test server), the guard still says whose address it is.
    #[tokio::test]
    async fn the_guard_names_the_client_address_for_the_handlers_behind_it() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = connect(&url).await.unwrap();
        let mut cfg = Config::for_tests(url, "unused");
        cfg.trusted_proxies = vec![Net::parse("127.0.0.0/8").unwrap()];
        let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
        let router = Router::new()
            .route(
                "/addr",
                get(|c: Option<axum::Extension<ClientAddr>>| async move {
                    c.map_or("none".to_owned(), |c| c.0.0.to_string())
                }),
            )
            .layer(axum::middleware::from_fn_with_state(app.clone(), guard))
            .with_state(app);
        let s = serve(router).await;
        assert_eq!(answer(s.request("GET", "/addr", A)).await.2, A);
        assert_eq!(answer(s.request("GET", "/addr", "")).await.2, "127.0.0.1");
    }
}
