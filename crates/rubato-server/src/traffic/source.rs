//! The traffic source port, and the development file that fills it.

use chrono::{DateTime, Utc};
use rubato_protocol::{SeenEdge, TrafficReport};
use std::path::PathBuf;

/// Why a fetch gave no report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The source did not answer: unreadable, timed out, refused. The state turns `failing`.
    #[error("{0}")]
    Failed(String),
    /// The source answered with something that is not a report. Rejected whole, reason `parse`.
    #[error("{0}")]
    Parse(String),
}

/// What a lookback saw (ADR 0083): every edge with calls in any lookback, each with the shortest
/// one, and the longest lookback asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookback {
    pub seen: Vec<SeenEdge>,
    pub lookback_secs: u32,
}

/// Anything that can produce a `TrafficReport` (ADR 0039). One fetch per interval. Each backend
/// is one adapter behind this port (ADR 0040).
pub trait Source: Send + Sync {
    /// The adapter, as `rubato.traffic.source` names it.
    fn name(&self) -> &'static str;
    fn fetch(&self) -> impl Future<Output = Result<TrafficReport, SourceError>> + Send;
    /// When each edge was last seen, over lookbacks longer than the window (ADR 0083). `None`
    /// when the source has no such thing: then nothing is claimed about quiet parts.
    fn lookback(&self) -> impl Future<Output = Result<Option<Lookback>, SourceError>> + Send {
        async { Ok(None) }
    }
}

/// The source config selects: one adapter or the other, never both (ADR 0040).
#[derive(Debug, Clone)]
pub enum ConfiguredSource {
    DevFile(DevFileSource),
    Prometheus(super::prometheus::PrometheusSource),
}

impl Source for ConfiguredSource {
    fn name(&self) -> &'static str {
        match self {
            ConfiguredSource::DevFile(s) => s.name(),
            ConfiguredSource::Prometheus(s) => s.name(),
        }
    }

    async fn fetch(&self) -> Result<TrafficReport, SourceError> {
        match self {
            ConfiguredSource::DevFile(s) => s.fetch().await,
            ConfiguredSource::Prometheus(s) => s.fetch().await,
        }
    }

    async fn lookback(&self) -> Result<Option<Lookback>, SourceError> {
        match self {
            // The development file carries `seen` and `lookback_secs` in itself, if any.
            ConfiguredSource::DevFile(_) => Ok(None),
            ConfiguredSource::Prometheus(s) => s.lookback().await,
        }
    }
}

/// `RUBATO_DEV_TRAFFIC`: a JSON `TrafficReport` on disk. `observed_at` may be left out, and is
/// then the file's modification time, so a file nobody rewrites goes stale. Development and
/// tests only.
#[derive(Debug, Clone)]
pub struct DevFileSource {
    path: PathBuf,
}

impl DevFileSource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Source for DevFileSource {
    fn name(&self) -> &'static str {
        "dev_file"
    }

    async fn fetch(&self) -> Result<TrafficReport, SourceError> {
        let failed = |e: std::io::Error| SourceError::Failed(format!("dev traffic file: {e}"));
        let text = tokio::fs::read_to_string(&self.path)
            .await
            .map_err(failed)?;
        let modified: DateTime<Utc> = tokio::fs::metadata(&self.path)
            .await
            .and_then(|m| m.modified())
            .map_err(failed)?
            .into();
        let mut value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| SourceError::Parse(e.to_string()))?;
        if let Some(obj) = value.as_object_mut() {
            obj.entry("observed_at")
                .or_insert_with(|| serde_json::Value::String(modified.to_rfc3339()));
        }
        serde_json::from_value(value).map_err(|e| SourceError::Parse(e.to_string()))
    }
}
