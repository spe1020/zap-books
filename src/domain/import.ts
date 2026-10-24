import type { BusinessData, Currency, ImportedTransaction } from "./types";
import { currencies } from "./types";
import {
  blankDraft,
  event,
  id,
  parseMoney,
  saveDraft,
  validDate,
} from "./ledger";
export interface CSVMapping {
  date: string;
  description: string;
  amount: string;
  currency: string;
  reference: string;
}
export interface ImportRow {
  row: number;
  date: string;
  description: string;
  amountMinor: number;
  currency: Currency;
  reference: string;
  fingerprint: string;
  error?: string;
  duplicate: boolean;
}
export function parseCSV(text: string): string[][] {
  if (text.length > 2_000_000)
    throw new Error("CSV must be smaller than 2 MB.");
  const rows: string[][] = [];
  let row: string[] = [],
    cell = "",
    quoted = false,
    closed = false;
  const source = text.replace(/^\uFEFF/, "");
  for (let i = 0; i < source.length; i++) {
    const c = source[i];
    if (quoted) {
      if (c === '"') {
        if (source[i + 1] === '"') {
          cell += '"';
          i++;
        } else {
          quoted = false;
          closed = true;
        }
      } else cell += c;
    } else if (c === '"') {
      if (cell || closed) throw new Error("Malformed CSV quote.");
      quoted = true;
    } else if (c === "," || c === "\n" || c === "\r") {
      row.push(cell.trim());
      cell = "";
      closed = false;
      if (c !== ",") {
        if (row.some(Boolean)) rows.push(row);
        row = [];
        if (c === "\r" && source[i + 1] === "\n") i++;
      }
    } else {
      if (closed && c.trim())
        throw new Error("Unexpected text after a quoted value.");
      if (!closed) cell += c;
    }
  }
  if (quoted) throw new Error("Unclosed quote in CSV.");
  row.push(cell.trim());
  if (row.some(Boolean)) rows.push(row);
  if (rows.length < 2)
    throw new Error("CSV needs a header and at least one transaction.");
  if (rows[0].some((x) => !x) || new Set(rows[0]).size !== rows[0].length)
    throw new Error("CSV headers must be unique and nonempty.");
  if (rows.length > 2001)
    throw new Error("Import up to 2,000 transactions at a time.");
  return rows;
}
export function previewImport(
  b: BusinessData,
  rows: string[][],
  map: CSVMapping,
  accountId: string,
  convention: "negative" | "positive",
): ImportRow[] {
  const account = b.accounts.find(
    (x) => x.id === accountId && x.kind !== "expense",
  );
  const seen = new Map(
    b.payments.map((p) => [
      p.fingerprint,
      { date: p.date, amountMinor: p.amountMinor, currency: p.currency },
    ]),
  );
  return rows.slice(1).map((r, i) => {
    const value = (key: keyof CSVMapping) => r[rows[0].indexOf(map[key])] ?? "";
    const date = value("date"),
      description = value("description"),
      reference = value("reference");
    const currency = (
      map.currency
        ? value("currency").toUpperCase()
        : (account?.currency ?? "USD")
    ) as Currency;
    let amountMinor = 0,
      error: string | undefined;
    try {
      if (!account || account.businessId !== b.profile.id)
        throw new Error("Select a payment account.");
      if (r.length !== rows[0].length)
        throw new Error("Column count does not match header.");
      if (!validDate(date)) throw new Error("Use a valid YYYY-MM-DD date.");
      if (!description) throw new Error("Description is required.");
      if (!currencies.includes(currency) || currency !== account.currency)
        throw new Error("Currency must match the selected account.");
      amountMinor =
        parseMoney(value("amount")) * (convention === "negative" ? -1 : 1);
      if (amountMinor <= 0)
        throw new Error(
          "This is not an expense outflow under the selected sign convention.",
        );
    } catch (e) {
      error = (e as Error).message;
    }
    // Stable source identity: account + provider reference; fallback uses full normalized transaction, never amount alone.
    const fingerprint = JSON.stringify(
      reference
        ? [b.profile.id, accountId, "ref", reference]
        : [
            b.profile.id,
            accountId,
            date,
            description.trim().toLowerCase().replace(/\s+/g, " "),
            amountMinor,
            currency,
          ],
    );
    const duplicate = seen.has(fingerprint);
    const previous = seen.get(fingerprint);
    if (
      previous &&
      (previous.date !== date ||
        previous.amountMinor !== amountMinor ||
        previous.currency !== currency)
    )
      error = "Existing reference has conflicting transaction details.";
    if (!error) seen.set(fingerprint, { date, amountMinor, currency });
    return {
      row: i + 2,
      date,
      description,
      amountMinor,
      currency,
      reference,
      fingerprint,
      error,
      duplicate,
    };
  });
}
export function commitImport(
  business: BusinessData,
  rows: ImportRow[],
  accountId: string,
): BusinessData {
  let b = structuredClone(business);
  if (rows.some((x) => x.error))
    throw new Error("Resolve invalid CSV rows before importing.");
  const account = b.accounts.find(
    (a) => a.id === accountId && a.kind !== "expense",
  );
  if (!account) throw new Error("Payment account not found.");
  for (const row of rows) {
    const previous = b.payments.find((p) => p.fingerprint === row.fingerprint);
    if (previous) {
      if (
        previous.date !== row.date ||
        previous.amountMinor !== row.amountMinor ||
        previous.currency !== row.currency
      )
        throw new Error(
          "Existing reference has conflicting transaction details.",
        );
      continue;
    }
    if (
      !validDate(row.date) ||
      !row.description ||
      !Number.isSafeInteger(row.amountMinor) ||
      row.amountMinor <= 0 ||
      row.currency !== account.currency
    )
      throw new Error("Invalid import row.");
    const d = {
      ...blankDraft(b),
      vendor: row.description,
      description: row.description,
      date: row.date,
      amountMinor: row.amountMinor,
      currency: row.currency,
      accountId,
      paymentId: id(),
    };
    const p: ImportedTransaction = {
      id: d.paymentId,
      businessId: b.profile.id,
      date: row.date,
      description: row.description,
      amountMinor: row.amountMinor,
      currency: row.currency,
      accountId,
      reference: row.reference,
      fingerprint: row.fingerprint,
      draftId: d.id,
    };
    b.payments.push(p);
    b = saveDraft(b, d);
    event(
      b,
      "expense.matched",
      d.id,
      "Imported payment linked to one draft. Attach receipts to this draft.",
      [p.id],
    );
  }
  return b;
}
