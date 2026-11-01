import type { SourceDocument } from "../domain/types";
import { id, now } from "../domain/ledger";
import { MAX_RECEIPT_BYTES } from "../domain/documents";
export async function attachment(
  file: File,
  businessId: string,
): Promise<SourceDocument> {
  if (file.size > MAX_RECEIPT_BYTES || file.size === 0)
    throw new Error("Choose a nonempty image or PDF up to 10 MB.");
  const header = new Uint8Array(await file.slice(0, 16).arrayBuffer());
  const starts = (...bytes: number[]) => bytes.every((v, i) => header[i] === v);
  let mime: string;
  if (starts(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a))
    mime = "image/png";
  else if (starts(0xff, 0xd8, 0xff)) mime = "image/jpeg";
  else if (
    new TextDecoder().decode(header.slice(0, 4)) === "RIFF" &&
    new TextDecoder().decode(header.slice(8, 12)) === "WEBP"
  )
    mime = "image/webp";
  else if (new TextDecoder().decode(header.slice(0, 5)) === "%PDF-")
    mime = "application/pdf";
  else
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
