//! The spend a transcript adds up to, recorded on the session.

use crate::sessions::Registry;

impl Registry {
    pub fn set_spend(&mut self, session_id: &str, usd: f64) {
        if let Some(e) = self.find_by_session(session_id) {
            e.spend_usd = Some(usd);
        }
    }
}
