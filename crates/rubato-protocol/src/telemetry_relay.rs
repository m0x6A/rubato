//! The web's span relay (ADR 0082): the browser posts its own client spans, as OTLP/JSON, to
//! its own server, which rewrites and forwards them. The body is the standard OTLP shape, not a
//! Rubato type, so only the route and the limits live here.

/// The largest post the relay reads, in bytes.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// The most spans one post may carry.
pub const MAX_SPANS: usize = 512;
/// The browser cuts a batch here, under the 64 KiB a `keepalive` request may carry.
pub const BROWSER_BATCH_BYTES: usize = 60 * 1024;
/// The browser flushes within this many milliseconds of its first buffered span, so both ends
/// of a call reach the service-graph generator within its pairing wait.
pub const FLUSH_MS: u32 = 1_000;
/// Per login: posts that may arrive at once, and milliseconds until one more may.
pub const BUCKET_BURST: u32 = 4;
pub const BUCKET_REFILL_MS: u32 = 2_000;
/// A span more than this many seconds from the server's clock is dropped.
pub const MAX_CLOCK_SKEW_SECS: i64 = 3_600;
/// The `service.name` every relayed span carries, whatever the browser sent.
pub const SERVICE_NAME: &str = "web";

pub mod routes {
    /// `POST`: an OTLP/JSON `ExportTraceServiceRequest` of the web's client spans. 202 when
    /// accepted, 404 when the server exports no traces.
    pub const TELEMETRY_TRACES: &str = "/api/telemetry/traces";
}
