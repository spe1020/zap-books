import type {
  BusinessData,
  Currency,
  ImportedTransaction,
  ImportedExpenseSource,
  ImportProvider,
} from "./types";
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
  vendor?: string;
  category?: string;
  purpose?: string;
  transactionType?: string;
  sourceAccount?: string;
}
export type DateFormat = "iso" | "mdy" | "dmy";
export interface ImportOptions {
  kind?: "payment" | "expense-history";
  provider?: ImportProvider;
  dateFormat?: DateFormat;
  filename?: string;
}
export interface ImportRow {
  row: number;
  businessId: string;
  accountId: string;
  date: string;
  description: string;
  amountMinor: number;
  currency: Currency;
  reference: string;
  fingerprint: string;
  error?: string;
  duplicate: boolean;
  vendor?: string;
  categoryId?: string;
  purpose?: string;
  warnings?: string[];
  importSource?: ImportedExpenseSource;
}
function importFingerprint(
  businessId: string,
  accountId: string,
  row: Pick<
    ImportRow,
    "date" | "description" | "amountMinor" | "currency" | "reference" | "vendor"
  >,
  provider?: ImportProvider,
) {
  const scope = [
    businessId,
    accountId,
    ...(provider ? ["expense-history", provider] : []),
  ];
  return JSON.stringify(
    row.reference
      ? [...scope, "ref", row.reference]
      : [
          ...scope,
          row.date,
          row.description.trim().toLowerCase().replace(/\s+/g, " "),
          ...(provider
            ? [(row.vendor ?? row.description).trim().toLowerCase()]
            : []),
          row.amountMinor,
          row.currency,
        ],
  );
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
      // Whitespace before an opening quote (`a, "b"`) is dropped; any other
      // text before it is an error.
      if (cell.trim() || closed) throw new Error("Malformed CSV quote.");
      cell = "";
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
export function guessMapping(headers: string[]): CSVMapping {
  const lookup = (names: string[]) =>
    headers.find((h) =>
      names.includes(h.toLowerCase().replace(/[^a-z0-9]/g, "")),
    ) ?? "";
  return {
    date: lookup(["date", "transactiondate", "expensedate"]),
    description: lookup([
      "description",
      "memodescription",
      "memo",
      "expensedescription",
      "merchant",
      "vendor",
    ]),
    amount: lookup(["amount", "total", "totalamount", "expenseamount"]),
    currency: lookup(["currency", "currencycode"]),
    reference: lookup([
      "expenseid",
      "transactionid",
      "reference",
      "referencenumber",
      "referenceno",
      "id",
      "num",
    ]),
    vendor: lookup(["vendor", "vendorname", "merchant", "payee", "name"]),
    category: lookup(["category", "expensecategory", "expenseaccount"]),
    purpose: lookup(["businesspurpose", "purpose"]),
    transactionType: lookup(["transactiontype", "type"]),
    sourceAccount: lookup([
      "paymentaccount",
      "paymentaccountname",
      "paidthrough",
      "bankaccount",
    ]),
  };
}
export function normalizeImportDate(value: string, format: DateFormat): string {
  if (format === "iso") return value;
  const match = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(value);
  if (!match) return "";
  const [, first, second, year] = match;
  const month = format === "mdy" ? first : second,
    day = format === "mdy" ? second : first;
  return `${year}-${month.padStart(2, "0")}-${day.padStart(2, "0")}`;
}
function importAmount(value: string) {
  // Permit explicit comma thousands groups, never infer a decimal convention.
  if (/^-?\d{1,3}(,\d{3})+(\.\d{1,2})?$/.test(value))
    return parseMoney(value.replaceAll(",", ""));
  return parseMoney(value);
}
export function previewImport(
  b: BusinessData,
  rows: string[][],
  map: CSVMapping,
  accountId: string,
  convention: "negative" | "positive",
  options: ImportOptions = {},
): ImportRow[] {
  const account = b.accounts.find(
    (x) => x.id === accountId && x.kind !== "expense",
  );
  const history = options.kind === "expense-history";
  const provider = options.provider ?? "csv";
  const existing = history
    ? b.drafts.flatMap((d) => (d.importSource ? [d.importSource] : []))
    : b.payments;
  const seen = new Map(
    existing.map((p) => [
      p.fingerprint,
      { date: p.date, amountMinor: p.amountMinor, currency: p.currency },
    ]),
  );
  const accountColumn = rows[0].indexOf(map.sourceAccount ?? "");
  const sourceAccounts = new Set(
    rows
      .slice(1)
      .map((row) => (row[accountColumn] ?? "").trim().toLowerCase())
      .filter(Boolean),
  );
  let combinedAmount = 0;
  return rows.slice(1).map((r, i) => {
    const value = (key: keyof CSVMapping) =>
      r[rows[0].indexOf(map[key] ?? "")] ?? "";
    const date = normalizeImportDate(
        value("date"),
        options.dateFormat ?? "iso",
      ),
      description = value("description"),
      reference = value("reference");
    const vendor = map.vendor ? value("vendor") : description;
    const categoryName = value("category"),
      purpose = value("purpose");
    const category = b.categories.find(
      (c) =>
        c.businessId === b.profile.id &&
        c.name.toLowerCase() === categoryName.toLowerCase(),
    );
    const currency = (
      map.currency
        ? value("currency").toUpperCase()
        : (account?.currency ?? "USD")
    ) as Currency;
    const warnings: string[] = [];
    if (history && !vendor) warnings.push("Add a vendor during review.");
    if (history && !category)
      warnings.push(
        categoryName
          ? `Unmapped category: ${categoryName}. Choose it during review.`
          : "Choose a category during review.",
      );
    if (history && !purpose)
      warnings.push("Add the business purpose during review.");
    let amountMinor = 0,
      error: string | undefined;
    try {
      if (sourceAccounts.size > 1)
        throw new Error(
          "File contains multiple payment accounts. Export one account at a time.",
        );
      if (!account || account.businessId !== b.profile.id)
        throw new Error("Select a payment account.");
      if (r.length !== rows[0].length)
        throw new Error("Column count does not match header.");
      if (!map.date || !map.description || !map.amount)
        throw new Error("Map date, description, and amount.");
      if (!validDate(date))
        throw new Error(
          "Date does not match the selected format or is invalid.",
        );
      if (!description) throw new Error("Description is required.");
      if (!currencies.includes(currency) || currency !== account.currency)
        throw new Error("Currency must match the selected account.");
      if (
        map.transactionType &&
        ![
          "expense",
          "check",
          "cheque",
          "credit card expense",
          "credit card charge",
          "cash expense",
        ].includes(value("transactionType").toLowerCase())
      )
        throw new Error(
          "Only paid expense rows are supported; remove bills, income, transfers, and subtotal rows.",
        );
      amountMinor =
        importAmount(value("amount")) * (convention === "negative" ? -1 : 1);
      if (amountMinor <= 0)
        throw new Error(
          "This is not an expense outflow under the selected sign convention.",
        );
    } catch (e) {
      error = (e as Error).message;
    }
    const fingerprint = importFingerprint(
      b.profile.id,
      accountId,
      { date, description, vendor, amountMinor, currency, reference },
      history ? provider : undefined,
    );
    const previous = seen.get(fingerprint),
      duplicate = seen.has(fingerprint);
    if (
      previous &&
      (previous.date !== date ||
        previous.amountMinor !== amountMinor ||
        previous.currency !== currency)
    )
      error = "Existing reference has conflicting transaction details.";
    if (!error && !duplicate) {
      combinedAmount += amountMinor;
      if (!Number.isSafeInteger(combinedAmount))
        error = "Combined import amount exceeds the supported range.";
    }
    if (!error) seen.set(fingerprint, { date, amountMinor, currency });
    const importSource: ImportedExpenseSource | undefined = history
      ? {
          id: id(),
          businessId: b.profile.id,
          provider,
          reference,
          fingerprint,
          date,
          amountMinor,
          currency,
          vendor,
          category: categoryName,
          filename: options.filename ?? "",
        }
      : undefined;
    return {
      row: i + 2,
      businessId: b.profile.id,
      accountId,
      date,
      description,
      amountMinor,
      currency,
      reference,
      fingerprint,
      error,
      duplicate,
      vendor,
      categoryId: category?.id,
      purpose,
      warnings,
      importSource,
    };
  });
}
export function commitImport(
  business: BusinessData,
  rows: ImportRow[],
  accountId: string,
): BusinessData {
  let b = structuredClone(business);
  if (rows.some((r) => r.error))
    throw new Error("Resolve invalid CSV rows before importing.");
  const account = b.accounts.find(
    (a) =>
      a.id === accountId &&
      a.kind !== "expense" &&
      a.businessId === b.profile.id,
  );
  if (!account) throw new Error("Payment account not found.");
  // Checked for every row before any is processed, so nothing is imported.
  if (
    rows.some((r) => r.businessId !== b.profile.id || r.accountId !== accountId)
  )
    throw new Error(
      "Import rows were previewed for a different business or account. Preview the file again.",
    );
  for (const row of rows) {
    if (
      row.importSource &&
      (row.importSource.businessId !== b.profile.id ||
        row.importSource.fingerprint !== row.fingerprint ||
        row.importSource.amountMinor !== row.amountMinor ||
        row.importSource.date !== row.date ||
        row.importSource.currency !== row.currency)
    )
      throw new Error("Invalid import source.");
    // Recompute before duplicate lookup: even a repeated row must belong to
    // this destination. Never silently reassign a preview from another account.
    if (
      row.fingerprint !==
      importFingerprint(
        b.profile.id,
        accountId,
        row,
        row.importSource?.provider,
      )
    )
      throw new Error(
        "Import preview does not match this business or payment account. Preview the file again.",
      );
    const existing = row.importSource
      ? b.drafts.flatMap((d) => (d.importSource ? [d.importSource] : []))
      : b.payments;
    const previous = existing.find((p) => p.fingerprint === row.fingerprint);
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
    const draft = {
      ...blankDraft(b),
      vendor: row.vendor ?? row.description,
      description: row.description,
      date: row.date,
      amountMinor: row.amountMinor,
      currency: row.currency,
      accountId,
      categoryId: row.categoryId ?? "",
      purpose: row.purpose ?? "",
      ...(row.importSource
        ? { importSource: row.importSource }
        : { paymentId: id() }),
    };
    if (draft.paymentId) {
      const payment: ImportedTransaction = {
        id: draft.paymentId,
        businessId: b.profile.id,
        date: row.date,
        description: row.description,
        amountMinor: row.amountMinor,
        currency: row.currency,
        accountId,
        reference: row.reference,
        fingerprint: row.fingerprint,
        draftId: draft.id,
      };
      b.payments.push(payment);
    }
    b = saveDraft(b, draft);
    event(
      b,
      "expense.matched",
      draft.id,
      row.importSource
        ? `Expense history imported from ${row.importSource.provider}; source record retained for review. This is not bank payment evidence.`
        : "Imported payment linked to one draft. Attach receipts to this draft.",
      [row.importSource?.id ?? draft.paymentId!],
    );
  }
  return b;
}
