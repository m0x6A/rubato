//! The server's `Welcome` carries the limits this companion runs with and what the server can
//! take (ADR 0051). Kept until the next Welcome: an answer the server did not announce it takes
//! is not sent, since an older server drops what it cannot decode.

use rubato_protocol::Limits;

pub fn on_welcome(
    limits: Limits,
    capabilities: Vec<String>,
    current: &std::sync::Mutex<Limits>,
    server_capabilities: &std::sync::Mutex<Vec<String>>,
) {
    *current.lock().unwrap() = Limits { ..limits };
    tracing::info!(capabilities = ?capabilities, "server capabilities");
    *server_capabilities.lock().unwrap() = capabilities;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rubato_protocol::capability;

    #[test]
    fn each_welcome_replaces_the_limits_and_the_capabilities() {
        let limits = std::sync::Mutex::new(Limits::default());
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
        assert_eq!(limits.lock().unwrap().session_message_max_chars, 10);
        assert_eq!(*caps.lock().unwrap(), [capability::SESSION_MESSAGES]);
        // An older server's Welcome has none.
        on_welcome(Limits::default(), Vec::new(), &limits, &caps);
        assert!(caps.lock().unwrap().is_empty());
    }
}
