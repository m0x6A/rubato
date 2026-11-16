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
mod fields;
mod guard;
mod prometheus;
mod source;
mod supervisor;

pub use dismiss::{dismiss, list as dismissals, undismiss};
pub use fetch::{Fetcher, Generations, Outcome, Targets};
pub use fields::{
    FieldRefusal, check as check_source, origin as source_origin, secret_names, sources,
};
pub use prometheus::{
    FIELDS as PROMETHEUS_FIELDS, PrometheusConfig, PrometheusSource, SECRETS as PROMETHEUS_SECRETS,
    Seed, seed_from_env,
};
pub use source::{ConfiguredSource, DevFileSource, Source, SourceError};
pub use supervisor::run;

#[cfg(test)]
mod dismiss_tests;
#[cfg(test)]
mod prometheus_tests;
