//! The server's `Welcome` carries the limits this companion runs with.

use rubato_protocol::Limits;
use tokio::sync::watch;

/// Publish the server's limits. Readers wake only when they differ from the current ones, so a
/// reconnect's identical `Welcome` does not restart every timer.
pub fn on_welcome(limits: Limits, current: &watch::Sender<Limits>) {
    current.send_if_modified(|l| {
        let changed = *l != limits;
        *l = limits;
        changed
    });
}
