//! Slice `traffic`: observed calls per seam, from the latest report of a traffic source
//! (ADR 0039). The source is a port with two adapters: the development file
//! `RUBATO_DEV_TRAFFIC` (plan #104) and the OTel service graph over a Prometheus-compatible
//! query API, `RUBATO_TRAFFIC_PROMETHEUS_URL` (ADR 0040). The latest report is kept in memory in
//! `AppState::traffic`, never in SQLite; the overview derives `Overview.traffic` from it. Owns
//! table `traffic_dismissals`: what a person dismissed on the overlay, per workspace (ADR 0060).
//!
//! Service names in a report come from outside the team. They are text only: never ids, class
//! names or URLs, and they never become map parts.

mod dismiss;
mod fetch;
mod prometheus;
mod source;

pub use dismiss::{dismiss, list as dismissals, undismiss};
pub use fetch::{Fetcher, Outcome, run};
pub use prometheus::{PrometheusConfig, PrometheusSource};
pub use source::{ConfiguredSource, DevFileSource, Source, SourceError};

#[cfg(test)]
mod dismiss_tests;
#[cfg(test)]
mod prometheus_tests;
