import { describe, expect, test } from "vitest";
import { checkDocument, MAX_STORED_BYTES } from "./documents";
import type { SourceDocument, StoredFile } from "./types";

const stored: StoredFile = {
  hash: "4d".repeat(32),
  size: 2064,
  alg: "AES-256-GCM",
  key: "A".repeat(43) + "=",
  nonce: "B".repeat(16),
};
const document = (patch: Partial<StoredFile> | null = {}): SourceDocument => ({
  id: "doc-stored",
  businessId: "northstar",
  name: "receipt.pdf",
  mime: "application/pdf",
  size: 2048,
  blob: new Blob(["x"]),
  kind: "receipt",
  createdAt: "2026-10-15T12:00:00.000Z",
  demo: false,
  sha256: "5e".repeat(32),
  ...(patch ? { stored: { ...stored, ...patch } } : {}),
});

describe("a document's stored copy (receipt storage)", () => {
  test("a document without one, or with a valid one, passes", () => {
    expect(() => checkDocument(document(null))).not.toThrow();
    expect(() => checkDocument(document())).not.toThrow();
    expect(() =>
      checkDocument(document({ size: MAX_STORED_BYTES })),
    ).not.toThrow();
  });

  test("it needs the file's own hash", () => {
    const { sha256: _, ...unhashed } = document();
    expect(() => checkDocument(unhashed)).toThrow(
      "A stored receipt needs the SHA-256 of its file.",
    );
  });

  test.each([
    ["uppercase", { hash: "4D".repeat(32) }, "address"],
    ["short", { hash: "4d".repeat(31) }, "address"],
    ["zero size", { size: 0 }, "between"],
    ["oversized", { size: MAX_STORED_BYTES + 1 }, "between"],
    ["fractional size", { size: 1.5 }, "between"],
    ["unknown algorithm", { alg: "AES-128-GCM" as never }, "encryption"],
    ["short key", { key: "A".repeat(42) + "==" }, "256-bit"],
    ["non-canonical key", { key: "A".repeat(42) + "B=" }, "256-bit"],
    ["unpadded key", { key: "A".repeat(43) }, "256-bit"],
    ["long nonce", { nonce: "B".repeat(20) }, "96-bit"],
    ["url-safe nonce", { nonce: "B".repeat(15) + "-" }, "96-bit"],
  ])("a %s is refused", (_, patch, message) => {
    expect(() => checkDocument(document(patch))).toThrow(message);
  });
});
