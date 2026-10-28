//! Size limits that keep every event publishable on public relays.

/// The largest serialized event (its NIP-01 JSON, in bytes) any builder
/// produces or any parser accepts. strfry, which many public relays run,
/// refuses events over 65,536 bytes by default; nostr-rs-relay's default is
/// 131,072 and khatru's 512,000. `docs/protocol.md` records the check.
pub const MAX_EVENT_BYTES: usize = 65_536;

/// How many rows a client puts in its first `CommitImport` proposal of an
/// import. On a [`crate::RejectionCode::TooLarge`] rejection it sends the
/// same rows again in a batch of [`next_batch`] rows (see `docs/protocol.md`).
pub const IMPORT_BATCH_START: usize = 25;

/// The batch to try after a `tooLarge` rejection of `rows` rows whose event
/// would have been `size` bytes against `limit`: `rows × limit ÷ size × 0.9`,
/// rounded down, at least 1. The 10% margin covers rows longer than the
/// batch's average. A rejected single row can't be sent at all.
pub fn next_batch(rows: usize, size: u64, limit: u64) -> usize {
    if size == 0 {
        return rows.max(1);
    }
    let scaled = (rows as u128 * limit as u128 * 9) / (size as u128 * 10);
    usize::try_from(scaled)
        .unwrap_or(usize::MAX)
        .clamp(1, rows.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_batch_scales_by_the_overshoot_with_a_margin() {
        // 25 rows came to 70,000 bytes: 25 × 65,536 ÷ 70,000 × 0.9 = 21.06.
        assert_eq!(next_batch(25, 70_000, 65_536), 21);
        // Twice the limit: about half, less the margin.
        assert_eq!(next_batch(25, 131_072, 65_536), 11);
        // Never zero, never more than before.
        assert_eq!(next_batch(1, 900_000, 65_536), 1);
        assert_eq!(next_batch(3, 65_537, 65_536), 2);
    }
}
