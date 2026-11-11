//! Slice `queue`: the issues a developer intends to run on one of their own companions, in
//! order (ADR 0020). Everyone sees every queue; only the companion's paired user writes it.
//! Stored intent, never a progress state. Owns table `companion_queue`.

mod handlers;
mod store;

pub use handlers::{add, record_depth, remove, reorder};
pub use store::{all, launch_not_started, launch_started};
