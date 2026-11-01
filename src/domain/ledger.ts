import type {
  BusinessData,
  Currency,
  ExpenseDraft,
  FiatCurrency,
  FinancialEvent,
  EventType,
  ImportedExpenseSource,
  JournalEntry,
  JournalLine,
  LedgerCommands,
  LedgerQueries,
} from "./types";
import { currencies, precision } from "./types";
import { finalizeExtractions } from "./extraction";
import {
  fiatValueError,
  fiatValueOf,
  fiatValueShape,
  millisatsError,
  fiatValueMatches,
} from "./bitcoin";
export const id = () => crypto.randomUUID();
export const now = () => new Date().toISOString();
/** The user's local calendar date as `YYYY-MM-DD`. Timestamps stay UTC. */
export function today() {
  const d = new Date();
  const pad = (n: number, width = 2) => String(n).padStart(width, "0");
  return `${pad(d.getFullYear(), 4)}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
export const currentPeriod = () => today().slice(0, 7);
/** How a bitcoin amount is shown: sats, the default, or BTC. */
export type BitcoinUnit = "sats" | "BTC";
/**
 * An amount for display. Fiat uses the currency's own format; bitcoin shows
 * whole sats ("2,100 sats"), or BTC to eight places ("0.00002100 BTC").
 */
export function money(
  value: number,
  currency: Currency,
  unit: BitcoinUnit = "sats",
) {
  if (currency !== "BTC")
    return new Intl.NumberFormat("en-US", {
      style: "currency",
      currency,
    }).format(value / 100);
  if (unit === "sats")
    return `${new Intl.NumberFormat("en-US").format(value)} ${Math.abs(value) === 1 ? "sat" : "sats"}`;
  const [whole, fraction] = amountInput(value, "BTC").split(".");
  return `${whole.replace(/\B(?=(\d{3})+$)/g, ",")}.${fraction} BTC`;
}
const placeNames = [
  "no",
  "one",
  "two",
  "three",
  "four",
  "five",
  "six",
  "seven",
  "eight",
];
/** The message for an amount `parseMoney` can't read in `currency`. */
const amountFormat = (currency: Currency) =>
  currency === "BTC"
    ? "Use an amount with up to eight decimal places, such as 0.00012345."
    : `Use an amount with up to ${placeNames[precision[currency]]} decimal places, such as 42.50.`;
/**
 * Parses an amount typed in `currency`'s major unit (dollars, or bitcoin)
 * into exact minor units (cents, or sats), with up to the currency's number
 * of decimal places.
 */
export function parseMoney(input: string, currency: Currency): number {
  const places = precision[currency];
  const s = input.trim();
  if (!new RegExp(`^-?\\d+(\\.\\d{1,${places}})?$`).test(s))
    throw new Error(amountFormat(currency));
  const negative = s.startsWith("-");
  const [whole, fraction = ""] = s.replace("-", "").split(".");
  const value =
    Number(whole) * 10 ** places + Number(fraction.padEnd(places, "0"));
  if (!Number.isSafeInteger(value)) throw new Error("Amount is too large.");
  return negative ? -value : value;
}
/** Parses a whole number of sats, such as 2100. */
export function parseSats(input: string): number {
  const s = input.trim();
  if (!/^-?\d+$/.test(s)) throw new Error("Enter whole sats, such as 2100.");
  const value = Number(s);
  if (!Number.isSafeInteger(value)) throw new Error("Amount is too large.");
  return value;
}
/** Minor units as an amount in the major unit, to `currency`'s places. */
export function amountInput(n: number, currency: Currency) {
  const places = precision[currency];
  const unit = 10 ** places;
  const abs = Math.abs(n);
  const fraction = abs % unit;
  return `${n < 0 ? "-" : ""}${(abs - fraction) / unit}.${String(fraction).padStart(places, "0")}`;
}
/** A real `YYYY-MM-DD` calendar day in 1900 or later. */
export function validDate(date: string) {
  return (
    /^\d{4}-\d{2}-\d{2}$/.test(date) &&
    Number(date.slice(0, 4)) >= 1900 &&
    !Number.isNaN(Date.parse(date)) &&
    new Date(date).toISOString().slice(0, 10) === date
  );
}
export function sum(a: number, b: number) {
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
  for (const error of [
    fiatValueError(d.currency, d.amountMinor, d.fiatValue),
    millisatsError(d.currency, d.amountMinor, d.millisats),
  ])
    if (error) errors.push(error);
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
  if (entry.transferType === "exchange") return assertExchange(b, entry);
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
    // A category's line is spend, so it sits on an expense account; every
    // other line moves money between bank, card, loan and equity accounts.
    if (
      line.categoryId
        ? account.kind !== "expense" ||
          !b.categories.some(
            (c) =>
              c.id === line.categoryId &&
              c.businessId === entry.businessId &&
              c.accountId === account.id,
          )
        : account.kind === "expense"
    )
      throw new Error("Invalid journal category.");
    debit = sum(debit, line.debit);
    credit = sum(credit, line.credit);
  }
  if (debit !== credit || debit <= 0)
    throw new Error("Journal entry must balance.");
  // A BTC journal's sats are its total; it carries its fiat value at the
  // time, and no other journal carries one.
  if (fiatValueError(entry.currency, debit, entry.fiatValue))
    throw new Error("Invalid journal fiat value.");
  if (millisatsError(entry.currency, debit, entry.millisats))
    throw new Error("Invalid journal amounts.");
}
/**
 * An exchange is the one journal where two currencies meet: a BTC wallet's
 * line in sats, and a fiat bank or card account's line in its own currency,
 * for exactly the fiat value's amount, the two in opposite directions. Each
 * side balances in its own currency through the recorded rate.
 */
function assertExchange(b: BusinessData, entry: JournalEntry) {
  const value = entry.fiatValue;
  if (entry.currency !== "BTC" || entry.lines.length !== 2)
    throw new Error("Invalid journal context.");
  if (!value || fiatValueShape(value) || value.source !== "exchange")
    throw new Error("Invalid journal fiat value.");
  const sides = entry.lines.map((line) => {
    const account = b.accounts.find(
      (x) => x.id === line.accountId && x.businessId === entry.businessId,
    );
    const btc = account?.kind === "wallet" && account.currency === "BTC";
    const fiat =
      (account?.kind === "bank" || account?.kind === "card") &&
      account.currency === value.currency;
    if (!btc && !fiat) throw new Error("Invalid journal account or currency.");
    if (
      ![line.debit, line.credit].every(
        (x) => Number.isSafeInteger(x) && x >= 0,
      ) ||
      (line.debit > 0 && line.credit > 0)
    )
      throw new Error("Invalid journal amounts.");
    if (line.categoryId) throw new Error("Invalid journal category.");
    return { btc, line };
  });
  const btc = sides.find((x) => x.btc)?.line;
  const fiat = sides.find((x) => !x.btc)?.line;
  if (!btc || !fiat) throw new Error("Invalid journal account or currency.");
  const sats = btc.debit + btc.credit;
  if (
    !(btc.debit > 0 ? fiat.credit > 0 : btc.credit > 0 && fiat.debit > 0) ||
    fiat.debit + fiat.credit !== value.amountMinor
  )
    throw new Error("Journal entry must balance.");
  if (!fiatValueMatches(sats, value))
    throw new Error("Invalid journal fiat value.");
  if (millisatsError(entry.currency, sats, entry.millisats))
    throw new Error("Invalid journal amounts.");
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
 * With `allowImportSource: false`, the draft's `importSource` and `paymentId`
 * must equal the evidence it already had: the stored draft's, or
 * `evidenceOf`'s when a correction carries its original's evidence onto a
 * new draft.
 *
 * The stored draft is built from an explicit list of the fields a caller may
 * set (see `storedDraft`). Every field that carries authority, the evidence
 * links, `journalId` and `correctionOf`, comes from the record's own history:
 * the stored draft, or `evidenceOf` for a correction. A field added to
 * `ExpenseDraft` is therefore inert here until it is listed, never trusted by
 * default (PARITY_NOTES W13).
 *
 * @internal
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
  const payment = (options.evidenceOf ?? prior)?.paymentId;
  if (!options.allowImportSource && draft.paymentId !== payment)
    throw new Error("Payment links can only be created by an import.");
  if (
    draft.businessId !== b.profile.id ||
    draft.status !== "draft" ||
    b.drafts.some((x) => x.id === draft.id && x.status !== "draft")
  )
    throw new Error("Only drafts in this business can be edited.");
  // A stored draft must be one Rust can load (PARITY_NOTES D2), whatever
  // filled it in: a receipt reading, for one.
  if (!currencies.includes(draft.currency))
    throw new Error("Choose a supported currency.");
  if (!Number.isSafeInteger(draft.amountMinor))
    throw new Error("Amount must be in whole minor units.");
  if (draft.fiatValue !== undefined) {
    const error = fiatValueShape(draft.fiatValue);
    if (error) throw new Error(error);
  }
  if (draft.millisats !== undefined && !Number.isSafeInteger(draft.millisats))
    throw new Error("Millisats must be a whole number greater than zero.");
  if (
    draft.documentIds.some(
      (docId) =>
        !b.documents.some(
          (x) => x.id === docId && x.businessId === draft.businessId,
        ),
    )
  )
    throw new Error("Invalid receipt reference.");
  const stored = storedDraft(draft, {
    paymentId: options.allowImportSource ? draft.paymentId : payment,
    importSource: options.allowImportSource ? draft.importSource : evidence,
    journalId: prior?.journalId,
    correctionOf: options.evidenceOf?.id ?? prior?.correctionOf,
  });
  const index = b.drafts.findIndex((x) => x.id === stored.id);
  if (index === -1) b.drafts.push(stored);
  else b.drafts[index] = stored;
  if (
    stored.vendor.trim() &&
    !b.vendors.some((v) => normalize(v.name) === normalize(stored.vendor))
  )
    b.vendors.push({
      id: id(),
      businessId: b.profile.id,
      name: stored.vendor.trim(),
    });
  event(
    b,
    index === -1 ? "expense.created" : "expense.updated",
    stored.id,
    index === -1 ? "Expense draft created" : "Expense draft updated",
    [
      ...stored.documentIds,
      ...(stored.paymentId ? [stored.paymentId] : []),
      ...(stored.importSource ? [stored.importSource.id] : []),
    ],
  );
  return b;
}
/**
 * The fields of a draft that carry authority: the evidence it rests on and
 * the posting and correction it belongs to. `writeDraft` takes them from the
 * record's history, never from the caller.
 */
interface DraftAuthority {
  paymentId: string | undefined;
  importSource: ImportedExpenseSource | undefined;
  journalId: string | undefined;
  correctionOf: string | undefined;
}
/**
 * The draft `writeDraft` stores: the caller-settable fields copied from
 * `draft`, and the authority-bearing ones from `authority`. This is an
 * allow-list: a field missing here is dropped, not copied, so a new field on
 * `ExpenseDraft` must be placed deliberately on one side or the other. The
 * key order matches `blankDraft`, with the optional fields after it in the
 * order the ledger has always written them.
 */
function storedDraft(
  draft: ExpenseDraft,
  authority: DraftAuthority,
): ExpenseDraft {
  const stored: ExpenseDraft = {
    id: draft.id,
    businessId: draft.businessId,
    vendor: draft.vendor,
    date: draft.date,
    description: draft.description,
    amountMinor: draft.amountMinor,
    currency: draft.currency,
    categoryId: draft.categoryId,
    accountId: draft.accountId,
    purpose: draft.purpose,
    documentIds: [...draft.documentIds],
    status: draft.status,
    duplicateReason: draft.duplicateReason,
    createdAt: draft.createdAt,
  };
  if (draft.fiatValue !== undefined)
    stored.fiatValue = fiatValueOf(draft.fiatValue);
  if (draft.millisats !== undefined) stored.millisats = draft.millisats;
  if (authority.paymentId !== undefined) stored.paymentId = authority.paymentId;
  if (authority.importSource !== undefined)
    stored.importSource = structuredClone(authority.importSource);
  if (authority.correctionOf !== undefined)
    stored.correctionOf = authority.correctionOf;
  if (authority.journalId !== undefined) stored.journalId = authority.journalId;
  return stored;
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
  if (
    d.paymentId &&
    b.payments.find(
      (p) => p.id === d.paymentId && p.businessId === b.profile.id,
    )?.draftId !== d.id
  )
    throw new Error("This payment belongs to a different expense.");
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
  if (d.fiatValue) journal.fiatValue = fiatValueOf(d.fiatValue);
  if (d.millisats !== undefined) journal.millisats = d.millisats;
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
  // The receipt readings for this draft are judged against what was posted.
  return finalizeExtractions(b, d.id);
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
  // writeDraft records the replacement as a correction of `evidenceOf`.
  b = writeDraft(b, replacement, {
    allowImportSource: false,
    evidenceOf: original,
  });
  // The replacement takes over the original's payment.
  const payment = b.payments.find(
    (p) => p.id === original.paymentId && p.businessId === b.profile.id,
  );
  if (original.paymentId && payment) payment.draftId = replacement.id;
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
/**
 * Spending's "dollar value at time of transaction": for each reporting
 * currency, the recorded fiat values of the period's bitcoin expenses, a
 * reversal subtracting its original's. Built only from what was recorded,
 * never a price, and never added to `totals`, which keeps sats as sats.
 */
export function recordedFiatSpending(
  b: BusinessData,
  period: string,
): Partial<Record<FiatCurrency, number>> {
  const result: Partial<Record<FiatCurrency, number>> = {};
  for (const j of b.journals) {
    if (
      j.businessId !== b.profile.id ||
      !j.date.startsWith(period) ||
      !j.fiatValue
    )
      continue;
    let spend = 0;
    for (const line of j.lines)
      if (line.categoryId) spend = sum(spend, line.debit - line.credit);
    if (spend === 0) continue;
    const c = j.fiatValue.currency;
    result[c] = sum(
      result[c] ?? 0,
      spend > 0 ? j.fiatValue.amountMinor : -j.fiatValue.amountMinor,
    );
  }
  return result;
}
export const ledger: LedgerCommands & LedgerQueries = {
  approve,
  correct,
  totals,
  recordedFiatSpending,
};
