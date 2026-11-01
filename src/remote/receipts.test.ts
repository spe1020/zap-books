import { describe, expect, test } from "vitest";
import type { SourceDocument } from "../domain/types";
import { checkDocument } from "../domain/documents";
import { sha256 } from "./commands";
import { decryptReceipt, encryptReceipt } from "./receipts";

// An invented one-pixel PNG.
const png = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aOCcAAAAASUVORK5CYII=",
  ),
  (c) => c.charCodeAt(0),
);

async function document(): Promise<SourceDocument & { sha256: string }> {
  const blob = new Blob([png], { type: "image/png" });
  return {
    id: "doc-1",
    businessId: "b".repeat(64),
    name: "invented.png",
    mime: "image/png",
    size: blob.size,
    blob,
    kind: "receipt",
    createdAt: "2026-10-15T12:00:00.000Z",
    demo: false,
    sha256: await sha256(blob),
  };
}

const bytes = async (blob: Blob) => new Uint8Array(await blob.arrayBuffer());

describe("receipt encryption (receipt storage)", () => {
  test("a receipt decrypts to the same bytes, and its stored copy passes the ledger's check", async () => {
    const { document: stored, ciphertext } = await encryptReceipt(
      await document(),
    );
    expect(() => checkDocument(stored)).not.toThrow();
    expect(stored.stored!.size).toBe(png.length + 16);
    expect(stored.stored!.hash).toBe(await sha256(new Blob([ciphertext])));
    const opened = await decryptReceipt(stored, ciphertext.slice().buffer);
    expect(await bytes(opened)).toEqual(png);
    expect(opened.type).toBe("image/png");
  });

  test("every file gets its own key and nonce", async () => {
    const d = await document();
    const a = (await encryptReceipt(d)).document.stored!;
    const b = (await encryptReceipt(d)).document.stored!;
    expect(a.key).not.toBe(b.key);
    expect(a.nonce).not.toBe(b.nonce);
    expect(a.hash).not.toBe(b.hash);
  });

  test("altered ciphertext is refused, and nothing is returned", async () => {
    const { document: stored, ciphertext } = await encryptReceipt(
      await document(),
    );
    const altered = ciphertext.slice();
    altered[5] ^= 1;
    await expect(decryptReceipt(stored, altered.buffer)).rejects.toThrow(
      "altered or damaged",
    );
    // Even with the hash made to match, the key won't open it.
    const forged = {
      ...stored,
      stored: {
        ...stored.stored!,
        hash: await sha256(new Blob([altered])),
      },
    };
    await expect(decryptReceipt(forged, altered.buffer)).rejects.toThrow(
      "altered or damaged",
    );
  });

  test("a file that isn't the document's is refused", async () => {
    const { document: stored, ciphertext } = await encryptReceipt(
      await document(),
    );
    await expect(
      decryptReceipt({ ...stored, sha256: "0".repeat(64) }, ciphertext.buffer),
    ).rejects.toThrow("altered or damaged");
    await expect(
      decryptReceipt({ ...stored, mime: "application/pdf" }, ciphertext.buffer),
    ).rejects.toThrow("isn't the type the books record");
  });
});
