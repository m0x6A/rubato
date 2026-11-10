//! Slice `server_link`: the outbound websocket to the server (ADR 0012). Envelopes with sequence
//! numbers, an outbox replayed after reconnect, acks both ways.

mod client;
mod welcome;

pub use client::{Client, Identity, ws_url};
pub use welcome::on_welcome;
