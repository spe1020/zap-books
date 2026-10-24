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
export interface Account extends Scoped {
  name: string;
  kind: "bank" | "card" | "expense";
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
export interface JournalEntry extends Scoped {
  date: string;
  currency: Currency;
  lines: JournalLine[];
  kind: "expense" | "transfer" | "reversal";
  expenseId?: string;
  reversesId?: string;
  memo: string;
  createdAt: string;
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
  | "settings.updated";
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
}
export interface Workspace {
  schemaVersion: 1;
  revision: number;
  businesses: BusinessData[];
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
