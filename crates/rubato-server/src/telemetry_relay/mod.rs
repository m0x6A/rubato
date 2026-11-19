//! Slice `telemetry_relay` (ADR 0082): the web's own client spans, posted to its own server,
//! rewritten to `service.name=web` with only what the server lets through, and forwarded to the
//! server's OTLP traces endpoint, so the service graph draws `web → rubato-server`. Owns no
//! table; its token buckets live in memory.

mod handler;
mod rewrite;

pub use handler::{relay, take_token};
pub use rewrite::{Invalid, Rewritten, is_route, rewrite};

#[cfg(test)]
mod tests;
