//! The server's `Welcome` carries the limits this companion runs with.

use rubato_protocol::Limits;

pub fn on_welcome(limits: Limits, current: &std::sync::Mutex<Limits>) {
    *current.lock().unwrap() = Limits { ..limits };
}
