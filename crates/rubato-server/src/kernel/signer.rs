//! HMAC-SHA256 signer for cookie values `id.sig`. In the kernel because `AppState` holds one.

use hmac::{Hmac, Mac};
use sha2::Sha256;

#[derive(Clone)]
pub struct Signer {
    key: Vec<u8>,
}

impl Signer {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            key: secret.to_vec(),
        }
    }

    fn mac(&self, id: &str) -> String {
        let mut m = Hmac::<Sha256>::new_from_slice(&self.key).expect("hmac key");
        m.update(id.as_bytes());
        m.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    pub fn sign(&self, id: &str) -> String {
        format!("{id}.{}", self.mac(id))
    }

    pub fn verify(&self, value: &str) -> Option<String> {
        let (id, sig) = value.rsplit_once('.')?;
        let expected = self.mac(id);
        // Constant-time compare.
        if expected.len() != sig.len() {
            return None;
        }
        let diff = expected
            .bytes()
            .zip(sig.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        (diff == 0).then(|| id.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_round_trips_and_rejects_tampering() {
        let s = Signer::new(b"secret");
        let v = s.sign("abc");
        assert_eq!(s.verify(&v).as_deref(), Some("abc"));
        let mut t = v.clone();
        t.replace_range(0..1, "x");
        assert_eq!(s.verify(&t), None);
        assert_eq!(Signer::new(b"other").verify(&v), None);
    }
}
