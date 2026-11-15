//! The approval hash of a plan file (ADR 0027, ADR 0054) lives in `rubato-protocol`
//! (`plan_hash`), where the companion can use it too; the server re-exports it.

pub use rubato_protocol::{approval_hash, approval_names as names};
