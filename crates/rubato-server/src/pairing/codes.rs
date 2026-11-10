//! Device codes, user codes and tokens.

use rand::RngCore;

/// Eight characters from an alphabet without look-alikes, as `XXXX-XXXX`.
pub fn user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    let chars: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn user_codes_are_shaped() {
        let c = super::user_code();
        assert_eq!(c.len(), 9);
        assert_eq!(&c[4..5], "-");
        assert!(
            c.chars()
                .filter(|c| *c != '-')
                .all(|c| "BCDFGHJKLMNPQRSTVWXZ23456789".contains(c))
        );
    }
}
