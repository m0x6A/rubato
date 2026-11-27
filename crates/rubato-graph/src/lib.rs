//! The context graph's store (ADR 0084, ADR 0086). One SQLite file holds every source; a source
//! is replaced whole, in one transaction, and never patched. `rubato-server` builds sources into
//! its file and serves snapshots of them; `rubato-companion` applies snapshots to its copy and
//! answers the graph tools from it. Both use this crate, so there is one schema, one replace,
//! one code path that writes a snapshot and computes its hash, and one set of queries.
//!
//! The wire types are `rubato_protocol::graph`. This crate holds no slice and names neither
//! binary.

#[doc(hidden)]
pub mod fixture;
mod query;
mod schema;
mod snapshot;
mod store;

pub use query::{About, Context, Direction, Related, SeamContext};
pub use schema::USER_VERSION;
pub use snapshot::{Snapshot, content_hash, edge_line, node_line};
pub use store::{Build, Replaced, SourceRow, Store};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Db(#[from] sqlx::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// A snapshot that must not be applied: a wrong format, a hash that does not match, a body
    /// over the cap.
    #[error("snapshot: {0}")]
    Snapshot(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests;
