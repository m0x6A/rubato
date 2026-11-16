//! Slice `agent_reports`: what each launched session's agent last said it is doing (ADR 0071,
//! plan 226 §2). A signal shown with its age, never a progress state: `overview` reads it only
//! to fill `Overview.agent_lines`, and `derive_progress` never sees it. Owns table
//! `agent_reports`. Names no other slice.

mod store;

pub use store::{all, prune, prune_absent, record};
