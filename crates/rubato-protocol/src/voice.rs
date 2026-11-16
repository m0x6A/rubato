//! A plan room's voice channel (ADR 0073, plan 146): Rubato mints a LiveKit join token, takes one
//! push-to-talk utterance at a time into the room as text, and speaks an agent's reply to a
//! spoken question. Behind `RUBATO_VOICE`, off by default; every route answers 404 `voice_off`
//! while it is off. Rubato never starts or connects to LiveKit.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which room's voice channel to join (`POST` `routes::VOICE_TOKEN`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceTokenRequest {
    pub repo: String,
    pub number: u64,
    pub branch: String,
}

/// A LiveKit join token for one person and one plan room. `expires_at` bounds the join only:
/// LiveKit refreshes the token of a connected participant (ADR 0073).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceToken {
    /// The LiveKit server, `wss://…`.
    pub url: String,
    pub token: String,
    /// The LiveKit room: `rubato-` and 32 hex digits of the plan room's key.
    pub room: String,
    /// `<login>#<suffix>`, so two tabs of one person do not evict each other.
    pub identity: String,
    pub expires_at: DateTime<Utc>,
}

/// The query of an utterance (`POST` `routes::VOICE_UTTERANCE`); the audio is the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UtteranceQuery {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    /// Chosen by the browser; it becomes the room message's id, so a repeat posts once.
    pub id: Uuid,
    /// An agent's handle in the room: the transcript is posted as `@<to> <transcript>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// A failed join, reported by the browser (`POST` `routes::VOICE_JOIN_FAILED`): Rubato never sees
/// LiveKit, so this is the only signal for a LiveKit that is down or unreachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceJoinFailed {
    pub repo: String,
    pub number: u64,
    pub branch: String,
    /// One of `join_reason`; anything else is counted as `other`.
    pub reason: String,
}

/// Whether voice is on for this server, on `Me` only while it is (ADR 0073).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceAvailability {
    /// A voice channel can be joined: LiveKit is configured.
    pub enabled: bool,
    /// A speech provider is configured: utterances reach the room and spoken replies play.
    pub speech: bool,
    /// The provider's hosts, for the consent text; empty for the fake adapter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speech_hosts: Vec<String>,
}

/// The kinds of failed join a browser reports.
pub mod join_reason {
    /// The signalling WebSocket was refused or dropped.
    pub const SIGNAL: &str = "signal";
    /// No media path: UDP blocked and no TURN.
    pub const ICE: &str = "ice";
    /// The microphone was denied.
    pub const PERMISSION: &str = "permission";
    /// Not a secure context: the page is plain HTTP off localhost.
    pub const INSECURE: &str = "insecure";
    pub const OTHER: &str = "other";
    pub const ALL: [&str; 5] = [SIGNAL, ICE, PERMISSION, INSECURE, OTHER];

    /// A reported reason as counted: one of `ALL`, else `other`.
    pub fn normalise(r: &str) -> &'static str {
        ALL.iter().find(|a| **a == r).copied().unwrap_or(OTHER)
    }
}

/// Why a voice request was refused: the `reason` of its answer, and `rubato.reason` on
/// `rubato.web.rejected` under the voice route. `no_plan`, `empty` and `too_long` are the
/// room's own (`room::reason`).
pub mod reason {
    /// `RUBATO_VOICE` is off: every voice route answers 404.
    pub const VOICE_OFF: &str = "voice_off";
    /// No speech adapter configured: voice is for people only.
    pub const NO_SPEECH: &str = "no_speech";
    /// No LiveKit configured.
    pub const NO_LIVEKIT: &str = "no_livekit";
    /// `to` is not an agent present in the room.
    pub const UNKNOWN_AGENT: &str = "unknown_agent";
    /// The provider is at its concurrent-call limit.
    pub const BUSY: &str = "busy";
    /// The message is not an agent's reply to a spoken turn: it is never spoken.
    pub const NOT_SPOKEN: &str = "not_spoken";
    /// The provider failed (timeout, status, transport, bad body): 502.
    pub const PROVIDER: &str = "provider";
    /// An utterance id already used in another room, or by another person.
    pub const ID_REUSED: &str = "id_reused";
    /// A join-failure report over its size limit.
    pub const TOO_LARGE: &str = "too_large";
}

/// Bounds, fixed by plan 146 (A1–A5, A8).
pub mod limits {
    /// An utterance body, 2 MiB.
    pub const UTTERANCE_MAX_BYTES: usize = 2 * 1024 * 1024;
    /// Seconds of one utterance the browser records at most.
    pub const UTTERANCE_MAX_SECS: u64 = 60;
    /// A provider's response body, 4 MiB.
    pub const PROVIDER_BODY_MAX_BYTES: usize = 4 * 1024 * 1024;
    /// One provider call's deadline.
    pub const PROVIDER_DEADLINE_SECS: u64 = 30;
    /// Provider calls at once, STT and TTS together.
    pub const PROVIDER_CONCURRENCY: usize = 2;
    /// How long a call waits for a permit before it answers `busy`.
    pub const PROVIDER_QUEUE_SECS: u64 = 2;
    /// The spoken cut of a reply.
    pub const SPOKEN_CUT_MAX_CHARS: usize = 400;
    /// Clips kept in memory, and for how long.
    pub const CLIP_CACHE_ENTRIES: usize = 32;
    pub const CLIP_CACHE_SECS: i64 = 600;
    /// The join window of a token.
    pub const TOKEN_SECS: i64 = 600;
    /// A join-failure report's body.
    pub const JOIN_FAILED_MAX_BYTES: usize = 1024;
}

/// Team routes, like the room's: never `scoped()`.
pub mod routes {
    /// `POST` a `VoiceTokenRequest`: a `VoiceToken`.
    pub const VOICE_TOKEN: &str = "/api/room/voice/token";
    /// `POST` raw audio with an `UtteranceQuery`: the posted `RoomMessage`, 201 (200 for a repeat).
    pub const VOICE_UTTERANCE: &str = "/api/room/voice/utterance";
    /// `GET` the speech of one agent reply to a spoken turn.
    pub const VOICE_SPEECH: &str = "/api/room/voice/speech/{message_id}";
    /// `POST` a `VoiceJoinFailed`: 204.
    pub const VOICE_JOIN_FAILED: &str = "/api/room/voice/join-failed";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_join_reason_counts_as_other() {
        for r in join_reason::ALL {
            assert_eq!(join_reason::normalise(r), r);
        }
        assert_eq!(join_reason::normalise("UDP"), join_reason::OTHER);
        assert_eq!(join_reason::normalise(""), join_reason::OTHER);
    }

    #[test]
    fn an_utterance_query_round_trips_without_to() {
        let q = UtteranceQuery {
            repo: "m0x6A/rubato".into(),
            number: 146,
            branch: "issue/146-voice".into(),
            id: Uuid::nil(),
            to: None,
        };
        let v = serde_json::to_value(&q).unwrap();
        assert!(v.get("to").is_none());
        assert_eq!(serde_json::from_value::<UtteranceQuery>(v).unwrap(), q);
    }

    #[test]
    fn availability_omits_no_hosts() {
        let a = VoiceAvailability {
            enabled: true,
            speech: false,
            speech_hosts: vec![],
        };
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::json!({"enabled": true, "speech": false})
        );
    }
}
