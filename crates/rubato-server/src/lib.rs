//! `rubato-server` as a library so integration tests build the same router the binary serves.
//!
//! Organised by vertical slices (ADR 0015): one module per capability, a `kernel` of leaf
//! infrastructure that names no slice, and a composition root (`lib.rs`, `app.rs`, `tasks.rs`)
//! that names every slice. Slices: `auth`, `pairing`, `companion_link`, `pins`, `launch`, `map`,
//! `plans`, `github_poll`, `overview`, `web_link`. `slices.rs` checks the layout.

pub mod app;
pub mod kernel;
pub mod tasks;

pub mod auth;
pub mod companion_link;
pub mod github_poll;
pub mod launch;
pub mod layout;
pub mod map;
pub mod map_draft;
pub mod new_issue;
pub mod overview;
pub mod pairing;
pub mod pins;
pub mod plans;
pub mod queue;
pub mod web_link;

#[cfg(test)]
mod slices;

pub use kernel::VERSION;
