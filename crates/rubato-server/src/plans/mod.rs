//! Slice `plans`: parse plan files (ADR 0009) fetched from issue branches and keep the latest
//! per issue. Owns table `plans`.

mod fetch;
mod parse;
mod store;

pub use fetch::fetch_and_store;
pub use parse::{PlanParseError, parse_plan};
pub use store::{PlanRow, all_plans as all};
