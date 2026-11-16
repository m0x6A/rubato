//! The server's `Welcome` carries the limits this companion runs with and what the server can
//! take (ADR 0051). Kept until the next Welcome: an answer the server did not announce it takes
//! is not sent, since an older server drops what it cannot decode.

use rubato_protocol::Limits;
use tokio::sync::watch;

/// Publish the server's limits and capabilities. Limit readers wake only when the limits differ
/// from the current ones, so a reconnect's identical `Welcome` does not restart every timer.
pub fn on_welcome(
    limits: Limits,
    capabilities: Vec<String>,
    current: &watch::Sender<Limits>,
    server_capabilities: &std::sync::Mutex<Vec<String>>,
) {
    current.send_if_modified(|l| {
        let changed = *l != limits;
        *l = limits;
        changed
    });
    tracing::info!(capabilities = ?capabilities, "server capabilities");
    *server_capabilities.lock().unwrap() = capabilities;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::capability;

    #[test]
    fn each_welcome_replaces_the_limits_and_the_capabilities() {
        let (limits, mut rx) = watch::channel(Limits::default());
        let caps = std::sync::Mutex::new(Vec::new());
        let short = Limits {
            session_message_max_chars: 10,
            ..Limits::default()
        };
        on_welcome(
            short,
            vec![capability::SESSION_MESSAGES.into()],
            &limits,
            &caps,
        );
        assert!(rx.has_changed().unwrap(), "new limits wake the readers");
        assert_eq!(rx.borrow_and_update().session_message_max_chars, 10);
        assert_eq!(*caps.lock().unwrap(), [capability::SESSION_MESSAGES]);
        // A reconnect's identical Welcome wakes nobody; an older server's has no capabilities.
        on_welcome(short, Vec::new(), &limits, &caps);
        assert!(!rx.has_changed().unwrap(), "the same limits wake nobody");
        assert!(caps.lock().unwrap().is_empty());
    }
}
