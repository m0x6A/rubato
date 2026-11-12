//! Slice `plans`: parse plan files (ADR 0009) fetched from issue branches and keep the latest
//! per issue. Owns table `plans`.

mod fetch;
mod hash;
mod parse;
mod store;

pub use fetch::{fetch_and_store, hash_on_branch, plan_path};
pub use hash::{approval_hash, names as approval_names};
pub use parse::{PlanParseError, parse_plan};
pub use store::{PlanRow, all_plans as all, has_plan};
