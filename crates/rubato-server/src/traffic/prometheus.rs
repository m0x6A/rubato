//! The production traffic source (ADR 0040): the OpenTelemetry servicegraph connector's request
//! and failure totals, read through any Prometheus-compatible query API. Everything specific to
//! that stack lives here and nowhere else: the query language, the metric prefix, the label
//! names, the credentials. The rest of the server only sees a `TrafficReport`.

use super::source::{Source, SourceError};
use chrono::{DateTime, TimeZone, Utc};
use rubato_protocol::traffic::{LOOKBACK_SECS, MAX_EDGES, MAX_WINDOW_SECS, MIN_WINDOW_SECS};
use rubato_protocol::{SeenEdge, TrafficEdge, TrafficReport};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// The largest answer read from the query API; more is a `parse` rejection.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// The adapter's own settings, read from `RUBATO_TRAFFIC_PROMETHEUS_*` and
/// `RUBATO_TRAFFIC_WINDOW_SECS`. Headers are never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct PrometheusConfig {
    /// Base URL of the query API, without a trailing slash.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
    pub metric_prefix: String,
    pub client_namespace_label: String,
    pub server_namespace_label: String,
    /// The service namespaces this workspace reads (issue #288): a call is kept when its client
    /// or its server is in one of them. Empty: every namespace.
    pub namespaces: Vec<String>,
    pub window_secs: u32,
}

impl std::fmt::Debug for PrometheusConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrometheusConfig")
            .field("url", &self.url)
            .field(
                "headers",
                &self.headers.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .field("metric_prefix", &self.metric_prefix)
            .field("client_namespace_label", &self.client_namespace_label)
            .field("server_namespace_label", &self.server_namespace_label)
            .field("namespaces", &self.namespaces)
            .field("window_secs", &self.window_secs)
            .finish()
    }
}

fn is_label(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_metric_prefix(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_' || f == ':')
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// How many namespaces a source may list, and how long one may be.
pub const MAX_NAMESPACES: usize = 20;
const MAX_NAMESPACE_LEN: usize = 63;

/// A `service.namespace` a source may list: letters, digits, `.`, `_` and `-`. Narrow on
/// purpose, so the value goes into the query's matcher with only the dot to escape.
fn is_namespace(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAMESPACE_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The adapter's name in a workspace's settings (ADR 0075).
pub const SOURCE: &str = "prometheus";

/// Each text field of a workspace's Prometheus source and the variable that seeds it once
/// (ADR 0075). Only this adapter knows what they mean (ADR 0040).
pub const FIELDS: [(&str, &str); 6] = [
    ("url", "RUBATO_TRAFFIC_PROMETHEUS_URL"),
    (
        "namespace_labels",
        "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS",
    ),
    ("metric_prefix", "RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX"),
    ("namespaces", "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES"),
    ("timeout_secs", "RUBATO_TRAFFIC_PROMETHEUS_TIMEOUT_SECS"),
    ("window_secs", "RUBATO_TRAFFIC_WINDOW_SECS"),
];

/// Each secret of a workspace's Prometheus source and the variable that seeds it once.
pub const SECRETS: [(&str, &str); 1] = [("headers", "RUBATO_TRAFFIC_PROMETHEUS_HEADERS")];

/// A traffic source as the environment describes it, for the first start's seed (ADR 0075):
/// the adapter's name, its fields and its secrets (in the clear: the caller seals them).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Seed {
    pub source: String,
    pub fields: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, String>,
}

impl std::fmt::Debug for Seed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seed")
            .field("source", &self.source)
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// The Prometheus source the environment sets, as a workspace's fields and secrets, or `None`
/// when `RUBATO_TRAFFIC_PROMETHEUS_URL` is unset. Only variables that are set are copied.
pub fn seed_from_env(get: impl Fn(&str) -> Option<String>) -> Option<Seed> {
    get("RUBATO_TRAFFIC_PROMETHEUS_URL")?;
    let pick = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .filter_map(|(field, var)| get(var).map(|v| ((*field).to_owned(), v)))
            .collect()
    };
    Some(Seed {
        source: SOURCE.to_owned(),
        fields: pick(&FIELDS),
        secrets: pick(&SECRETS),
    })
}

impl PrometheusConfig {
    /// A workspace's source from its fields and opened secrets (ADR 0075), checked by the same
    /// rules as the environment was: each field is read as the variable it was seeded from, so
    /// there is one validator. `fetch_secs` bounds the timeout as in `from_env`.
    pub fn from_fields(
        fields: &BTreeMap<String, String>,
        secrets: &BTreeMap<String, String>,
        fetch_secs: u32,
    ) -> Result<Option<Self>, String> {
        let get = |var: &str| {
            FIELDS
                .iter()
                .find(|(_, v)| *v == var)
                .and_then(|(f, _)| fields.get(*f).cloned())
                .or_else(|| {
                    SECRETS
                        .iter()
                        .find(|(_, v)| *v == var)
                        .and_then(|(f, _)| secrets.get(*f).cloned())
                })
        };
        Self::from_env(get, fetch_secs)
    }

    /// The settings from `get` (the environment in production), or `None` when
    /// `RUBATO_TRAFFIC_PROMETHEUS_URL` is unset. `fetch_secs` bounds the query timeout: both
    /// queries of a fetch fit inside the interval they belong to.
    pub fn from_env(
        get: impl Fn(&str) -> Option<String>,
        fetch_secs: u32,
    ) -> Result<Option<Self>, String> {
        let Some(url) = get("RUBATO_TRAFFIC_PROMETHEUS_URL") else {
            return Ok(None);
        };
        let parsed = url::Url::parse(&url)
            .map_err(|e| format!("RUBATO_TRAFFIC_PROMETHEUS_URL is not a URL: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("RUBATO_TRAFFIC_PROMETHEUS_URL must be http or https".into());
        }
        let mut headers = Vec::new();
        for pair in get("RUBATO_TRAFFIC_PROMETHEUS_HEADERS")
            .unwrap_or_default()
            .split(',')
            .filter(|p| !p.trim().is_empty())
        {
            let Some((k, v)) = pair.split_once('=') else {
                // The value may be a secret: name only the position.
                return Err("RUBATO_TRAFFIC_PROMETHEUS_HEADERS: every entry is name=value".into());
            };
            let k = k.trim();
            if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err("RUBATO_TRAFFIC_PROMETHEUS_HEADERS: a header name is not valid".into());
            }
            headers.push((k.to_owned(), v.trim().to_owned()));
        }
        let number = |name: &str, default: u32| -> Result<u32, String> {
            match get(name) {
                None => Ok(default),
                Some(v) => v
                    .parse::<u32>()
                    .map_err(|_| format!("{name}={v:?} is not a number")),
            }
        };
        let window_secs = number("RUBATO_TRAFFIC_WINDOW_SECS", 3600)?;
        if !(MIN_WINDOW_SECS..=MAX_WINDOW_SECS).contains(&window_secs) {
            return Err(format!(
                "RUBATO_TRAFFIC_WINDOW_SECS={window_secs} is outside {MIN_WINDOW_SECS}–{MAX_WINDOW_SECS}"
            ));
        }
        let timeout_secs = number("RUBATO_TRAFFIC_PROMETHEUS_TIMEOUT_SECS", 10)?
            .clamp(1, (fetch_secs.saturating_sub(1) / 2).max(1));
        let metric_prefix = get("RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX")
            .unwrap_or_else(|| "traces_service_graph_".into());
        if !is_metric_prefix(&metric_prefix) {
            return Err(format!(
                "RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX={metric_prefix:?} is not a metric name prefix"
            ));
        }
        let labels = get("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS")
            .unwrap_or_else(|| "client_service_namespace,server_service_namespace".into());
        let (cl, sl) = labels
            .split_once(',')
            .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
            .filter(|(a, b)| is_label(a) && is_label(b) && a != b)
            .ok_or_else(|| {
                format!(
                    "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS={labels:?} must be two different label names, client first"
                )
            })?;
        let listed = get("RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES").unwrap_or_default();
        let mut namespaces: Vec<String> = Vec::new();
        for ns in listed.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if !is_namespace(ns) {
                return Err(format!(
                    "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES: {ns:?} is not a namespace (letters, digits, '.', '_' and '-', at most {MAX_NAMESPACE_LEN})"
                ));
            }
            if !namespaces.iter().any(|n| n == ns) {
                namespaces.push(ns.to_owned());
            }
        }
        if namespaces.len() > MAX_NAMESPACES {
            return Err(format!(
                "RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES lists {} namespaces; at most {MAX_NAMESPACES}",
                namespaces.len()
            ));
        }
        Ok(Some(Self {
            url: url.trim_end_matches('/').to_owned(),
            headers,
            timeout: Duration::from_secs(u64::from(timeout_secs)),
            metric_prefix,
            client_namespace_label: cl,
            server_namespace_label: sl,
            namespaces,
            window_secs,
        }))
    }

    /// The instant query for one metric: calls per pair of services over the window, the
    /// largest first, one more than a report may hold so a cut is seen. With namespaces listed
    /// (issue #288), only series whose client or server is in one of them: `or` keeps a series
    /// that matches both once.
    pub fn query(&self, metric: &str) -> String {
        self.query_over(metric, self.window_secs)
    }

    /// The same query over `secs` instead of the window: a lookback (ADR 0083).
    pub fn query_over(&self, metric: &str, secs: u32) -> String {
        let increase = |matcher: &str| {
            format!(
                "increase({}{}{}[{}s])",
                self.metric_prefix, metric, matcher, secs
            )
        };
        let series = if self.namespaces.is_empty() {
            increase("")
        } else {
            // `is_namespace` leaves the dot as the only regex character; the matcher is anchored.
            let any = self
                .namespaces
                .iter()
                .map(|n| n.replace('.', "\\\\."))
                .collect::<Vec<_>>()
                .join("|");
            format!(
                "{} or {}",
                increase(&format!("{{{}=~\"{any}\"}}", self.client_namespace_label)),
                increase(&format!("{{{}=~\"{any}\"}}", self.server_namespace_label))
            )
        };
        format!(
            "topk({}, sum by (client, server, {}, {}) ({series}))",
            MAX_EDGES + 1,
            self.client_namespace_label,
            self.server_namespace_label,
        )
    }
}

#[derive(Deserialize)]
struct Answer {
    status: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<Sample>,
}

#[derive(Deserialize)]
struct Sample {
    metric: HashMap<String, String>,
    /// `[unix seconds, "value"]`.
    value: (f64, String),
}

/// `(client, client namespace, server, server namespace)`.
type Key = (String, Option<String>, String, Option<String>);

/// A count from a sample value: rounded; NaN, infinite or negative is 0.
fn count(v: &str) -> u64 {
    match v.parse::<f64>() {
        Ok(f) if f.is_finite() && f > 0.0 => f.round() as u64,
        _ => 0,
    }
}

/// The source of ADR 0040.
#[derive(Debug, Clone)]
pub struct PrometheusSource {
    config: PrometheusConfig,
    http: reqwest::Client,
    /// Every connect is checked against `guard::refused` (ADR 0075); off for a local server
    /// and for the adapter's own tests against a fake on loopback.
    guarded: bool,
}

impl PrometheusSource {
    /// Unguarded: the adapter's tests, against a fake query API on loopback.
    pub fn new(config: PrometheusConfig) -> Self {
        Self::build(config, false)
    }

    /// A workspace's source (ADR 0075): every address it connects to is checked, unless the
    /// server is itself local. No proxy from the environment and no redirects either way, so
    /// the address checked is the one connected to.
    pub fn guarded(config: PrometheusConfig, server_is_local: bool) -> Self {
        Self::build(config, !server_is_local)
    }

    fn build(config: PrometheusConfig, guarded: bool) -> Self {
        let mut b = reqwest::Client::builder()
            .timeout(config.timeout)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        if guarded {
            b = b.dns_resolver(std::sync::Arc::new(super::guard::GuardedResolver));
        }
        Self {
            http: b.build().unwrap_or_default(),
            config,
            guarded,
        }
    }

    /// One instant query over the window, the samples keyed by their pair of services.
    async fn samples(
        &self,
        metric: &str,
        at: DateTime<Utc>,
    ) -> Result<BTreeMap<Key, u64>, SourceError> {
        self.samples_over(metric, self.config.window_secs, at).await
    }

    /// One instant query over `secs`.
    async fn samples_over(
        &self,
        metric: &str,
        secs: u32,
        at: DateTime<Utc>,
    ) -> Result<BTreeMap<Key, u64>, SourceError> {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("query", &self.config.query_over(metric, secs))
            .append_pair("time", &at.timestamp().to_string())
            .finish();
        let mut req = self
            .http
            .post(format!("{}/api/v1/query", self.config.url))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form);
        for (k, v) in &self.config.headers {
            req = req.header(k, v);
        }
        let failed = |e: String| SourceError::Failed(format!("prometheus {metric}: {e}"));
        // An IP literal never reaches the resolver: checked here (ADR 0075).
        if self.guarded && super::guard::literal_refused(&self.config.url) {
            return Err(failed(super::guard::REFUSED.into()));
        }
        let mut res = req.send().await.map_err(|e| {
            // The guard's refusal is deep in the error's sources (ADR 0075): say it as itself.
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                if s.to_string() == super::guard::REFUSED {
                    return failed(super::guard::REFUSED.into());
                }
                source = s.source();
            }
            // reqwest's message may carry the URL, never the headers.
            failed(if e.is_timeout() {
                "timed out".into()
            } else {
                e.without_url().to_string()
            })
        })?;
        let status = res.status();
        let mut body = Vec::new();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| failed(e.without_url().to_string()))?
        {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(SourceError::Parse(format!(
                    "prometheus {metric}: answer over {MAX_BODY_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        if status.is_redirection() {
            return Err(failed(format!(
                "HTTP {}: redirects are not followed (ADR 0075)",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(failed(format!("HTTP {}", status.as_u16())));
        }
        let answer: Answer = serde_json::from_slice(&body)
            .map_err(|e| SourceError::Parse(format!("prometheus {metric}: {e}")))?;
        if answer.status != "success" {
            return Err(failed(answer.error.unwrap_or(answer.status)));
        }
        let data = answer
            .data
            .ok_or_else(|| SourceError::Parse(format!("prometheus {metric}: no data")))?;
        if data.result_type != "vector" {
            return Err(SourceError::Parse(format!(
                "prometheus {metric}: result is a {}, not a vector",
                data.result_type
            )));
        }
        let ns = |m: &HashMap<String, String>, label: &str| {
            m.get(label).filter(|s| !s.is_empty()).cloned()
        };
        let mut out = BTreeMap::new();
        for s in data.result {
            let (Some(client), Some(server)) = (s.metric.get("client"), s.metric.get("server"))
            else {
                continue;
            };
            let key = (
                client.clone(),
                ns(&s.metric, &self.config.client_namespace_label),
                server.clone(),
                ns(&s.metric, &self.config.server_namespace_label),
            );
            let n = out.entry(key).or_insert(0u64);
            *n = n.saturating_add(count(&s.value.1));
        }
        Ok(out)
    }

    /// The report at `at`: requests per pair, failures joined on the same four labels and
    /// capped at the requests, cut at `MAX_EDGES` with `truncated`.
    pub async fn report_at(&self, at: DateTime<Utc>) -> Result<TrafficReport, SourceError> {
        let requests = self.samples("request_total", at).await?;
        let failed = self.samples("request_failed_total", at).await?;
        let mut edges: Vec<TrafficEdge> = requests
            .into_iter()
            .filter(|(_, calls)| *calls > 0)
            .map(|(key, calls)| {
                let errors = failed.get(&key).copied().unwrap_or(0).min(calls);
                let (client, client_namespace, server, server_namespace) = key;
                TrafficEdge {
                    client,
                    server,
                    calls,
                    errors,
                    client_namespace,
                    server_namespace,
                }
            })
            .collect();
        edges.sort_by_key(|e| std::cmp::Reverse(e.calls));
        let truncated = edges.len() > MAX_EDGES;
        edges.truncate(MAX_EDGES);
        Ok(TrafficReport {
            observed_at: Utc.timestamp_opt(at.timestamp(), 0).single().unwrap_or(at),
            window_secs: self.config.window_secs,
            edges,
            truncated,
            // The lookback task fills these (ADR 0083); the window fetch never does.
            seen: Vec::new(),
            lookback_secs: None,
        })
    }
}

impl PrometheusSource {
    /// The lookbacks at `at` (ADR 0083): the request totals over each lookback longer than the
    /// window, shortest first. Every edge with calls in any of them, in the window or not, with
    /// the shortest one; cut at `MAX_EDGES`, busiest lookback first.
    pub async fn lookback_at(
        &self,
        at: DateTime<Utc>,
    ) -> Result<Option<super::source::Lookback>, SourceError> {
        let spans: Vec<u32> = LOOKBACK_SECS
            .into_iter()
            .filter(|s| *s > self.config.window_secs)
            .collect();
        let Some(&longest) = spans.last() else {
            return Ok(None);
        };
        let mut first: BTreeMap<Key, u32> = BTreeMap::new();
        for secs in spans {
            for (key, calls) in self.samples_over("request_total", secs, at).await? {
                if calls > 0 {
                    first.entry(key).or_insert(secs);
                }
            }
        }
        let mut seen: Vec<SeenEdge> = first
            .into_iter()
            .map(
                |((client, client_namespace, server, server_namespace), within_secs)| SeenEdge {
                    client,
                    server,
                    client_namespace,
                    server_namespace,
                    within_secs,
                },
            )
            .collect();
        seen.sort_by_key(|e| e.within_secs);
        seen.truncate(MAX_EDGES);
        Ok(Some(super::source::Lookback {
            seen,
            lookback_secs: longest,
        }))
    }
}

impl Source for PrometheusSource {
    fn name(&self) -> &'static str {
        "prometheus"
    }

    async fn fetch(&self) -> Result<TrafficReport, SourceError> {
        self.report_at(Utc::now()).await
    }

    async fn lookback(&self) -> Result<Option<super::source::Lookback>, SourceError> {
        self.lookback_at(Utc::now()).await
    }
}
