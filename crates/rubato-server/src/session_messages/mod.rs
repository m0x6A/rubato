//! Slice `session_messages`: messages a person sends from Rubato to a session they launched,
//! queued here and typed by their own companion when the session waits for input (ADR 0051,
//! plan 122 design §D1). Owns table `session_messages`. At most once: a message that was pushed
//! and went unanswered while its companion stayed live is *delivery unknown*, never pushed again.

mod dispatch;
mod handlers;
mod store;

#[cfg(test)]
mod tests;

pub use dispatch::{
    dispatch, on_hello, on_hook, on_result, on_sessions, record_gauges, run_sweep, sweep,
};
pub use handlers::{list, send};
