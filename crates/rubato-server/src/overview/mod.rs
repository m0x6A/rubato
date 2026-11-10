//! Slice `overview`: progress per issue, derived on read and never stored; findings; the gauges
//! that summarise them. Reads through the owning slices; owns no table.

mod build;
mod gauges;
mod handler;
mod inputs;

pub use build::{Built, OverviewInputs, build, counts};
pub use gauges::refresh_gauges;
pub use handler::overview;
pub use inputs::build_overview_inputs;
