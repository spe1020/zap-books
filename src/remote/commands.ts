/**
 * The protocol command each `LedgerCommand` becomes (`docs/protocol.md`,
 * Commands). Proposals never carry a receipt file: only each document's id
 * and SHA-256 and, once it is encrypted, its `stored` copy (`receipts.ts`).
 */
import type { BusinessData, SourceDocument } from "../domain/types";
import { recordExtraction } from "../domain/extraction";
import type { LedgerCommand, ReceiptReading } from "../adapters/ledger-client";
import { aRole, type MembershipChange } from "../domain/members";

/** SHA-256 of a file's bytes, lowercase hex. */
export async function sha256(blob: Blob): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    await blob.arrayBuffer(),
  );
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** Gives each new receipt its hash, computed from the file attached. */
export async function withHashes(
  documents: SourceDocument[],
): Promise<SourceDocument[]> {
  return Promise.all(
    documents.map(async (d) =>
      d.sha256 || !d.blob ? d : { ...d, sha256: await sha256(d.blob) },
    ),
  );
}

const reference = (d: SourceDocument) => {
  if (!d.sha256) throw new Error("A receipt's hash is missing.");
  return {
    id: d.id,
    sha256: d.sha256,
    name: d.name,
    mime: d.mime,
    size: d.size,
    ...(d.stored && { stored: d.stored }),
  };
};

/** The reading as the record the service checks and stores. */
function extractionRecord(
  b: BusinessData,
  draftId: string,
  reading: ReceiptReading,
) {
  const recorded = recordExtraction(b, { draftId, ...reading });
  return recorded.extractions[recorded.extractions.length - 1];
}

/** The protocol command for `command`, applied to the books `b`. */
export function protocolCommand(
  b: BusinessData,
  command: LedgerCommand,
): Record<string, unknown> {
  switch (command.type) {
    case "SaveDraft":
      return {
        type: "SaveDraft",
        draft: command.draft,
        documents: command.documents.map(reference),
        ...(command.extraction && {
          extraction: extractionRecord(b, command.draft.id, command.extraction),
        }),
      };
    case "Correct":
      return {
        type: "Correct",
        originalId: command.originalId,
        replacement: command.replacement,
        reason: command.reason,
        documents: command.documents.map(reference),
        ...(command.extraction && {
          extraction: extractionRecord(
            b,
            command.replacement.id,
            command.extraction,
          ),
        }),
      };
    case "Approve":
      return { type: "Approve", expenseId: command.expenseId };
    case "CommitImport":
      return {
        type: "CommitImport",
        accountId: command.accountId,
        rows: command.rows,
      };
    case "UpdateSettings":
      return { type: "UpdateSettings", change: command.change };
    case "SaveRule":
      return { type: "SaveRule", rule: command.rule };
    case "RecordTransfer":
      return { type: "RecordTransfer", transfer: command.transfer };
    case "CorrectTransfer":
      return {
        type: "CorrectTransfer",
        originalId: command.originalId,
        replacement: command.replacement,
        reason: command.reason,
      };
    case "RecordOpeningBalance":
      return {
        type: "RecordOpeningBalance",
        openingBalance: command.openingBalance,
      };
    case "ChangeMembership":
      return { type: "ChangeMembership", change: command.change };
    case "Veto":
      return { type: "Veto", pending: command.pending };
  }
}

/** A short description, for the "waiting to sync" list. */
export function describe(command: LedgerCommand): string {
  switch (command.type) {
    case "SaveDraft":
      return `Save draft${command.draft.vendor.trim() ? `: ${command.draft.vendor.trim()}` : ""}`;
    case "Approve":
      return "Approve an expense";
    case "Correct":
      return `Correct an expense${command.replacement.vendor.trim() ? `: ${command.replacement.vendor.trim()}` : ""}`;
    case "CommitImport":
      return `Import ${command.rows.length} row${command.rows.length === 1 ? "" : "s"}`;
    case "UpdateSettings":
      return "Update settings";
    case "SaveRule":
      return `Save rule: ${command.rule.vendorIncludes.trim()}`;
    case "RecordTransfer":
      return "Record a transfer";
    case "CorrectTransfer":
      return "Correct a transfer";
    case "RecordOpeningBalance":
      return "Record an opening balance";
    case "ChangeMembership":
      return describeChange(command.change);
    case "Veto":
      return "Veto a pending change";
  }
}

/** A membership change in a few words. */
export function describeChange(change: MembershipChange): string {
  switch (change.op) {
    case "add":
      return `Add ${aRole(change.role)}`;
    case "remove":
      return "Remove a member";
    case "changeRole":
      return `Make a member ${aRole(change.role)}`;
    case "replaceRecoveryKey":
      return change.key ? "Replace the backup key" : "Drop the backup key";
    case "replaceServiceKey":
      return "Move to a new service key";
    case "recover":
      return "Recover an owner";
  }
}

/** The command as queued: receipt files are kept separately. */
export function withoutFiles(command: LedgerCommand): LedgerCommand {
  if (command.type !== "SaveDraft" && command.type !== "Correct")
    return command;
  return {
    ...command,
    documents: command.documents.map(
      ({ blob: _, ...rest }) => rest as SourceDocument,
    ),
  };
}
