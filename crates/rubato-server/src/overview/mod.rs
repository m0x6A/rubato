//! Slice `overview`: progress per issue, derived on read and never stored; findings; the gauges
//! that summarise them. Reads through the owning slices; owns no table.

mod build;
mod gauges;
#[cfg(test)]
mod golden_tests;
mod handler;
mod inputs;
mod issue;
#[cfg(test)]
mod traffic_tests;

pub use build::{Built, OverviewInputs, build, counts};
pub use gauges::{Refreshed, refresh, refresh_gauges};
pub use handler::overview;
pub use inputs::build_overview_inputs;
pub use issue::issue_summary;
