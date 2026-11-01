import type { SourceDocument, StoredAlgorithm } from "./types";

/** The largest receipt that can be attached, in bytes. */
export const MAX_RECEIPT_BYTES = 10 * 1024 * 1024;
/** AES-256-GCM adds a 16-byte tag, so a stored receipt can be this large. */
export const MAX_STORED_BYTES = MAX_RECEIPT_BYTES + 16;
export const storedAlgorithms: StoredAlgorithm[] = ["AES-256-GCM"];

const hex64 = /^[0-9a-f]{64}$/;
// Canonical base64, with padding, of exactly 32 bytes and of 12 bytes.
const key256 = /^[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]=$/;
const nonce96 = /^[A-Za-z0-9+/]{16}$/;

/**
 * Checks a document's `stored` copy, if it has one: its address, size,
 * algorithm, key and nonce, and that the document also records the hash of
 * its file, which the browser checks after decrypting. A document without
 * `stored` (every one made before receipt storage) passes unchanged.
 */
export function checkDocument(document: SourceDocument): void {
  const stored = document.stored;
  if (stored === undefined) return;
  if (typeof document.sha256 !== "string" || !hex64.test(document.sha256))
    throw new Error("A stored receipt needs the SHA-256 of its file.");
  if (typeof stored.hash !== "string" || !hex64.test(stored.hash))
    throw new Error(
      "A stored receipt's address must be 64 lowercase hex characters.",
    );
  if (
    !Number.isSafeInteger(stored.size) ||
    stored.size <= 0 ||
    stored.size > MAX_STORED_BYTES
  )
    throw new Error(
      "A stored receipt must be between 1 byte and 10 MB plus its encryption overhead.",
    );
  if (!storedAlgorithms.includes(stored.alg))
    throw new Error("Unsupported receipt encryption.");
  if (
    typeof stored.key !== "string" ||
    typeof stored.nonce !== "string" ||
    !key256.test(stored.key) ||
    !nonce96.test(stored.nonce)
  )
    throw new Error(
      "A stored receipt needs a 256-bit key and a 96-bit nonce, in base64.",
    );
}
