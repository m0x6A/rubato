//! Slice `traffic`: observed calls per seam, from the latest report of a traffic source
//! (ADR 0039). The source is a port; the one source so far is the development file
//! `RUBATO_DEV_TRAFFIC` (plan #104, Q1 (d)). The latest report is kept in memory in
//! `AppState::traffic`, never in SQLite; the overview derives `Overview.traffic` from it. Owns no
//! table.
//!
//! Service names in a report come from outside the team. They are text only: never ids, class
//! names or URLs, and they never become map parts.

mod fetch;
mod source;

pub use fetch::{Fetcher, Outcome, run};
pub use source::{DevFileSource, Source, SourceError};

#[cfg(test)]
mod tests;
