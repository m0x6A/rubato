//! Slice `companion_link`: the companion websocket (ADR 0012). Bearer token, envelopes with
//! sequence numbers, acks, launch commands out. Owns tables `companion_events`,
//! `session_snapshots` and `stale_events`.

mod store;
mod ws;

pub use store::{all_sessions, session_snapshots};
pub use ws::{keep_members_sessions, mark_quiet, upgrade};
