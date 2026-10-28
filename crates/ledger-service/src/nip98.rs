//! NIP-98 HTTP Auth for reads: a signed kind-27235 event in the
//! `Authorization` header, checked for its URL, method and time, and never
//! accepted twice.
//!
//! The checks themselves are `nostr`'s `verify_auth_header`. Replay
//! protection needs the auth event's id, which that function doesn't return,
//! so the header is decoded here too.

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, Engine};
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

/// Standard base64, padding required and canonical, no stray trailing bits:
/// every header has one decoding.
const STRICT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical)
        .with_decode_allow_trailing_bits(false),
);

/// Strict standard base64 with padding. Anything else, including an empty
/// string, is refused.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() {
        return None;
    }
    STRICT.decode(text).ok()
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
