//! NIP-98 HTTP Auth for reads: a signed kind-27235 event in the
//! `Authorization` header, checked for its URL, method and time, and never
//! accepted twice.
//!
//! The checks themselves are `nostr`'s `verify_auth_header`. Replay
//! protection needs the auth event's id, which that function doesn't return,
//! so the header is decoded here too (base64 is an encoding, not
//! cryptography, and `base64` isn't an allowed dependency).

use nostr::nips::nip98::{verify_auth_header, HttpMethod};
use nostr::prelude::{Event, EventId, PublicKey, Timestamp};
use nostr::types::url::Url;

/// How long a used auth event is remembered. `verify_auth_header` refuses
/// events more than 60 seconds old or 30 seconds ahead, so this covers every
/// event that could still pass.
pub const REMEMBER_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorized {
    pub pubkey: PublicKey,
    pub event_id: EventId,
}

/// Checks `header` for a request to `url` with `method` at `now`. The caller
/// still has to record the event id to refuse replays.
pub fn check(header: &str, url: &Url, method: HttpMethod, now: Timestamp) -> Option<Authorized> {
    let pubkey = verify_auth_header(header, url, method, now, None).ok()?;
    let encoded = header.strip_prefix("Nostr ")?;
    let json = decode_base64(encoded)?;
    let event = Event::from_json(json).ok()?;
    (event.pubkey == pubkey).then_some(Authorized {
        pubkey,
        event_id: event.id,
    })
}

/// Strict standard base64 with padding. Anything else is refused, so every
/// header has one decoding.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let (quads, rest) = text.as_bytes().as_chunks::<4>();
    if quads.is_empty() || !rest.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(quads.len() * 3);
    let last = quads.len() - 1;
    for (i, quad) in quads.iter().enumerate() {
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && i != last) {
            return None;
        }
        let mut n = 0u32;
        for &c in quad.iter().take(4 - pad) {
            n = (n << 6) | value(c)?;
        }
        n <<= 6 * pad as u32;
        // Non-canonical: bits below the last byte must be zero.
        if (pad == 1 && n & 0xff != 0) || (pad == 2 && n & 0xffff != 0) {
            return None;
        }
        let decoded = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend(decoded.iter().take(3 - pad));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_strictly() {
        let decoded = |s| decode_base64(s).map(String::from_utf8);
        assert_eq!(decoded("aGk="), Some(Ok("hi".into())));
        assert_eq!(decoded("aGVsbG8="), Some(Ok("hello".into())));
        assert_eq!(decoded("YWJj"), Some(Ok("abc".into())));
        assert_eq!(decoded("YQ=="), Some(Ok("a".into())));
        for bad in ["", "aGk", "aG=k", "aGk==", "a===", "aGl=", "YR==", "@@@@"] {
            assert_eq!(decode_base64(bad), None, "{bad}");
        }
    }
}
