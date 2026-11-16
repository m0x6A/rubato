//! The LiveKit join token (plan 146 design §2): an HS256 JWT signed with the LiveKit secret, built
//! with the server's own `hmac` and `sha2`. Rubato never talks to LiveKit; `exp` bounds the join
//! only, since LiveKit refreshes the tokens of connected participants (ADR 0073).

use crate::kernel::config::LiveKitConfig;
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use rubato_protocol::voice::limits::TOKEN_SECS;
use rubato_protocol::{RoomKey, VoiceToken};
use sha2::{Digest, Sha256};

/// The LiveKit room of a plan room: `rubato-` and 32 hex digits of `sha256("<repo>#<n>@<branch>")`,
/// stable, URL-safe, and not spelling the branch out to LiveKit.
pub fn room_name(room: &RoomKey) -> String {
    let d = Sha256::digest(format!("{}#{}@{}", room.repo, room.number, room.branch).as_bytes());
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    format!("rubato-{}", &hex[..32])
}

/// A join token for `login` in `room`, valid from `now` for `TOKEN_SECS`. `suffix` makes the
/// identity unique per tab: LiveKit drops the older participant of a duplicate identity.
pub fn mint(
    lk: &LiveKitConfig,
    login: &str,
    room: &RoomKey,
    now: DateTime<Utc>,
    suffix: &str,
) -> VoiceToken {
    let name = room_name(room);
    let identity = format!("{login}#{suffix}");
    let exp = now + Duration::seconds(TOKEN_SECS);
    let header = serde_json::json!({ "alg": "HS256", "typ": "JWT" });
    let claims = serde_json::json!({
        "iss": lk.api_key,
        "sub": identity,
        "name": login,
        "nbf": now.timestamp(),
        "exp": exp.timestamp(),
        "video": {
            "room": name,
            "roomJoin": true,
            "canSubscribe": true,
            "canPublish": true,
            "canPublishData": false,
            "canPublishSources": ["microphone"],
        },
    });
    let signing = format!(
        "{}.{}",
        base64url(header.to_string().as_bytes()),
        base64url(claims.to_string().as_bytes())
    );
    let sig = sign(lk.api_secret.expose().as_bytes(), signing.as_bytes());
    VoiceToken {
        url: lk.url.clone(),
        token: format!("{signing}.{}", base64url(&sig)),
        room: name,
        identity,
        expires_at: exp,
    }
}

pub(super) fn sign(secret: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding (RFC 4648 §5), as a JWT needs. Hand-written: `base64` is not a
/// direct dependency, and this is twenty lines.
pub fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// The inverse of [`base64url`], for tests that read a token back.
#[cfg(test)]
pub fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let vals: Option<Vec<u32>> = s
        .bytes()
        .map(|c| ALPHABET.iter().position(|a| *a == c).map(|p| p as u32))
        .collect();
    for chunk in vals?.chunks(4) {
        let mut n = 0u32;
        for (i, v) in chunk.iter().enumerate() {
            n |= v << (18 - 6 * i);
        }
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Secret;

    #[test]
    fn base64url_matches_the_rfc_vectors() {
        // RFC 4648 §10, in the url alphabet without padding.
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff], "-_8"),
        ];
        for (raw, enc) in cases {
            assert_eq!(base64url(raw), *enc);
            assert_eq!(base64url_decode(enc).unwrap(), raw.to_vec());
        }
        println!("checked {} base64url vectors", cases.len());
    }

    #[test]
    fn the_room_name_is_stable_and_hides_the_branch() {
        let k = RoomKey {
            repo: "m0x6A/rubato".into(),
            number: 146,
            branch: "issue/146-voice-meetings".into(),
        };
        let a = room_name(&k);
        assert_eq!(a, room_name(&k));
        assert!(a.starts_with("rubato-") && a.len() == "rubato-".len() + 32);
        assert!(!a.contains("146"));
        let other = RoomKey {
            branch: "issue/146-other".into(),
            ..k
        };
        assert_ne!(a, room_name(&other));
        let _ = Secret::new("x");
    }
}
