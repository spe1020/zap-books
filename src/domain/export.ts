import type { BusinessData, Currency } from "./types";
import { amountInput, validDate } from "./ledger";
import { lineCurrency } from "./bitcoin";
import {
  accountKindLabels,
  balanceMeanings,
  balances,
  transferTypeLabels,
} from "./transfers";
export type ExportFormat = "expenses" | "journal";
export interface ExportFilter {
  from: string;
  to: string;
  currency: Currency | "";
}
export interface CSVExport {
  headers: string[];
  rows: string[][];
  filename: string;
  csv: string;
}
/**
 * Quotes every cell and neutralizes text a spreadsheet would run as a
 * formula by prefixing `'`: a cell starting (after any whitespace or BOM)
 * with `=`, `+`, `-`, `@` or `|`, or with a tab, CR or LF. A sign followed
 * only by digits, `.` and `,` (`-12.50`, `+1`) is left alone: a spreadsheet
 * reads it as a number, and a formula needs a name, a reference or an
 * operator. Other signed text (`-x`, `+1 x`, ` -5`) is still prefixed,
 * since a spreadsheet would evaluate it.
 */
function csvCell(value: string) {
  const formula =
    (/^[\s\uFEFF]*[=+\-@|]/.test(value) && !/^[+-]\d[\d.,]*$/.test(value)) ||
    /^[\t\r\n]/.test(value);
  const literal = formula ? `'${value}` : value;
  return `"${literal.replaceAll('"', '""')}"`;
}
export function encodeCSV(rows: string[][]) {
  return (
    "\uFEFF" + rows.map((r) => r.map(csvCell).join(",")).join("\r\n") + "\r\n"
  );
}
export function exportRecords(
  b: BusinessData,
  format: ExportFormat,
  filter: ExportFilter,
): CSVExport {
  if (
    (filter.from && !validDate(filter.from)) ||
    (filter.to && !validDate(filter.to))
  )
    throw new Error("Choose valid export dates.");
  if (filter.from && filter.to && filter.from > filter.to)
    throw new Error("The start date must be on or before the end date.");
  const inRange = (date: string, currency: Currency) =>
    (!filter.from || date >= filter.from) &&
    (!filter.to || date <= filter.to) &&
    (!filter.currency || currency === filter.currency);
  const accountOf = (id: string) =>
    b.accounts.find((a) => a.id === id && a.businessId === b.profile.id);
  const account = (id: string) => accountOf(id)?.name ?? "";
  const category = (id: string | undefined) =>
    b.categories.find((c) => c.id === id && c.businessId === b.profile.id)
      ?.name ?? "";
  let headers: string[], rows: string[][];
  if (format === "expenses") {
    headers = [
      "Business ID",
      "Business",
      "Expense ID",
      "Date",
      "Vendor",
      "Description",
      "Amount",
      "Currency",
      "Category",
      "Payment account",
      "Business purpose",
      "Corrects expense ID",
      "Source system",
      "Source reference",
      "Receipt count",
      "Paid personally",
    ];
    rows = b.drafts
      .filter(
        (d) =>
          d.businessId === b.profile.id &&
          d.status === "posted" &&
          inRange(d.date, d.currency),
      )
      .sort((a, z) => a.date.localeCompare(z.date) || a.id.localeCompare(z.id))
      .map((d) => {
        const payment = b.payments.find(
          (p) => p.id === d.paymentId && p.businessId === b.profile.id,
        );
        return [
          b.profile.id,
          b.profile.name,
          d.id,
          d.date,
          d.vendor,
          d.description,
          amountInput(d.amountMinor, d.currency),
          d.currency,
          category(d.categoryId),
          account(d.accountId),
          d.purpose,
          d.correctionOf ?? "",
          d.importSource?.provider ?? (payment ? "Payment CSV" : ""),
          d.importSource?.reference ?? payment?.reference ?? "",
          String(d.documentIds.length),
          // An expense paid from a loan account is owed to the owner; from
          // an equity account, it's the owner's contribution.
          { liability: "Loan", equity: "Contribution" }[
            accountOf(d.accountId)?.kind as string
          ] ?? "",
        ];
      });
  } else {
    headers = [
      "Business ID",
      "Business",
      "Journal ID",
      "Date",
      "Type",
      "Expense ID",
      "Reverses journal ID",
      "Account ID",
      "Account",
      "Category",
      "Debit",
      "Credit",
      "Currency",
      "Memo",
      "Recorded at",
      "Transfer type",
      "Purpose",
      "Reference",
      "Corrects journal ID",
    ];
    rows = b.journals
      // An exchange is in both of its currencies.
      .filter(
        (j) =>
          j.businessId === b.profile.id &&
          (inRange(j.date, j.currency) ||
            (j.transferType === "exchange" &&
              !!j.fiatValue &&
              inRange(j.date, j.fiatValue.currency))),
      )
      .sort(
        (a, z) =>
          a.date.localeCompare(z.date) ||
          a.createdAt.localeCompare(z.createdAt),
      )
      .flatMap((j) =>
        j.lines.map((l) => {
          const currency = lineCurrency(j, accountOf(l.accountId)?.kind);
          return [
            b.profile.id,
            b.profile.name,
            j.id,
            j.date,
            j.kind,
            j.expenseId ?? "",
            j.reversesId ?? "",
            l.accountId,
            account(l.accountId),
            category(l.categoryId),
            amountInput(l.debit, currency),
            amountInput(l.credit, currency),
            currency,
            j.memo,
            j.createdAt,
            j.transferType ? transferTypeLabels[j.transferType] : "",
            j.purpose ?? "",
            j.reference ?? "",
            j.correctionOf ?? "",
          ];
        }),
      );
  }
  return {
    headers,
    rows,
    filename: `${fileStem(b)}-${format}-${filter.from || "start"}-to-${filter.to || "latest"}-${filter.currency || "all-currencies"}.csv`,
    csv: encodeCSV([headers, ...rows]),
  };
}

const fileStem = (b: BusinessData) =>
  b.profile.name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "") || "business";

/**
 * Every bank, card, loan and equity account's balance in `currency` on
 * `asOf`, one row each, from `balances`. A card's balance is what's owed on
 * it, and a loan's what's owed to the owner.
 */
export function exportBalances(
  b: BusinessData,
  asOf: string,
  currency: Currency,
): CSVExport {
  const headers = [
    "Business ID",
    "Business",
    "As of",
    "Account ID",
    "Account",
    "Account type",
    "Balance",
    "Currency",
    "Balance is",
  ];
  const rows = balances(b, asOf, currency).map((r) => [
    b.profile.id,
    b.profile.name,
    asOf,
    r.accountId,
    r.name,
    accountKindLabels[r.kind],
    amountInput(r.balance, currency),
    currency,
    balanceMeanings[r.kind as keyof typeof balanceMeanings],
  ]);
  return {
    headers,
    rows,
    filename: `${fileStem(b)}-balances-${asOf}-${currency}.csv`,
    csv: encodeCSV([headers, ...rows]),
  };
}
