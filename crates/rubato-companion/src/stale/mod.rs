//! Slice `stale`: never silent. The heartbeat, and *working* sessions with no signal for longer
//! than the stale limit (`rubato.session.stale`).

mod heartbeat;
mod sweep;

pub use heartbeat::heartbeat;
pub use sweep::sweep;
