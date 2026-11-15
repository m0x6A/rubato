//! The production traffic source (ADR 0040): the OpenTelemetry servicegraph connector's request
//! and failure totals, read through any Prometheus-compatible query API. Everything specific to
//! that stack lives here and nowhere else: the query language, the metric prefix, the label
//! names, the credentials. The rest of the server only sees a `TrafficReport`.

use super::source::{Source, SourceError};
use chrono::{DateTime, TimeZone, Utc};
use rubato_protocol::traffic::{MAX_EDGES, MAX_WINDOW_SECS, MIN_WINDOW_SECS};
use rubato_protocol::{TrafficEdge, TrafficReport};
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

impl PrometheusConfig {
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
        Ok(Some(Self {
            url: url.trim_end_matches('/').to_owned(),
            headers,
            timeout: Duration::from_secs(u64::from(timeout_secs)),
            metric_prefix,
            client_namespace_label: cl,
            server_namespace_label: sl,
            window_secs,
        }))
    }

    /// The instant query for one metric: calls per pair of services over the window, the
    /// largest first, one more than a report may hold so a cut is seen.
    pub fn query(&self, metric: &str) -> String {
        format!(
            "topk({}, sum by (client, server, {}, {}) (increase({}{}[{}s])))",
            MAX_EDGES + 1,
            self.client_namespace_label,
            self.server_namespace_label,
            self.metric_prefix,
            metric,
            self.window_secs
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
}

impl PrometheusSource {
    pub fn new(config: PrometheusConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// One instant query, the samples keyed by their pair of services.
    async fn samples(
        &self,
        metric: &str,
        at: DateTime<Utc>,
    ) -> Result<BTreeMap<Key, u64>, SourceError> {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("query", &self.config.query(metric))
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
        let mut res = req.send().await.map_err(|e| {
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
        })
    }
}

impl Source for PrometheusSource {
    fn name(&self) -> &'static str {
        "prometheus"
    }

    async fn fetch(&self) -> Result<TrafficReport, SourceError> {
        self.report_at(Utc::now()).await
    }
}
