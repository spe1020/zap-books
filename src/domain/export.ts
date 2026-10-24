import type { BusinessData, Currency } from "./types";
import { amountInput, validDate } from "./ledger";
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
function csvCell(value: string) {
  // Quote delimiters and neutralize formulas in spreadsheet applications.
  const literal =
    /^[\s\uFEFF]*[=+\-@]/.test(value) || /^[\t\r\n]/.test(value)
      ? `'${value}`
      : value;
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
  const account = (id: string) =>
    b.accounts.find((a) => a.id === id && a.businessId === b.profile.id)
      ?.name ?? "";
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
          amountInput(d.amountMinor),
          d.currency,
          category(d.categoryId),
          account(d.accountId),
          d.purpose,
          d.correctionOf ?? "",
          d.importSource?.provider ?? (payment ? "Payment CSV" : ""),
          d.importSource?.reference ?? payment?.reference ?? "",
          String(d.documentIds.length),
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
    ];
    rows = b.journals
      .filter(
        (j) => j.businessId === b.profile.id && inRange(j.date, j.currency),
      )
      .sort(
        (a, z) =>
          a.date.localeCompare(z.date) ||
          a.createdAt.localeCompare(z.createdAt),
      )
      .flatMap((j) =>
        j.lines.map((l) => [
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
          amountInput(l.debit),
          amountInput(l.credit),
          j.currency,
          j.memo,
          j.createdAt,
        ]),
      );
  }
  const name =
    b.profile.name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "business";
  return {
    headers,
    rows,
    filename: `${name}-${format}-${filter.from || "start"}-to-${filter.to || "latest"}-${filter.currency || "all-currencies"}.csv`,
    csv: encodeCSV([headers, ...rows]),
  };
}
