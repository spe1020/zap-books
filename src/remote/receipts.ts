/**
 * Receipt files on the ledger service (docs/briefs/receipt-storage.md).
 * Each file is encrypted in this browser with AES-256-GCM, through
 * WebCrypto, under a key and nonce of its own, before it leaves the device.
 * The key travels only in the document record, inside the books. Opening a
 * file checks it against the document: a mismatch is an error, never a
 * file shown.
 */
import type { SourceDocument, StoredFile } from "../domain/types";
import { checkDocument } from "../domain/documents";
import { fileType } from "../adapters/attachments";

const ALG = "AES-256-GCM";

const hex = (bytes: ArrayBuffer) =>
  [...new Uint8Array(bytes)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

async function sha256(bytes: BufferSource) {
  return hex(await crypto.subtle.digest("SHA-256", bytes));
}

const toBase64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes));
const fromBase64 = (text: string) =>
  Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

/** Why a stored receipt can't be shown. */
export class ReceiptError extends Error {
  name = "ReceiptError";
}

/**
 * Encrypts `document`'s file with a fresh random key and nonce. Returns the
 * document with its `stored` copy described, and the ciphertext to upload.
 */
export async function encryptReceipt(
  document: SourceDocument & { sha256: string },
): Promise<{ document: SourceDocument; ciphertext: Uint8Array<ArrayBuffer> }> {
  const raw = crypto.getRandomValues(new Uint8Array(32));
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const key = await crypto.subtle.importKey("raw", raw, "AES-GCM", false, [
    "encrypt",
  ]);
  const ciphertext = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      key,
      await document.blob.arrayBuffer(),
    ),
  );
  const stored: StoredFile = {
    hash: await sha256(ciphertext),
    size: ciphertext.byteLength,
    alg: ALG,
    key: toBase64(raw),
    nonce: toBase64(nonce),
  };
  const next = { ...document, stored };
  checkDocument(next);
  return { document: next, ciphertext };
}

/**
 * Opens a downloaded ciphertext as `document`'s file: it must be the blob
 * the document names, decrypt with its key, hash to its `sha256`, and be
 * the file type it records.
 */
export async function decryptReceipt(
  document: SourceDocument,
  ciphertext: ArrayBuffer,
): Promise<Blob> {
  const stored = document.stored;
  if (!stored || stored.alg !== ALG)
    throw new ReceiptError("This receipt's encryption isn't supported.");
  const altered = new ReceiptError(
    "This receipt's stored file doesn't match the books: it was altered or damaged, so it isn't shown.",
  );
  if (
    ciphertext.byteLength !== stored.size ||
    (await sha256(ciphertext)) !== stored.hash
  )
    throw altered;
  let plaintext: ArrayBuffer;
  try {
    const key = await crypto.subtle.importKey(
      "raw",
      fromBase64(stored.key),
      "AES-GCM",
      false,
      ["decrypt"],
    );
    plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: fromBase64(stored.nonce) },
      key,
      ciphertext,
    );
  } catch {
    throw altered;
  }
  if ((await sha256(plaintext)) !== document.sha256) throw altered;
  if (fileType(new Uint8Array(plaintext.slice(0, 16))) !== document.mime)
    throw new ReceiptError(
      "This receipt's file isn't the type the books record, so it isn't shown.",
    );
  return new Blob([plaintext], { type: document.mime });
}
