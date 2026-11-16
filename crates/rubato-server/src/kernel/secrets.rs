//! Sealing with `RUBATO_TOKEN_KEY` (AES-256-GCM, a random nonce per seal): user tokens (ADR
//! 0030) and a workspace's traffic secrets (ADR 0075). Moved here from `auth::tokens` so a slice
//! other than `auth` can open what it stores.

use crate::kernel::config::TokenKey;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, anyhow};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(text.len().is_multiple_of(2), "odd hex");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).context("hex"))
        .collect()
}

pub fn seal(key: &TokenKey, plain: &str) -> Result<String> {
    let cipher = Aes256Gcm::new(&key.0.into());
    let nonce: [u8; 12] = rand::random();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|_| anyhow!("encrypting a token failed"))?;
    Ok(hex(&[nonce.as_slice(), &ct].concat()))
}

pub fn open(key: &TokenKey, sealed: &str) -> Result<String> {
    let bytes = unhex(sealed)?;
    anyhow::ensure!(bytes.len() > 12, "sealed token too short");
    let cipher = Aes256Gcm::new(&key.0.into());
    let plain = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .map_err(|_| anyhow!("a stored token does not open with RUBATO_TOKEN_KEY"))?;
    Ok(String::from_utf8(plain)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_text_opens_with_its_key_only() {
        let key = TokenKey([7; 32]);
        let sealed = seal(&key, "Authorization=Bearer x").unwrap();
        assert!(!sealed.contains("Bearer"));
        assert_eq!(open(&key, &sealed).unwrap(), "Authorization=Bearer x");
        assert!(open(&TokenKey([8; 32]), &sealed).is_err());
        assert_ne!(
            seal(&key, "a").unwrap(),
            seal(&key, "a").unwrap(),
            "a nonce per seal"
        );
    }
}
