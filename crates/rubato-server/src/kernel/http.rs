//! The one way a handler turns a store error into a status: logged with the slice that failed.

use axum::http::StatusCode;

/// Log `error!(slice, error, "request failed")` and answer 500.
pub fn internal(slice: &'static str, e: impl std::fmt::Display) -> StatusCode {
    tracing::error!(slice, error = %e, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    #[test]
    fn internal_logs_slice_and_error() {
        let (events, _guard) = capture::start();
        assert_eq!(
            internal("pairing", "disk full"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let e = &events[0];
        assert_eq!(e.level, tracing::Level::ERROR);
        assert_eq!(e.message, "request failed");
        assert_eq!(e.fields.get("slice").map(String::as_str), Some("pairing"));
        assert_eq!(e.fields.get("error").map(String::as_str), Some("disk full"));
    }
}
