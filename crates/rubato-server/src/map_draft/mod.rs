//! Slice `map_draft`: the shared draft of the system map and its proposal as a pull request
//! (ADR 0018). Owns tables `map_draft_ops` and `map_proposal`.

mod apply;
mod handlers;
mod poll;
mod propose;
mod store;
mod yaml;

pub use handlers::{add_op, discard, remove_op, restore};
pub use poll::{check_proposal, clear_merged, record_gauges};
pub use propose::{propose, proposed_yaml, withdraw};
pub use store::snapshot;
