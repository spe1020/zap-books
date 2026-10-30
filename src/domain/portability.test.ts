import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import { encodeCSV, exportRecords } from "./export";
import { commitImport, guessMapping, parseCSV, previewImport } from "./import";
import {
  approve,
  blankDraft,
  correct,
  currentPeriod,
  id,
  parseMoney,
  saveDraft,
  totals,
  writeDraft,
} from "./ledger";
const filters = { from: "", to: "", currency: "" as const };
const historyCSV =
  'Expense Date,Vendor Name,Description,Expense Amount,Currency Code,Expense Category,Reference Number,Business Purpose\n10/23/2026,Paper & Pine,Workshop materials,"1,234.56",USD,Office & supplies,QB-901,Client workshop';
function migration() {
  const b = seedWorkspace().businesses[0],
    rows = parseCSV(historyCSV);
  const preview = previewImport(
    b,
    rows,
    guessMapping(rows[0]),
    b.accounts[0].id,
    "positive",
    {
      kind: "expense-history",
      provider: "quickbooks",
      dateFormat: "mdy",
      filename: "expenses.csv",
    },
  );
  return { b, preview, account: b.accounts[0].id };
}
describe("portable financial exports", () => {
  test("exports current expenses once, excludes drafts/transfers and keeps corrected history in the journal", () => {
    const b = seedWorkspace().businesses[0];
    const csv = exportRecords(b, "expenses", filters);
    const rows = parseCSV(csv.csv);
    const amount = rows[0].indexOf("Amount");
    expect(
      rows.slice(1).reduce((sum, r) => sum + parseMoney(r[amount]), 0),
    ).toBe(totals(b, currentPeriod()).USD);
    expect(csv.rows).toHaveLength(6);
    expect(csv.rows.filter((r) => r[4] === "Atlas Creative")).toHaveLength(1);
    const journal = exportRecords(b, "journal", filters);
    expect(journal.rows.some((r) => r[4] === "reversal" && r[6])).toBe(true);
    expect(journal.rows.some((r) => r[4] === "transfer")).toBe(true);
    for (const currency of ["USD", "EUR", "GBP"]) {
      const rows = journal.rows.filter((r) => r[12] === currency);
      expect(rows.reduce((n, r) => n + parseMoney(r[10]), 0)).toBe(
        rows.reduce((n, r) => n + parseMoney(r[11]), 0),
      );
    }
  });
  test("exports only the selected business, dates, and currency", () => {
    const [b, other] = seedWorkspace().businesses;
    const d = {
      ...blankDraft(b),
      vendor: "Euro vendor",
      description: "Service",
      purpose: "Client work",
      categoryId: b.categories[0].id,
      accountId: b.accounts.find((a) => a.currency === "EUR")!.id,
      currency: "EUR" as const,
      amountMinor: 1234,
    };
    const next = approve(saveDraft(b, d), d.id);
    const rows = exportRecords(next, "expenses", {
      ...filters,
      currency: "EUR",
    }).rows;
    expect(rows).toHaveLength(1);
    expect(rows[0][6]).toBe("12.34");
    const mixed = {
      ...next,
      drafts: [...next.drafts, ...other.drafts],
      journals: [...next.journals, ...other.journals],
    };
    for (const format of ["expenses", "journal"] as const)
      expect(
        exportRecords(mixed, format, filters).rows.every(
          (r) => r[0] === b.profile.id,
        ),
      ).toBe(true);
    expect(
      exportRecords(b, "expenses", {
        ...filters,
        from: "1900-01-01",
        to: "1900-02-01",
      }).rows,
    ).toHaveLength(0);
    expect(() =>
      exportRecords(b, "expenses", { ...filters, from: "2026-02-30" }),
    ).toThrow();
    expect(() =>
      exportRecords(b, "expenses", {
        ...filters,
        from: "2026-12-31",
        to: "2026-01-01",
      }),
    ).toThrow();
  });
  test("CSV quotes delimiters and prevents spreadsheet formulas in user-controlled text", () => {
    const b = seedWorkspace().businesses[0];
    const d = b.drafts.find((d) => d.status === "posted")!;
    d.vendor = '=HYPERLINK("https://invalid.example", "open")';
    d.description = 'Quoted "receipt", with\nnewline';
    const rows = parseCSV(exportRecords(b, "expenses", filters).csv);
    const row = rows.find((r) => r[2] === d.id)!;
    expect(row[4]).toBe("'" + d.vendor);
    expect(row[5]).toBe(d.description);
  });
  test("CSV neutralizes a leading | and leaves signed numbers alone (WEB-6)", () => {
    const cells = [
      "|cmd",
      " |x",
      "-12.50",
      "+1",
      "-1,000.00",
      "-x",
      "+1 x",
      " -5",
      "=1",
    ];
    const [row] = parseCSV(encodeCSV([["header"], cells]).slice(1)).slice(1);
    expect(row).toEqual([
      "'|cmd",
      "' |x",
      "-12.50",
      "+1",
      "-1,000.00",
      "'-x",
      "'+1 x",
      "' -5",
      "'=1",
    ]);
  });
});
describe("expense-history migration", () => {
  test("maps common export columns, explicit regional dates, category and exact minor units without claiming bank evidence", () => {
    const { b, preview, account } = migration();
    expect(preview[0].error).toBeUndefined();
    expect(preview[0].date).toBe("2026-10-23");
    expect(preview[0].amountMinor).toBe(123456);
    const next = commitImport(b, preview, account),
      draft = next.drafts.at(-1)!;
    expect(draft.vendor).toBe("Paper & Pine");
    expect(draft.categoryId).toBe(b.categories[1].id);
    expect(draft.purpose).toBe("Client workshop");
    expect(draft.status).toBe("draft");
    expect(draft.paymentId).toBeUndefined();
    expect(next.payments.length).toBe(b.payments.length);
    expect(next.journals).toEqual(b.journals);
    expect(draft.importSource?.provider).toBe("quickbooks");
  });
  test("repeat import stays safe after edits, posting and correction, and original source remains immutable", () => {
    const { b, preview, account } = migration();
    const initial = commitImport(b, preview, account);
    let next = initial;
    const d = next.drafts.at(-1)!;
    next = saveDraft(next, { ...d, vendor: "Edited vendor" });
    next = approve(next, d.id);
    const original = next.drafts.find((x) => x.id === d.id)!;
    next = correct(
      next,
      d.id,
      { ...original, id: id(), status: "draft", amountMinor: 123000 },
      "Corrected expense total",
    );
    expect(commitImport(next, preview, account).drafts).toEqual(next.drafts);
    expect(() =>
      saveDraft(initial, {
        ...d,
        importSource: { ...d.importSource!, reference: "different" },
      }),
    ).toThrow("cannot be changed");
    const duplicate = {
      ...d,
      id: id(),
      vendor: "Unrelated vendor",
      status: "draft" as const,
    };
    expect(() => saveDraft(next, duplicate)).toThrow(
      "Imported source evidence can only be added by an import.",
    );
    // Even through the import path, approval refuses a second posting.
    expect(() =>
      approve(
        writeDraft(next, duplicate, { allowImportSource: true }),
        duplicate.id,
      ),
    ).toThrow("already has a posted expense");
  });
  test("unknown categories stay unresolved, conflicts and unsupported document types block import", () => {
    const { b, account } = migration();
    let rows = parseCSV(
      historyCSV.replace("Office & supplies", "Legacy category"),
    );
    let preview = previewImport(
      b,
      rows,
      guessMapping(rows[0]),
      account,
      "positive",
      { kind: "expense-history", dateFormat: "mdy" },
    );
    expect(preview[0].categoryId).toBeUndefined();
    expect(preview[0].warnings?.join(" ")).toContain("Legacy category");
    const unresolved = commitImport(b, preview, account);
    expect(() => approve(unresolved, unresolved.drafts.at(-1)!.id)).toThrow(
      "Category is required",
    );
    rows = parseCSV(
      "Date,Name,Memo/Description,Amount,Transaction Type,Num\n23/10/2026,Vendor,Old bill,12.00,Bill,Z1",
    );
    preview = previewImport(
      b,
      rows,
      guessMapping(rows[0]),
      account,
      "positive",
      { kind: "expense-history", dateFormat: "dmy", provider: "zoho" },
    );
    expect(preview[0].error).toContain("Only paid expense");
    const repeated = parseCSV(
      historyCSV +
        "\n10/23/2026,Paper & Pine,Workshop materials,500.00,USD,Office & supplies,QB-901,Client workshop",
    );
    preview = previewImport(
      b,
      repeated,
      guessMapping(repeated[0]),
      account,
      "positive",
      { kind: "expense-history", dateFormat: "mdy" },
    );
    expect(preview[1].error).toContain("conflicting");
  });
  test("source evidence and approval remain business scoped", () => {
    const { b, preview, account } = migration();
    const other = seedWorkspace().businesses[1];
    expect(() => commitImport(other, preview, other.accounts[0].id)).toThrow(
      "Import rows were previewed for a different business or account",
    );
    const next = commitImport(b, preview, account);
    expect(() =>
      saveDraft(other, {
        ...next.drafts.at(-1)!,
        businessId: other.profile.id,
        accountId: other.accounts[0].id,
        categoryId: other.categories[0].id,
      }),
    ).toThrow("another business");
  });
  test("our expense CSV maps back into reviewable drafts with preserved exact amounts", () => {
    const b = seedWorkspace().businesses[0],
      csv = exportRecords(b, "expenses", filters),
      rows = parseCSV(csv.csv);
    const preview = previewImport(
      b,
      rows,
      guessMapping(rows[0]),
      b.accounts[0].id,
      "positive",
      { kind: "expense-history" },
    );
    expect(preview.every((r) => !r.error)).toBe(true);
    expect(preview.reduce((n, r) => n + r.amountMinor, 0)).toBe(458559);
  });
});

test("a mixed-account migration is blocked before any drafts are created", () => {
  const b = seedWorkspace().businesses[0];
  const rows = parseCSV(
    "Date,Vendor,Description,Amount,Currency,Payment account,Reference\n2026-10-01,A,Materials,12.00,USD,Checking,A1\n2026-10-02,B,Service,15.00,USD,Credit card,B1",
  );
  const preview = previewImport(
    b,
    rows,
    guessMapping(rows[0]),
    b.accounts[0].id,
    "positive",
    { kind: "expense-history" },
  );
  expect(
    preview.every((r) => r.error?.includes("multiple payment accounts")),
  ).toBe(true);
  expect(() => commitImport(b, preview, b.accounts[0].id)).toThrow(
    "Resolve invalid",
  );
  expect(b.drafts).toHaveLength(seedWorkspace().businesses[0].drafts.length);
});
