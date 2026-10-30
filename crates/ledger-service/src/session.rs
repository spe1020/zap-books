//! Read sessions. NIP-98 needs a signature for every request, which in a
//! browser is a signer-extension prompt per read. `POST /v1/session`,
//! signed with NIP-98, returns a random token that stands in for that
//! signature on reads for [`SESSION_SECS`]. Only its SHA-256 is stored.
//!
//! A token says who is asking, nothing more: every read still checks the
//! asker's membership, so a removal cuts a reader off at once.

use sha2::{Digest, Sha256};

/// How long a session lasts: one hour.
pub const SESSION_SECS: u64 = 60 * 60;

/// A new token, 32 random bytes as 64 lowercase hex characters, and the
/// hash to store for it.
pub fn generate() -> (String, String) {
    let token = hex::encode(rand::random::<[u8; 32]>());
    let hash = hash(&token).unwrap_or_default();
    (token, hash)
}

/// The stored hash of `token`: SHA-256 of its bytes, hex. `None` unless the
/// token has the form [`generate`] gives it, so nothing else is looked up.
pub fn hash(token: &str) -> Option<String> {
    let well_formed = token.len() == 64
        && token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    well_formed.then(|| hex::encode(Sha256::digest(token.as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_fresh_and_only_their_form_is_looked_up() {
        let (a, hash_a) = generate();
        let (b, _) = generate();
        assert_ne!(a, b);
        assert_eq!(hash(&a), Some(hash_a.clone()));
        assert_ne!(hash_a, a);
        for bad in ["", "abc", &a.to_uppercase(), &format!("{a}0"), &a[1..]] {
            assert_eq!(hash(bad), None, "{bad}");
        }
    }
}
