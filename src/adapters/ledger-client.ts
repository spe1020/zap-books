import type {
  ExpenseDraft,
  ExtractionFieldName,
  ExtractionResult,
  SourceDocument,
  Workspace,
} from "../domain/types";
import type { ImportRow } from "../domain/import";
import type { RuleInput, SettingsChange } from "../domain/settings";
import type { MembershipChange } from "../domain/members";
import type { OpeningBalanceInput, TransferInput } from "../domain/transfers";

/** What the receipt reader proposed for a draft, and what the user took. */
export interface ReceiptReading {
  documentId: string;
  result: ExtractionResult;
  accepted: ExtractionFieldName[];
}

/**
 * One change to a business's books. Each type is a command of the ledger
 * protocol (`docs/protocol.md`). A `LedgerClient` applies it locally or
 * sends it to the service as a proposal; the domain functions run first in
 * either case, so validation messages appear before anything is sent.
 */
export type LedgerCommand =
  | {
      type: "SaveDraft";
      draft: ExpenseDraft;
      /** Receipts first attached in this save. */
      documents: SourceDocument[];
      extraction?: ReceiptReading;
    }
  | { type: "Approve"; expenseId: string }
  | {
      type: "Correct";
      originalId: string;
      replacement: ExpenseDraft;
      reason: string;
      documents: SourceDocument[];
      extraction?: ReceiptReading;
    }
  | { type: "CommitImport"; accountId: string; rows: ImportRow[] }
  | { type: "UpdateSettings"; change: SettingsChange }
  | { type: "SaveRule"; rule: RuleInput }
  | { type: "RecordTransfer"; transfer: TransferInput }
  | {
      type: "CorrectTransfer";
      originalId: string;
      replacement: TransferInput;
      reason: string;
    }
  | { type: "RecordOpeningBalance"; openingBalance: OpeningBalanceInput }
  /** Membership needs the service: the local demo has no members. */
  | { type: "ChangeMembership"; change: MembershipChange }
  | { type: "Veto"; pending: string };

type Input<T extends LedgerCommand["type"]> = Omit<
  Extract<LedgerCommand, { type: T }>,
  "type"
>;

/**
 * The app's books, wherever they are kept. Every command answers with the
 * workspace as it stands afterwards, or throws with a message for the user.
 */
export interface LedgerClient {
  /** The workspace: every business this client holds, and its records. */
  load(): Promise<Workspace>;
  saveDraft(businessId: string, input: Input<"SaveDraft">): Promise<Workspace>;
  approve(businessId: string, input: Input<"Approve">): Promise<Workspace>;
  correct(businessId: string, input: Input<"Correct">): Promise<Workspace>;
  commitImport(
    businessId: string,
    input: Input<"CommitImport">,
  ): Promise<Workspace>;
  updateSettings(
    businessId: string,
    input: Input<"UpdateSettings">,
  ): Promise<Workspace>;
  saveRule(businessId: string, input: Input<"SaveRule">): Promise<Workspace>;
  recordTransfer(
    businessId: string,
    input: Input<"RecordTransfer">,
  ): Promise<Workspace>;
  correctTransfer(
    businessId: string,
    input: Input<"CorrectTransfer">,
  ): Promise<Workspace>;
  recordOpeningBalance(
    businessId: string,
    input: Input<"RecordOpeningBalance">,
  ): Promise<Workspace>;
  changeMembership(
    businessId: string,
    input: Input<"ChangeMembership">,
  ): Promise<Workspace>;
  veto(businessId: string, input: Input<"Veto">): Promise<Workspace>;
  /**
   * Commands that make up one action, in order: saving a draft and then
   * approving it, say. Each command is checked against the books as the one
   * before left them.
   */
  run(businessId: string, commands: LedgerCommand[]): Promise<Workspace>;
}

/** The single-command methods, in terms of `run`. */
export abstract class CommandClient implements LedgerClient {
  abstract load(): Promise<Workspace>;
  abstract run(
    businessId: string,
    commands: LedgerCommand[],
  ): Promise<Workspace>;
  saveDraft(businessId: string, input: Input<"SaveDraft">) {
    return this.run(businessId, [{ type: "SaveDraft", ...input }]);
  }
  approve(businessId: string, input: Input<"Approve">) {
    return this.run(businessId, [{ type: "Approve", ...input }]);
  }
  correct(businessId: string, input: Input<"Correct">) {
    return this.run(businessId, [{ type: "Correct", ...input }]);
  }
  commitImport(businessId: string, input: Input<"CommitImport">) {
    return this.run(businessId, [{ type: "CommitImport", ...input }]);
  }
  updateSettings(businessId: string, input: Input<"UpdateSettings">) {
    return this.run(businessId, [{ type: "UpdateSettings", ...input }]);
  }
  saveRule(businessId: string, input: Input<"SaveRule">) {
    return this.run(businessId, [{ type: "SaveRule", ...input }]);
  }
  recordTransfer(businessId: string, input: Input<"RecordTransfer">) {
    return this.run(businessId, [{ type: "RecordTransfer", ...input }]);
  }
  correctTransfer(businessId: string, input: Input<"CorrectTransfer">) {
    return this.run(businessId, [{ type: "CorrectTransfer", ...input }]);
  }
  recordOpeningBalance(
    businessId: string,
    input: Input<"RecordOpeningBalance">,
  ) {
    return this.run(businessId, [{ type: "RecordOpeningBalance", ...input }]);
  }
  changeMembership(businessId: string, input: Input<"ChangeMembership">) {
    return this.run(businessId, [{ type: "ChangeMembership", ...input }]);
  }
  veto(businessId: string, input: Input<"Veto">) {
    return this.run(businessId, [{ type: "Veto", ...input }]);
  }
}
