//! The `X-Hub-Signature-256` check (ADR 0074): HMAC-SHA256 of the raw body under the App's
//! webhook secret, compared in constant time, before anything in the body or the other headers
//! is read.

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Why a delivery's signature does not count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No `X-Hub-Signature-256` header.
    Missing,
    /// Not `sha256=` and 64 lower-case hex characters.
    Malformed,
    /// Well formed, and not the body's MAC under the secret.
    Mismatch,
}

impl Refused {
    pub fn reason(self) -> &'static str {
        match self {
            Refused::Missing => "signature_missing",
            Refused::Malformed => "signature_malformed",
            Refused::Mismatch => "signature_mismatch",
        }
    }
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Whether `header` is the signature GitHub computes for `body` under `secret`.
pub fn verify(secret: &[u8], header: Option<&str>, body: &[u8]) -> Result<(), Refused> {
    let header = header.ok_or(Refused::Missing)?;
    let hex = header.strip_prefix("sha256=").ok_or(Refused::Malformed)?;
    let hex = hex.as_bytes();
    if hex.len() != 64 {
        return Err(Refused::Malformed);
    }
    let mut sig = [0u8; 32];
    for (i, b) in sig.iter_mut().enumerate() {
        let hi = hex_value(hex[2 * i]).ok_or(Refused::Malformed)?;
        let lo = hex_value(hex[2 * i + 1]).ok_or(Refused::Malformed)?;
        *b = hi << 4 | lo;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Refused::Mismatch)?;
    mac.update(body);
    // `verify_slice` compares in constant time.
    mac.verify_slice(&sig).map_err(|_| Refused::Mismatch)
}

/// `sha256=<hex>` for `body` under `secret`: what GitHub sends. For tests and local tools.
pub fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's documented example ("Validating webhook deliveries"), then each way a signature
    /// can be wrong. Every case states its expected result.
    #[test]
    fn signatures() {
        let secret = b"It's a Secret to Everybody";
        let body = b"Hello, World!";
        let good = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        let flipped_body = b"Hello, World?";
        let upper = good.to_ascii_uppercase().replacen("SHA256=", "sha256=", 1);
        let short = &good[..good.len() - 2];
        let non_hex = format!("{}zz", &good[..good.len() - 2]);
        let no_prefix = good.trim_start_matches("sha256=");
        let signed = sign(secret, body);
        type Case<'a> = (
            &'a str,
            &'a [u8],
            &'a [u8],
            Option<&'a str>,
            Result<(), Refused>,
        );
        let cases: Vec<Case> = vec![
            ("github's example", secret, body, Some(good), Ok(())),
            ("sign() agrees", secret, body, Some(&signed), Ok(())),
            (
                "wrong secret",
                b"another secret",
                body,
                Some(good),
                Err(Refused::Mismatch),
            ),
            (
                "one body byte",
                secret,
                flipped_body,
                Some(good),
                Err(Refused::Mismatch),
            ),
            (
                "upper-case hex",
                secret,
                body,
                Some(&upper),
                Err(Refused::Malformed),
            ),
            (
                "no prefix",
                secret,
                body,
                Some(no_prefix),
                Err(Refused::Malformed),
            ),
            (
                "sha1 prefix",
                secret,
                body,
                Some("sha1=0a"),
                Err(Refused::Malformed),
            ),
            (
                "short hex",
                secret,
                body,
                Some(short),
                Err(Refused::Malformed),
            ),
            (
                "non-hex",
                secret,
                body,
                Some(&non_hex),
                Err(Refused::Malformed),
            ),
            ("missing", secret, body, None, Err(Refused::Missing)),
        ];
        assert_eq!(sign(secret, body), good, "sign() matches GitHub's example");
        for (name, secret, body, header, want) in &cases {
            assert_eq!(verify(secret, *header, body), *want, "{name}");
        }
        println!("checked {} signatures", cases.len());
    }
}
