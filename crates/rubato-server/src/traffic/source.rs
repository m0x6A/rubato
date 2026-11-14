//! The traffic source port, and the development file that fills it.

use chrono::{DateTime, Utc};
use rubato_protocol::TrafficReport;
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

/// Anything that can produce a `TrafficReport` (ADR 0039). One fetch per interval.
pub trait Source: Send + Sync {
    fn fetch(&self) -> impl Future<Output = Result<TrafficReport, SourceError>> + Send;
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
