import type {
  BusinessData,
  Currency,
  ExpenseDraft,
  FinancialEvent,
  EventType,
  JournalEntry,
  JournalLine,
  LedgerCommands,
  LedgerQueries,
} from "./types";
import { currencies } from "./types";
export const id = () => crypto.randomUUID();
export const now = () => new Date().toISOString();
/** The user's local calendar date as `YYYY-MM-DD`. Timestamps stay UTC. */
export function today() {
  const d = new Date();
  const pad = (n: number, width = 2) => String(n).padStart(width, "0");
  return `${pad(d.getFullYear(), 4)}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
export const currentPeriod = () => today().slice(0, 7);
export function money(value: number, currency: Currency) {
  return new Intl.NumberFormat("en-US", { style: "currency", currency }).format(
    value / 100,
  );
}
export function parseMoney(input: string): number {
  const s = input.trim();
  if (!/^-?\d+(\.\d{1,2})?$/.test(s))
    throw new Error(
      "Use an amount with up to two decimal places, such as 42.50.",
    );
  const negative = s.startsWith("-");
  const [whole, cents = ""] = s.replace("-", "").split(".");
  const value = Number(whole) * 100 + Number(cents.padEnd(2, "0"));
  if (!Number.isSafeInteger(value)) throw new Error("Amount is too large.");
  return negative ? -value : value;
}
export const amountInput = (n: number) =>
  `${n < 0 ? "-" : ""}${Math.floor(Math.abs(n) / 100)}.${String(Math.abs(n) % 100).padStart(2, "0")}`;
export function validDate(date: string) {
  return (
    /^\d{4}-\d{2}-\d{2}$/.test(date) &&
    !Number.isNaN(Date.parse(date)) &&
    new Date(date).toISOString().slice(0, 10) === date
  );
}
function sum(a: number, b: number) {
  const result = a + b;
  if (!Number.isSafeInteger(result))
    throw new Error("Financial total exceeds supported range.");
  return result;
}
const normalize = (s: string) => s.trim().toLowerCase().replace(/\s+/g, " ");
export function duplicates(b: BusinessData, d: ExpenseDraft): ExpenseDraft[] {
  return b.drafts.filter(
    (x) =>
      x.id !== d.id &&
      x.businessId === d.businessId &&
      x.id !== d.correctionOf &&
      x.status !== "corrected" &&
      x.amountMinor === d.amountMinor &&
      x.currency === d.currency &&
      x.date === d.date &&
      normalize(x.vendor) === normalize(d.vendor) &&
      !!d.vendor.trim(),
  );
}
export function missingDetails(b: BusinessData, d: ExpenseDraft): string[] {
  const errors: string[] = [];
  if (d.businessId !== b.profile.id)
    errors.push("Expense belongs to another business.");
  if (d.importSource && d.importSource.businessId !== b.profile.id)
    errors.push("Import source belongs to another business.");
  if (!d.vendor.trim()) errors.push("Vendor is required.");
  if (!validDate(d.date)) errors.push("A valid expense date is required.");
  if (!d.description.trim()) errors.push("Description is required.");
  if (!Number.isSafeInteger(d.amountMinor) || d.amountMinor <= 0)
    errors.push("Amount must be greater than zero in whole minor units.");
  if (!currencies.includes(d.currency))
    errors.push("Choose a supported currency.");
  const category = b.categories.find(
    (x) => x.id === d.categoryId && x.businessId === d.businessId,
  );
  if (!category) errors.push("Category is required.");
  const account = b.accounts.find(
    (x) =>
      x.id === d.accountId &&
      x.businessId === d.businessId &&
      x.kind !== "expense",
  );
  if (!account) errors.push("Payment account is required.");
  else if (account.currency !== d.currency)
    errors.push("Payment account currency must match the expense.");
  if (!d.purpose.trim()) errors.push("Business purpose is required.");
  if (
    d.documentIds.some(
      (docId) =>
        !b.documents.some(
          (x) => x.id === docId && x.businessId === d.businessId,
        ),
    )
  )
    errors.push("Receipt does not belong to this business.");
  if (d.paymentId) {
    const p = b.payments.find(
      (x) => x.id === d.paymentId && x.businessId === d.businessId,
    );
    if (
      !p ||
      p.amountMinor !== d.amountMinor ||
      p.currency !== d.currency ||
      p.accountId !== d.accountId ||
      p.date !== d.date
    )
      errors.push(
        "Expense must match its imported payment amount, date, currency, and account.",
      );
  }
  return errors;
}
export function validation(b: BusinessData, d: ExpenseDraft) {
  const errors = missingDetails(b, d);
  if (duplicates(b, d).length && !d.duplicateReason.trim())
    errors.push("Explain why this possible duplicate is a separate expense.");
  return errors;
}
export function assertBalanced(b: BusinessData, entry: JournalEntry) {
  if (
    entry.businessId !== b.profile.id ||
    !currencies.includes(entry.currency) ||
    !validDate(entry.date) ||
    entry.lines.length < 2
  )
    throw new Error("Invalid journal context.");
  let debit = 0,
    credit = 0;
  for (const line of entry.lines) {
    const account = b.accounts.find(
      (x) => x.id === line.accountId && x.businessId === entry.businessId,
    );
    if (
      !account ||
      (account.kind !== "expense" && account.currency !== entry.currency)
    )
      throw new Error("Invalid journal account or currency.");
    if (
      ![line.debit, line.credit].every(
        (x) => Number.isSafeInteger(x) && x >= 0,
      ) ||
      (line.debit > 0 && line.credit > 0)
    )
      throw new Error("Invalid journal amounts.");
    if (
      line.categoryId &&
      !b.categories.some(
        (c) =>
          c.id === line.categoryId &&
          c.businessId === entry.businessId &&
          c.accountId === account.id,
      )
    )
      throw new Error("Invalid journal category.");
    debit = sum(debit, line.debit);
    credit = sum(credit, line.credit);
  }
  if (debit !== credit || debit <= 0)
    throw new Error("Journal entry must balance.");
}
export function event(
  b: BusinessData,
  type: EventType,
  entityId: string,
  detail: string,
  sourceIds: string[] = [],
) {
  const at = now();
  const e: FinancialEvent = {
    id: id(),
    businessId: b.profile.id,
    schemaVersion: 1,
    type,
    actor: "local-demo-user",
    occurredAt: at,
    sourceIds,
    entityId,
    payload: { detail },
  };
  b.events.push(e);
  b.audits.push({
    id: id(),
    businessId: b.profile.id,
    action: type,
    entityId,
    at,
    actor: e.actor,
    detail,
  });
}
export function blankDraft(b: BusinessData): ExpenseDraft {
  return {
    id: id(),
    businessId: b.profile.id,
    vendor: "",
    date: today(),
    description: "",
    amountMinor: 0,
    currency: b.profile.currency,
    categoryId: "",
    accountId:
      b.accounts.find(
        (x) => x.kind === "bank" && x.currency === b.profile.currency,
      )?.id ?? "",
    purpose: "",
    documentIds: [],
    status: "draft",
    duplicateReason: "",
    createdAt: now(),
  };
}
export function saveDraft(
  business: BusinessData,
  draft: ExpenseDraft,
): BusinessData {
  return writeDraft(business, draft, { allowImportSource: false });
}
/**
 * The body of `saveDraft`. Only `commitImport` may pass
 * `allowImportSource: true`; it is exported for that call alone and is not
 * part of the public ledger API.
 *
 * With `allowImportSource: false`, the draft's `importSource` must equal the
 * evidence it already had: the stored draft's, or `evidenceOf`'s when a
 * correction carries its original's evidence onto a new draft.
 */
export function writeDraft(
  business: BusinessData,
  draft: ExpenseDraft,
  options: { allowImportSource: boolean; evidenceOf?: ExpenseDraft },
): BusinessData {
  const b = structuredClone(business);
  const prior = b.drafts.find((x) => x.id === draft.id);
  if (draft.importSource && draft.importSource.businessId !== b.profile.id)
    throw new Error("Import source belongs to another business.");
  if (
    prior?.importSource &&
    JSON.stringify(prior.importSource) !== JSON.stringify(draft.importSource)
  )
    throw new Error("Imported source evidence cannot be changed.");
  const evidence = (options.evidenceOf ?? prior)?.importSource;
  if (
    !options.allowImportSource &&
    JSON.stringify(evidence) !== JSON.stringify(draft.importSource)
  )
    throw new Error(
      evidence
        ? "Imported source evidence cannot be changed."
        : "Imported source evidence can only be added by an import.",
    );
  if (
    draft.businessId !== b.profile.id ||
    draft.status !== "draft" ||
    b.drafts.some((x) => x.id === draft.id && x.status !== "draft")
  )
    throw new Error("Only drafts in this business can be edited.");
  if (
    draft.documentIds.some(
      (docId) =>
        !b.documents.some(
          (x) => x.id === docId && x.businessId === draft.businessId,
        ),
    )
  )
    throw new Error("Invalid receipt reference.");
  const index = b.drafts.findIndex((x) => x.id === draft.id);
  if (index === -1) b.drafts.push(structuredClone(draft));
  else b.drafts[index] = structuredClone(draft);
  if (
    draft.vendor.trim() &&
    !b.vendors.some((v) => normalize(v.name) === normalize(draft.vendor))
  )
    b.vendors.push({
      id: id(),
      businessId: b.profile.id,
      name: draft.vendor.trim(),
    });
  event(
    b,
    index === -1 ? "expense.created" : "expense.updated",
    draft.id,
    index === -1 ? "Expense draft created" : "Expense draft updated",
    [
      ...draft.documentIds,
      ...(draft.paymentId ? [draft.paymentId] : []),
      ...(draft.importSource ? [draft.importSource.id] : []),
    ],
  );
  return b;
}
export function approve(
  business: BusinessData,
  expenseId: string,
): BusinessData {
  const b = structuredClone(business);
  const d = b.drafts.find(
    (x) => x.id === expenseId && x.businessId === b.profile.id,
  );
  if (!d) throw new Error("Expense not found in this business.");
  if (d.status !== "draft")
    throw new Error("This expense has already been posted.");
  if (
    b.approvals.some((x) => x.expenseId === d.id) ||
    b.journals.some((x) => x.expenseId === d.id)
  )
    throw new Error("Duplicate posting prevented.");
  const errors = validation(b, d);
  if (errors.length) throw new Error(errors.join(" "));
  // A real correction has already reversed and marked its original corrected.
  // Naming an expense in correctionOf cannot exempt a still-posted record.
  if (
    d.importSource &&
    b.drafts.some(
      (x) =>
        x.id !== d.id &&
        x.status === "posted" &&
        x.importSource?.fingerprint === d.importSource?.fingerprint,
    )
  )
    throw new Error("This imported source already has a posted expense.");
  if (
    d.paymentId &&
    b.drafts.some(
      (x) =>
        x.id !== d.id && x.status === "posted" && x.paymentId === d.paymentId,
    )
  )
    throw new Error("This payment is already linked to a posted expense.");
  const category = b.categories.find(
    (x) => x.id === d.categoryId && x.businessId === b.profile.id,
  )!;
  const lines: JournalLine[] = [
    {
      accountId: category.accountId,
      categoryId: category.id,
      debit: d.amountMinor,
      credit: 0,
    },
    { accountId: d.accountId, debit: 0, credit: d.amountMinor },
  ];
  const journal: JournalEntry = {
    id: id(),
    businessId: b.profile.id,
    date: d.date,
    currency: d.currency,
    lines,
    kind: "expense",
    expenseId: d.id,
    memo: `${d.vendor} · ${d.description}`,
    createdAt: now(),
  };
  assertBalanced(b, journal);
  b.journals.push(journal);
  d.status = "posted";
  d.journalId = journal.id;
  b.approvals.push({
    id: id(),
    businessId: b.profile.id,
    expenseId: d.id,
    actor: "local-demo-user",
    at: now(),
    duplicateReason: d.duplicateReason,
  });
  event(
    b,
    "expense.approved",
    d.id,
    "Reviewed and approved. No payment initiated.",
    [
      ...d.documentIds,
      ...(d.paymentId ? [d.paymentId] : []),
      ...(d.importSource ? [d.importSource.id] : []),
    ],
  );
  event(b, "expense.posted", d.id, "Balanced journal entry recorded", [
    journal.id,
  ]);
  return b;
}
export function correct(
  business: BusinessData,
  originalId: string,
  replacement: ExpenseDraft,
  reason: string,
): BusinessData {
  let b = structuredClone(business);
  const original = b.drafts.find(
    (x) => x.id === originalId && x.businessId === b.profile.id,
  );
  if (!original || original.status !== "posted" || !reason.trim())
    throw new Error("A posted expense and correction reason are required.");
  if (
    replacement.id === original.id ||
    b.drafts.some((x) => x.id === replacement.id) ||
    replacement.businessId !== b.profile.id ||
    replacement.status !== "draft"
  )
    throw new Error("Invalid replacement.");
  if (original.paymentId !== replacement.paymentId)
    throw new Error("Preserve the payment link when correcting an expense.");
  if (
    JSON.stringify(original.importSource) !==
    JSON.stringify(replacement.importSource)
  )
    throw new Error(
      "Preserve import source evidence when correcting an expense.",
    );
  const prior = b.journals.find((x) => x.id === original.journalId)!;
  const reversal: JournalEntry = {
    ...prior,
    id: id(),
    kind: "reversal",
    reversesId: prior.id,
    memo: `Correction: ${reason}`,
    lines: prior.lines.map((x) => ({ ...x, debit: x.credit, credit: x.debit })),
    createdAt: now(),
  };
  assertBalanced(b, reversal);
  original.status = "corrected";
  b.journals.push(reversal);
  event(b, "expense.reversed", original.id, reason, [
    prior.id,
    reversal.id,
    replacement.id,
  ]);
  b = writeDraft(
    b,
    { ...replacement, correctionOf: original.id },
    { allowImportSource: false, evidenceOf: original },
  );
  return approve(b, replacement.id);
}
export function categoryTotals(
  b: BusinessData,
  period: string,
  currency: Currency,
) {
  const totals = new Map<string, number>();
  for (const j of b.journals)
    if (
      j.businessId === b.profile.id &&
      j.currency === currency &&
      j.date.startsWith(period)
    )
      for (const line of j.lines)
        if (line.categoryId)
          totals.set(
            line.categoryId,
            sum(totals.get(line.categoryId) ?? 0, line.debit - line.credit),
          );
  return [...totals]
    .map(([categoryId, amount]) => ({
      categoryId,
      name:
        b.categories.find(
          (c) => c.id === categoryId && c.businessId === b.profile.id,
        )?.name ?? "Unknown",
      amount,
    }))
    .filter((x) => x.amount !== 0)
    .sort((a, z) => z.amount - a.amount);
}
export function totals(
  b: BusinessData,
  period: string,
): Partial<Record<Currency, number>> {
  const result: Partial<Record<Currency, number>> = {};
  for (const j of b.journals)
    if (j.businessId === b.profile.id && j.date.startsWith(period))
      for (const line of j.lines)
        if (line.categoryId)
          result[j.currency] = sum(
            result[j.currency] ?? 0,
            line.debit - line.credit,
          );
  return result;
}
export const ledger: LedgerCommands & LedgerQueries = {
  approve,
  correct,
  totals,
};
