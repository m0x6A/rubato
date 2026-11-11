//! Who is on the team (ADR 0022): a GitHub account that can access the installation of the
//! GitHub App the server acts as. Checked at login and recorded on the session with its time;
//! every request then needs that record, no older than `TEAM_RECORD_MAX_AGE_SECS`.

use chrono::{DateTime, Duration, Utc};

/// A team record older than this sends the person through login again.
pub const TEAM_RECORD_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

/// Reasons a session or login is refused, as `rubato.reason` on `rubato.web.rejected`.
pub mod reason {
    pub const NOT_ON_TEAM: &str = "not_on_team";
    pub const TEAM_RECORD_STALE: &str = "team_record_stale";
    pub const INSTALLATION_LOOKUP_FAILED: &str = "installation_lookup_failed";
}

/// Whether the answer of `GET /user/installations`, asked with the person's own token, lists
/// the installation the server acts as. That call returns only installations of this App the
/// person can access, so being listed is having access to its repositories.
pub fn member_of(answer: &serde_json::Value, installation_id: u64) -> bool {
    answer
        .get("installations")
        .and_then(|i| i.as_array())
        .is_some_and(|list| {
            list.iter()
                .any(|i| i.get("id").and_then(|id| id.as_u64()) == Some(installation_id))
        })
}

/// Whether a session's team record still counts at `now`. None never does.
pub fn record_is_fresh(checked_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    checked_at.is_some_and(|at| now - at <= Duration::seconds(TEAM_RECORD_MAX_AGE_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn member_only_when_the_installation_is_listed() {
        // Recorded shapes of `GET /user/installations`.
        let two = json!({"total_count": 2, "installations": [
            {"id": 11, "account": {"login": "someone-else"}},
            {"id": 34, "account": {"login": "m0x6A"}}
        ]});
        let none = json!({"total_count": 0, "installations": []});
        let classic_oauth_token = json!({
            "message": "You must authenticate with an access token authorized to a GitHub App",
            "status": "403"
        });
        assert!(member_of(&two, 34));
        assert!(!member_of(&two, 35), "another installation is not ours");
        assert!(!member_of(&none, 34));
        assert!(!member_of(&classic_oauth_token, 34));
        println!("checked 4 answers");
    }

    #[test]
    fn a_record_counts_for_seven_days() {
        let now = Utc::now();
        assert!(!record_is_fresh(None, now));
        assert!(record_is_fresh(Some(now), now));
        assert!(record_is_fresh(Some(now - Duration::days(6)), now));
        assert!(!record_is_fresh(Some(now - Duration::days(8)), now));
    }
}
