//! `src/domain/documents.ts`: a document's stored copy.

use crate::error::{LedgerError, Result};
use crate::types::SourceDocument;

/// The largest receipt that can be attached, in bytes.
pub const MAX_RECEIPT_BYTES: u64 = 10 * 1024 * 1024;
/// AES-256-GCM adds a 16-byte tag, so a stored receipt can be this large.
pub const MAX_STORED_BYTES: u64 = MAX_RECEIPT_BYTES + 16;
pub const STORED_ALGORITHMS: [&str; 1] = ["AES-256-GCM"];

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn base64_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
}

/// Canonical base64, with padding, of exactly 32 bytes: 42 characters, one
/// whose last two bits are zero, and `=`.
fn key256(s: &str) -> bool {
    let b = s.as_bytes();
    match b {
        [body @ .., last, b'='] if body.len() == 42 => {
            body.iter().copied().all(base64_char) && b"AEIMQUYcgkosw048".contains(last)
        }
        _ => false,
    }
}

/// Base64 of exactly 12 bytes: 16 characters, no padding.
fn nonce96(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(base64_char)
}

/// `checkDocument`: a document's `stored` copy, if it has one, has a valid
/// address, size, algorithm, key and nonce, and the document records the
/// hash of its file. A document without one passes unchanged.
pub fn check_document(document: &SourceDocument) -> Result<()> {
    let Some(stored) = &document.stored else {
        return Ok(());
    };
    if !document.sha256.as_deref().is_some_and(hex64) {
        return Err(LedgerError::StoredWithoutHash);
    }
    if !hex64(&stored.hash) {
        return Err(LedgerError::StoredAddress);
    }
    if stored.size == 0 || stored.size > MAX_STORED_BYTES {
        return Err(LedgerError::StoredSize);
    }
    if !STORED_ALGORITHMS.contains(&stored.alg.as_str()) {
        return Err(LedgerError::StoredAlgorithm);
    }
    if !key256(&stored.key) || !nonce96(&stored.nonce) {
        return Err(LedgerError::StoredKey);
    }
    Ok(())
}
