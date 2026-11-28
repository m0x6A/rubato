//! How long a session counts (ADR 0064). Who may reach what is the tenant memberships now
//! (ADR 0088, superseding ADR 0022), the context graph's snapshot route included (design 345
//! §6.4): a login is recorded on the session with its time, and only housekeeping reads it.

use chrono::{DateTime, Duration, Utc};

/// A team record older than this sends the person through login again.
pub const TEAM_RECORD_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// A web session counts for this long from its creation, in every mode: the session cookie's own
/// `Max-Age` (ADR 0064). Without the App it is the only limit on the server.
pub const SESSION_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;

/// Reasons a session or login is refused, as `rubato.reason` on `rubato.web.rejected`.
pub mod reason {
    pub const SESSION_EXPIRED: &str = "session_expired";
}

/// Whether a session created at `created_at` still counts at `now` (ADR 0064).
pub fn session_is_current(created_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - created_at <= Duration::seconds(SESSION_MAX_AGE_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_counts_for_thirty_days() {
        let now = Utc::now();
        assert!(session_is_current(now, now));
        assert!(session_is_current(now - Duration::days(29), now));
        assert!(!session_is_current(now - Duration::days(31), now));
    }
}
