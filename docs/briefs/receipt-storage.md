# Brief: Encrypted receipt storage

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Save this brief as:** `docs/briefs/receipt-storage.md` (Prettier only)
**Read first:** decisions `0001`, `0002`, `0004`, `docs/protocol.md`, `docs/service.md`, `PARITY_NOTES.md`.

**Privacy rule:** invented values only. The guard must pass on every commit and PR description.

## Why

In remote mode, receipt files live only in the browser that attached them. Clearing that browser's data, or switching to another computer, loses them. Receipts are records a business is expected to keep, so they must be stored safely with the books.

## Design

- **The browser encrypts every receipt before it leaves the device,** using a fresh random key per file.
- **The encrypted file is stored by the ledger service** in a blob store on its own disk. The service stores and serves ciphertext only.
- **The file's key lives inside the document record in the books,** which are already encrypted under the epoch keys. So only members who can read the books can decrypt receipts, and the key travels with the record through rebuild, audit and backups.
- **The service sees the file key when it validates the proposal,** just as it sees all proposal plaintext. That's consistent with decision 0001: the service is trusted with plaintext. Say so plainly in `docs/service.md`.
- **Later (stage 4),** the same encrypted blobs can be mirrored to Blossom servers as off-site copies. They're useless without the keys in the books. Use content-hash addressing now, so that mirroring needs no format change.

### Encryption

- **AES-256-GCM, through the browser's built-in WebCrypto.** No hand-written cryptography, and no new JavaScript cryptography library. NIP-44 can't be used: its 65,535-byte limit is far below a 10 MB receipt.
- Each file gets a fresh random 256-bit key and a 96-bit nonce from `crypto.getRandomValues`. Never reuse either.
- Record the algorithm name, so it can change later.
- **After decrypting,** the browser checks that the plaintext's SHA-256 equals the document's existing `sha256` and that the file type matches its recorded `mime` (using the existing file-type check). A mismatch is an error, never a silent display.

### The document record (parity change)

Add an optional `blob` field to `SourceDocument`:

```text
blob: {
  hash: string        // SHA-256 of the ciphertext, hex: the blob's address
  size: number        // ciphertext size in bytes
  alg: "AES-256-GCM"
  key: string         // base64 file key
  nonce: string       // base64 nonce
}
```

- `ledger-core` validates its shape: lowercase 64-character hex hash, positive size within the limit, a known `alg`, and correctly sized key and nonce.
- A document with a `blob` must also have `sha256`.
- Documents created before this feature have no `blob`. Keep loading them unchanged.
- Usual parity rule: TypeScript first, regenerate fixtures, then Rust. Add a `PARITY_NOTES.md` entry. The demo seed doesn't need blobs.

### The service's blob store

- **`PUT /v1/blobs/{hash}?business=<id>`** with a session token. The body is the ciphertext as `application/octet-stream`.
  - The caller must be an owner, bookkeeper, assistant or submitter of that business.
  - The service computes the SHA-256 of what it received and refuses a mismatch with the path.
  - The limit is the largest receipt (10 MB) plus encryption overhead, enforced while reading, before the whole body is buffered.
  - Idempotent: uploading an existing blob again returns success and stores nothing new.
  - The blob is recorded as **pending** for that business until an accepted record refers to it.
- **`GET /v1/blobs/{hash}?business=<id>`** with a session token.
  - The caller must be a member who can read the books: owner, bookkeeper, viewer or assistant. Submitters and the recovery key can't download.
  - The blob must belong to that business and be referenced by an accepted record.
  - Serve it with `Content-Type: application/octet-stream`, `X-Content-Type-Options: nosniff`, and `Content-Disposition: attachment`.
- **Proposal checks:** a proposal whose document has a `blob` is refused unless that blob was uploaded for the same business and its size matches. On acceptance, the blob moves from pending to referenced, in the same transaction.
- **Cleanup:** pending blobs not referenced within 24 hours are deleted by the existing scheduler. Referenced blobs are never deleted, since corrections keep history.
- **Storage:** files on disk in a directory given by a new `--blob-dir` flag, named by hash, written atomically (temporary file, then rename) with permissions `0600`. Postgres holds the index: hash, business, size, state, and created time.
- **Quota:** a per-business limit (`--blob-quota`, default 5 GB). Uploads beyond it get a clear error.
- **Disk full** gives a clear error, and never a half-written blob.
- `docs/service.md`: the blob store must be on the encrypted volume and included in backups (PR 5 will rely on this).

### Protocol

- Documents in `SaveDraft`, `Correct` and `SubmitReceiptDraft` may carry a `blob`. The service's checks above apply in all three.
- Rebuild and audit treat the `blob` field as ordinary record data. Neither needs the blob bytes.
- Update `docs/protocol.md`, the WASM bindings and the permission notes.

### The browser

- **On attaching a receipt in remote mode:** hash the file, encrypt it, upload it, then send the proposal with the document's `blob`.
- **Offline:** the encrypted blob waits in the IndexedDB queue ahead of its proposal and is uploaded first. If the upload is refused, the proposal isn't sent, and the queue shows why.
- **Viewing a receipt:**
  - use the local copy if this browser has one;
  - otherwise download, decrypt and verify it, then show it the same way as today (PDFs in a frame, images as images, never SVGs as pages);
  - cache it locally after a successful download.
- **Remove "This receipt was attached on another device"** for documents that have a `blob`. Keep it for older documents that don't.
- **Show upload progress** for large files. A failed upload can be retried without re-encrypting with a new key, by keeping the queued ciphertext.
- **Local demo mode is unchanged.**

## Required tests

**Every one of these is a merge gate.**

- Round trip: encrypt, upload, propose, download from a **second browser context** signed in as another member, decrypt, and the bytes are identical.
- Ciphertext tampered on disk: the browser refuses it with a clear error, and shows nothing.
- An upload whose body doesn't match its hash is refused.
- An oversized upload is refused before the whole body is read.
- A proposal referring to a blob that wasn't uploaded, belongs to another business, or has the wrong size is refused.
- Downloads are refused for a submitter, the recovery key, a removed member, a non-member, and a member of another business.
- An unreferenced pending blob is deleted after 24 hours (injected clock), and a referenced one is never deleted.
- Uploading the same blob twice stores one copy.
- The quota is enforced.
- Offline: an attachment made offline uploads before its proposal when back online.
- `ledger-core`: `blob` shape validation, in TypeScript and Rust, with parity fixtures.
- The service never logs keys, nonces or blob contents. Add a test that checks the logs during a full round trip.

## PR sequence

Do these as separate PRs. **Stop after each one is open.**

1. **Core and service:** the `SourceDocument.blob` parity change, the protocol and WASM updates, the blob store, the endpoints, the proposal checks, cleanup, quota, and their tests.
2. **Browser:** encryption, upload, the offline queue, download, verification and viewing, plus the e2e gates.

## Allowed dependencies

- **Rust:** nothing new beyond features of existing crates. Stop and ask if something is needed.
- **Frontend:** none. WebCrypto is built into browsers.

## Stop and ask if

- any design here would let the service or anyone else read receipts without the key in the books, or would put a file key outside the encrypted books;
- the 10 MB limit, the 24-hour cleanup or the default quota causes a problem in testing;
- a protocol or `ledger-core` behavior change beyond the `blob` field seems needed.

## Out of scope

- Mirroring blobs to Blossom servers (stage 4).
- Moving receipts from older test businesses into the blob store.
- Receipts for transfers and opening balances.
- Deleting receipts, and retention policies.

## Each PR description must include

1. Results of every check, including the merge-gate tests.
2. Any deviation from this brief, and why.
3. Confirmation that the privacy guard passed.
