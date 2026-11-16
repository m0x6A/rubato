//! Slice `workspace_settings`: the settings that describe each workspace's application, kept in
//! the server's database and set at runtime (ADR 0075). Owns table `workspace_settings`.
//!
//! The first start seeds the table, in one transaction, from `RUBATO_WORKSPACES_FILE` (or
//! `RUBATO_MAP_*`) and the `RUBATO_TRAFFIC_*` / `RUBATO_DEFAULT_*` variables; after that the
//! database owns them and every start warns while those are still set. The slice fills the
//! kernel's workspace registry, which every other slice reads; it alone writes it. Application
//! settings never come here.

mod seed;
mod store;

pub use seed::{Drift, Started, start};

#[cfg(test)]
mod tests;
