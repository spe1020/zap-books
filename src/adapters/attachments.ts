import type { SourceDocument } from "../domain/types";
import { id, now } from "../domain/ledger";
import { MAX_RECEIPT_BYTES } from "../domain/documents";

/**
 * The type of a receipt file, from its first 16 bytes: PNG, JPEG, WebP or
 * PDF, or `undefined` for anything else. The file's name and declared type
 * are never trusted.
 */
export function fileType(header: Uint8Array): string | undefined {
  const starts = (...bytes: number[]) => bytes.every((v, i) => header[i] === v);
  const text = (from: number, to: number) =>
    new TextDecoder().decode(header.slice(from, to));
  if (starts(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a))
    return "image/png";
  if (starts(0xff, 0xd8, 0xff)) return "image/jpeg";
  if (text(0, 4) === "RIFF" && text(8, 12) === "WEBP") return "image/webp";
  if (text(0, 5) === "%PDF-") return "application/pdf";
  return undefined;
}

export async function attachment(
  file: File,
  businessId: string,
): Promise<SourceDocument> {
  if (file.size > MAX_RECEIPT_BYTES || file.size === 0)
    throw new Error("Choose a nonempty image or PDF up to 10 MB.");
  const mime = fileType(new Uint8Array(await file.slice(0, 16).arrayBuffer()));
  if (!mime)
    throw new Error(
      "Supported attachments: PNG, JPEG, WebP, or PDF. This file does not match a supported format.",
    );
  const blob = new Blob([file], { type: mime });
  return {
    id: id(),
    businessId,
    name: file.name,
    mime,
    size: blob.size,
    blob,
    kind: "receipt",
    createdAt: now(),
    demo: false,
  };
}
