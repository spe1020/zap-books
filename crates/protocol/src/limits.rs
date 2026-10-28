//! Size limits that keep every event publishable on public relays.

/// The largest serialized event (its NIP-01 JSON, in bytes) any builder
/// produces or any parser accepts. strfry, which many public relays run,
/// refuses events over 65,536 bytes by default; nostr-rs-relay's default is
/// 131,072 and khatru's 512,000. `docs/protocol.md` records the check.
pub const MAX_EVENT_BYTES: usize = 65_536;

/// How many rows a client puts in its first `CommitImport` proposal of an
/// import. On a [`crate::RejectionCode::TooLarge`] rejection it halves the
/// batch and sends the same rows again (see `docs/protocol.md`).
pub const IMPORT_BATCH_START: usize = 25;
