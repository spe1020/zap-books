export type Currency = "USD" | "EUR" | "GBP";
export const currencies: Currency[] = ["USD", "EUR", "GBP"];
export interface Scoped {
  id: string;
  businessId: string;
}
export interface Business extends Scoped {
  name: string;
  industry: string;
  currency: Currency;
  initials: string;
  color: string;
}
/**
 * `liability` is money the business owes (a loan from the owner, say) and
 * `equity` the owner's stake. Both can pay an expense: from a liability the
 * business now owes it, from equity the owner contributed it.
 */
export type AccountKind = "bank" | "card" | "expense" | "liability" | "equity";
export interface Account extends Scoped {
  name: string;
  kind: AccountKind;
  currency: Currency;
}
export interface Category extends Scoped {
  name: string;
  accountId: string;
}
export interface Vendor extends Scoped {
  name: string;
}
export interface SourceDocument extends Scoped {
  name: string;
  mime: string;
  size: number;
  blob: Blob;
  kind: "receipt";
  createdAt: string;
  demo: boolean;
  /** SHA-256 of the file bytes, lowercase hex, when known. */
  sha256?: string;
}
export interface ImportedTransaction extends Scoped {
  date: string;
  description: string;
  amountMinor: number;
  currency: Currency;
  accountId: string;
  reference: string;
  fingerprint: string;
  draftId: string;
}
export type ImportProvider = "csv" | "quickbooks" | "zoho";
// Embedded source evidence is preserved when the draft is edited or corrected.
export interface ImportedExpenseSource extends Scoped {
  provider: ImportProvider;
  reference: string;
  fingerprint: string;
  date: string;
  amountMinor: number;
  currency: Currency;
  vendor: string;
  category: string;
  filename: string;
}
export interface ExpenseDraft extends Scoped {
  vendor: string;
  date: string;
  description: string;
  amountMinor: number;
  currency: Currency;
  categoryId: string;
  accountId: string;
  purpose: string;
  documentIds: string[];
  paymentId?: string;
  importSource?: ImportedExpenseSource;
  status: "draft" | "posted" | "corrected";
  journalId?: string;
  correctionOf?: string;
  duplicateReason: string;
  createdAt: string;
}
export interface JournalLine {
  accountId: string;
  categoryId?: string;
  debit: number;
  credit: number;
}
/**
 * What a transfer journal records. `openingBalance` is set only by
 * `recordOpeningBalance`; the others are chosen by the user.
 */
export type TransferType =
  | "ownerLoan"
  | "ownerContribution"
  | "loanRepayment"
  | "ownerDraw"
  | "betweenAccounts"
  | "openingBalance";
export interface JournalEntry extends Scoped {
  date: string;
  currency: Currency;
  lines: JournalLine[];
  kind: "expense" | "transfer" | "reversal";
  expenseId?: string;
  reversesId?: string;
  memo: string;
  createdAt: string;
  /** Transfers, and their reversals. Absent on transfers made before types. */
  transferType?: TransferType;
  purpose?: string;
  reference?: string;
  documentIds?: string[];
  /** On a transfer that replaced a corrected one: the original's id. */
  correctionOf?: string;
}
export interface Approval extends Scoped {
  expenseId: string;
  actor: string;
  at: string;
  duplicateReason: string;
}
export interface ClassificationRule extends Scoped {
  vendorIncludes: string;
  categoryId: string;
  purpose: string;
  enabled: boolean;
}
export interface AuditEvent extends Scoped {
  action: string;
  entityId: string;
  at: string;
  actor: string;
  detail: string;
}
export type EventType =
  | "expense.created"
  | "expense.approved"
  | "expense.posted"
  | "expense.matched"
  | "expense.reversed"
  | "expense.updated"
  | "rule.saved"
  | "settings.updated"
  | "transfer.recorded"
  | "transfer.reversed";
// Application-defined schema. This is not a Nostr accounting standard.
export interface FinancialEvent extends Scoped {
  schemaVersion: 1;
  type: EventType;
  actor: string;
  occurredAt: string;
  sourceIds: string[];
  entityId: string;
  payload: Record<string, string | number | boolean>;
}
export interface BusinessData {
  profile: Business;
  accounts: Account[];
  categories: Category[];
  vendors: Vendor[];
  documents: SourceDocument[];
  payments: ImportedTransaction[];
  drafts: ExpenseDraft[];
  journals: JournalEntry[];
  approvals: Approval[];
  rules: ClassificationRule[];
  audits: AuditEvent[];
  events: FinancialEvent[];
  extractions: ExtractionRecord[];
  /** Absent until the first suggestion; workspaces saved before have none. */
  suggestions?: Suggestion[];
}
/**
 * A proposal attached to a draft by an assistant (decision 0004): a
 * category, a purpose or a note, with the evidence for it and the key that
 * signed it. It never changes the draft; a person accepts or ignores it.
 */
export interface Suggestion extends Scoped {
  draftId: string;
  /** The assistant's public key, 64 lowercase hex characters. */
  author: string;
  categoryId?: string;
  purpose?: string;
  note?: string;
  /** What the suggestion rests on; untrusted, shown as plain text. */
  evidence: string[];
  createdAt: string;
}
export interface Workspace {
  schemaVersion: 1;
  revision: number;
  businesses: BusinessData[];
  /** Stored records that failed the check on load, kept as they were. */
  quarantine?: QuarantinedRecord[];
}
/**
 * A stored record set aside because it failed the check on load. It is kept
 * so nothing is lost, but no screen, total or export reads it.
 */
export interface QuarantinedRecord {
  /** Null when the business itself couldn't be read. */
  businessId: string | null;
  /** `business` for a whole business, else the list it was in. */
  collection: string;
  id: string | null;
  /** A fixed description of the check that failed; never record text. */
  reason: string;
  record: unknown;
  at: string;
}
export interface AIProposal {
  categoryId?: string;
  purpose?: string;
  explanation: string;
  evidence: string[];
  uncertainty: string;
  simulated: true;
  ruleId?: string;
}
/** The fields a receipt reader proposes. Business purpose is never one. */
export const extractionFields = [
  "vendor",
  "date",
  "total",
  "currency",
  "reference",
  "subtotal",
  "tax",
  "fees",
  "discounts",
  "tip",
  "category",
] as const;
export type ExtractionFieldName = (typeof extractionFields)[number];
/** Fields read as lists of printed lines. */
export const extractionLists = ["tax", "fees", "discounts"] as const;
export type ExtractionListName = (typeof extractionLists)[number];
export interface ExtractedField<T> {
  status: "found" | "unclear" | "absent";
  /** null unless status is "found". */
  value: T | null;
  /** Short text seen on the receipt; untrusted, shown as plain text. */
  evidence: string;
}
/** One printed amount line. */
export interface ExtractedLine {
  /** Minor units; null unless the list is "found". */
  amount: number | null;
  /** Short text seen on the receipt; untrusted, shown as plain text. */
  evidence: string;
}
/** Every printed line of one kind. */
export interface ExtractedLines {
  status: "found" | "unclear" | "absent";
  /** The sum of the lines in minor units; null unless status is "found". */
  value: number | null;
  /** Empty when absent; an unclear list keeps the evidence it saw. */
  lines: ExtractedLine[];
}
export interface ExtractionFields {
  vendor: ExtractedField<string>;
  /** YYYY-MM-DD */
  date: ExtractedField<string>;
  /** Minor units. */
  total: ExtractedField<number>;
  currency: ExtractedField<Currency>;
  reference: ExtractedField<string>;
  subtotal: ExtractedField<number>;
  tax: ExtractedLines;
  /** Surcharges and convenience, service or delivery fees. */
  fees: ExtractedLines;
  /** Coupons, savings, rewards and markdowns, as positive amounts. */
  discounts: ExtractedLines;
  /** A tip or gratuity line; absent when the receipt has none. */
  tip: ExtractedField<number>;
  /** A category id of this business. */
  category: ExtractedField<string>;
}
export interface ExtractionResult {
  fields: ExtractionFields;
  warnings: string[];
  model: string;
  promptVersion: string;
  pagesRead: number;
  usedTextLayer: boolean;
  durationMs: number;
  /** Whether the model reports thinking support; null if unknown. Thinking is always requested off. */
  modelSupportsThinking: boolean | null;
  /** How the total was confirmed; null when it isn't found. Absent from older extractors. */
  totalConfirmedBy?:
    "label" | "reconciliation" | "labelAndReconciliation" | null;
}
export type ExtractionOutcome = "accepted" | "edited" | "ignored" | "unclear";
/** What a receipt reader proposed for a draft, and what the user did. */
export interface ExtractionRecord {
  id: string;
  businessId: string;
  documentId: string;
  draftId: string;
  model: string;
  promptVersion: string;
  createdAt: string;
  fields: ExtractionFields;
  /** Fields the user clicked Accept on while editing. */
  accepted: ExtractionFieldName[];
  /** Per-field outcome, set when the draft is approved; null before. */
  outcomes: Partial<Record<ExtractionFieldName, ExtractionOutcome>> | null;
}
export type ExtractorHealth =
  { ready: true; model: string } | { ready: false; reason: string };
export interface ExtractionAdapter {
  health(): Promise<ExtractorHealth>;
  /** Rejects with the signal's `AbortError` when `signal` cancels it. */
  extract(
    document: SourceDocument,
    categories: { id: string; name: string }[],
    signal?: AbortSignal,
  ): Promise<ExtractionResult>;
}
export interface AIAdapter {
  propose(business: BusinessData, expense: ExpenseDraft): AIProposal;
  answer(business: BusinessData, question: string, period: string): string;
}
export interface BusinessRepository {
  read(businessId: string): Promise<BusinessData>;
  commit(
    businessId: string,
    business: BusinessData,
    expectedRevision: number,
  ): Promise<Workspace>;
}
export interface ReceiptStorage {
  list(businessId: string): Promise<SourceDocument[]>;
  read(businessId: string, documentId: string): Promise<Blob>;
}
export interface LedgerCommands {
  approve(business: BusinessData, expenseId: string): BusinessData;
  correct(
    business: BusinessData,
    originalId: string,
    replacement: ExpenseDraft,
    reason: string,
  ): BusinessData;
}
export interface LedgerQueries {
  totals(
    business: BusinessData,
    period: string,
  ): Partial<Record<Currency, number>>;
}
export interface EventTransport {
  publish(events: FinancialEvent[]): Promise<void>;
  synchronize(businessId: string): Promise<FinancialEvent[]>;
}
