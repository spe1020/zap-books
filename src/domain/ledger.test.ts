import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import {
  amountInput,
  approve,
  assertBalanced,
  blankDraft,
  categoryTotals,
  correct,
  currentPeriod,
  duplicates,
  id,
  parseMoney,
  saveDraft,
  totals,
  validation,
} from "./ledger";
import { commitImport, parseCSV, previewImport } from "./import";
import { demoAI } from "../adapters/ai";
import { MockTransport } from "../adapters/transport";
const fresh = () => seedWorkspace().businesses[0];
describe("financial invariants", () => {
  test("parses decimal inputs exactly and rejects invalid precision", () => {
    expect(parseMoney("0.29")).toBe(29);
    expect(parseMoney("100.01")).toBe(10001);
    expect(parseMoney("-4.05")).toBe(-405);
    expect(amountInput(10001)).toBe("100.01");
    for (const x of ["1.001", "NaN", "1e4", "9007199254740992", ""])
      expect(() => parseMoney(x)).toThrow();
  });
  test("all seeded journals balance, corrections net and transfers are excluded", () => {
    const b = fresh();
    b.journals.forEach((j) => expect(() => assertBalanced(b, j)).not.toThrow());
    expect(totals(b, currentPeriod()).USD).toBe(461499);
    expect(
      categoryTotals(b, currentPeriod(), "USD").reduce(
        (sum, c) => sum + c.amount,
        0,
      ),
    ).toBe(461499);
    expect(totals(b, "1900-01")).toEqual({});
    expect(
      totals(
        { ...b, journals: b.journals.filter((j) => j.kind !== "transfer") },
        currentPeriod(),
      ),
    ).toEqual(totals(b, currentPeriod()));
    const j = structuredClone(b.journals[0]);
    j.lines[0].debit++;
    expect(() => assertBalanced(b, j)).toThrow("balance");
  });
  test("required fields and invalid transitions prevent posting", () => {
    const b = fresh(),
      d = blankDraft(b);
    expect(validation(b, d).length).toBeGreaterThan(3);
    expect(() => approve(saveDraft(b, d), d.id)).toThrow();
    const posted = b.drafts.find((d) => d.status === "posted")!;
    expect(() => saveDraft(b, { ...posted, status: "draft" })).toThrow();
  });
  test("approval is balanced and can only occur once", () => {
    const b = fresh(),
      d = b.drafts.find((d) => d.vendor === "Figma")!;
    const posted = approve(b, d.id);
    expect(totals(posted, currentPeriod()).USD).toBe(471099);
    expect(() => approve(posted, d.id)).toThrow("already been posted");
    expect(posted.approvals.filter((a) => a.expenseId === d.id)).toHaveLength(
      1,
    );
    expect(b.drafts.find((x) => x.id === d.id)?.status).toBe("draft");
  });
  test("a correction reverses the original and posts replacement atomically", () => {
    const b = fresh(),
      original = b.drafts.find((d) => d.vendor === "Adobe")!;
    const before = structuredClone(
      b.journals.find((j) => j.id === original.journalId),
    );
    const replacement = {
      ...original,
      id: id(),
      status: "draft" as const,
      amountMinor: 7000,
      journalId: undefined,
    };
    const next = correct(b, original.id, replacement, "Updated invoice amount");
    expect(next.journals.find((j) => j.id === before!.id)).toEqual(before);
    expect(
      next.journals.filter((j) => j.reversesId === before!.id),
    ).toHaveLength(1);
    expect(next.drafts.find((d) => d.id === replacement.id)?.correctionOf).toBe(
      original.id,
    );
    expect(totals(next, currentPeriod()).USD).toBe(459500);
    expect(() =>
      correct(next, original.id, { ...replacement, id: id() }, "Again"),
    ).toThrow();
    expect(() =>
      correct(b, original.id, { ...replacement, amountMinor: 0 }, "Invalid"),
    ).toThrow();
    expect(b.drafts.find((d) => d.id === original.id)?.status).toBe("posted");
  });
  test("duplicate review uses vendor, date, amount, and currency; requires a reason", () => {
    const b = fresh(),
      d = b.drafts.find((d) => d.vendor === "Linear" && d.status === "draft")!;
    expect(duplicates(b, d)).toHaveLength(1);
    expect(() => approve(b, d.id)).toThrow("duplicate");
    expect(duplicates(b, { ...d, vendor: "Different vendor" })).toHaveLength(0);
    d.duplicateReason = "Different invoice and service period";
    expect(approve(b, d.id).approvals.at(-1)?.duplicateReason).toBe(
      d.duplicateReason,
    );
  });
  test("currencies never combine and a payment account must match the currency", () => {
    const b = fresh(),
      template = b.drafts[0];
    const d = {
      ...blankDraft(b),
      vendor: "Euro vendor",
      description: "EUR service",
      purpose: "Client work",
      categoryId: template.categoryId,
      amountMinor: 10001,
      currency: "EUR" as const,
      accountId: b.accounts.find((a) => a.currency === "EUR")!.id,
    };
    const next = approve(saveDraft(b, d), d.id);
    expect(totals(next, currentPeriod())).toEqual({ USD: 461499, EUR: 10001 });
    expect(validation(b, { ...d, accountId: b.accounts[0].id })).toContain(
      "Payment account currency must match the expense.",
    );
  });
  test("category lookup, totals and duplicates ignore other businesses' records", () => {
    const [a, b] = seedWorkspace().businesses;
    const figma = a.drafts.find(
      (d) => d.vendor === "Figma" && d.status === "draft",
    )!;
    // A foreign category sharing the id must not be used for posting.
    const shadowed = {
      ...a,
      categories: [
        { ...b.categories[0], id: figma.categoryId },
        ...a.categories,
      ],
    };
    const posted = approve(shadowed, figma.id);
    expect(posted.journals.at(-1)!.lines[0].accountId).toBe(
      a.categories.find((c) => c.id === figma.categoryId)!.accountId,
    );
    // Foreign journals and drafts are ignored.
    const mixed = {
      ...a,
      journals: [...a.journals, ...b.journals],
      drafts: [...a.drafts, { ...figma, id: id(), businessId: b.profile.id }],
    };
    expect(totals(mixed, currentPeriod())).toEqual(totals(a, currentPeriod()));
    expect(categoryTotals(mixed, currentPeriod(), "USD")).toEqual(
      categoryTotals(a, currentPeriod(), "USD"),
    );
    expect(duplicates(mixed, figma)).toEqual(duplicates(a, figma));
  });
  test("business separation rejects foreign drafts, accounts, evidence, and journal context", async () => {
    const [a, b] = seedWorkspace().businesses;
    expect(() => saveDraft(b, a.drafts[0])).toThrow();
    expect(() => approve(b, a.drafts[0].id)).toThrow();
    expect(
      validation(a, { ...a.drafts[0], accountId: b.accounts[0].id }),
    ).toContain("Payment account is required.");
    expect(() =>
      saveDraft(a, { ...a.drafts[0], documentIds: [b.documents[0].id] }),
    ).toThrow();
    expect(() => assertBalanced(b, a.journals[0])).toThrow();
    expect(() => demoAI.propose(b, a.drafts[0])).toThrow("another business");
    expect(demoAI.answer(b, "What needs review?", currentPeriod())).toContain(
      "Fieldwork Landscapes",
    );
    const transport = new MockTransport();
    await transport.publish([...a.events, ...b.events]);
    await transport.publish(a.events);
    const synced = await transport.synchronize(a.profile.id);
    expect(synced).toHaveLength(a.events.length);
    expect(synced.every((e) => e.businessId === a.profile.id)).toBe(true);
  });
});
describe("CSV payments and evidence", () => {
  const map = {
    date: "date",
    description: "description",
    amount: "amount",
    currency: "currency",
    reference: "reference",
  };
  test("quoted CSV, duplicate imports and equal amounts with distinct identity", () => {
    const b = fresh();
    const rows = parseCSV(
      'date,description,amount,currency,reference\r\n2026-10-20,"Tools, Inc.",-42.50,USD,R1\r\n2026-10-21,"Tools, Inc.",-42.50,USD,R2\r\n',
    );
    const preview = previewImport(b, rows, map, b.accounts[0].id, "negative");
    expect(preview.every((r) => !r.error)).toBe(true);
    const next = commitImport(b, preview, b.accounts[0].id);
    expect(next.payments.length).toBe(b.payments.length + 2);
    const repeated = commitImport(next, preview, b.accounts[0].id);
    expect(repeated.drafts.length).toBe(next.drafts.length);
    const rePreview = previewImport(
      next,
      rows,
      map,
      b.accounts[0].id,
      "negative",
    );
    expect(rePreview.every((r) => r.duplicate)).toBe(true);
    expect(() => parseCSV('a,b\n"unterminated')).toThrow();
  });
  test("whitespace before an opening quote is allowed, other text is not", () => {
    expect(parseCSV('h1,h2\na, "b"')).toEqual([
      ["h1", "h2"],
      ["a", "b"],
    ]);
    expect(parseCSV('h1,h2\na,\t\u00a0"b, c"')[1]).toEqual(["a", "b, c"]);
    expect(() => parseCSV('h1,h2\na,x "b"')).toThrow("Malformed CSV quote.");
  });
  test("malformed dates, income, mismatched currencies and conflicting references are rejected", () => {
    const b = fresh();
    const header = ["date", "description", "amount", "currency", "reference"];
    for (const r of [
      ["2026-02-30", "A", "-1", "USD", "X"],
      ["2026-10-01", "A", "10", "USD", "Y"],
      ["2026-10-01", "A", "-10", "EUR", "Z"],
    ])
      expect(
        previewImport(b, [header, r], map, b.accounts[0].id, "negative")[0]
          .error,
      ).toBeTruthy();
    const rows = [header, ["2026-10-01", "Vendor", "-10", "USD", "ABC"]];
    const next = commitImport(
      b,
      previewImport(b, rows, map, b.accounts[0].id, "negative"),
      b.accounts[0].id,
    );
    rows[1][2] = "-20";
    const conflict = previewImport(
      next,
      rows,
      map,
      b.accounts[0].id,
      "negative",
    );
    expect(conflict[0].error).toContain("conflicting");
    expect(() => commitImport(next, conflict, b.accounts[0].id)).toThrow();
  });
  test("attaching a receipt to a payment does not create another expense", () => {
    const b = fresh(),
      d = b.drafts.find((d) => d.paymentId)!;
    const next = saveDraft(b, {
      ...d,
      documentIds: [...d.documentIds, b.documents[0].id],
    });
    expect(next.drafts.length).toBe(b.drafts.length);
    expect(next.payments.length).toBe(b.payments.length);
    const posted = approve(next, d.id);
    expect(posted.journals.length).toBe(b.journals.length + 1);
    expect(validation(b, { ...d, amountMinor: d.amountMinor + 1 })).toContain(
      "Expense must match its imported payment amount, date, currency, and account.",
    );
    expect(() =>
      correct(
        posted,
        d.id,
        { ...d, id: id(), amountMinor: d.amountMinor + 1 },
        "Changed receipt total",
      ),
    ).toThrow("Expense must match its imported payment");
    expect(posted.payments).toEqual(b.payments);
    expect(posted.drafts.find((x) => x.id === d.id)?.status).toBe("posted");
  });
});

test("conflicting references in one file are rejected without partial import", () => {
  const b = fresh();
  const rows = parseCSV(
    "date,description,amount,currency,reference\n2026-10-20,Tools,-42.50,USD,REF-1\n2026-10-21,Tools,-55.00,USD,REF-1",
  );
  const preview = previewImport(
    b,
    rows,
    {
      date: "date",
      description: "description",
      amount: "amount",
      currency: "currency",
      reference: "reference",
    },
    b.accounts[0].id,
    "negative",
  );
  expect(preview[1].error).toContain("conflicting");
  expect(() => commitImport(b, preview, b.accounts[0].id)).toThrow();
  expect(b.payments).toHaveLength(1);
});

test("rows previewed for another business cannot be committed", () => {
  const [northstar, fieldwork] = seedWorkspace().businesses;
  const rows = parseCSV(
    "date,description,amount,currency,reference\n2026-10-20,Tools,-42.50,USD,REF-FW",
  );
  const preview = previewImport(
    fieldwork,
    rows,
    {
      date: "date",
      description: "description",
      amount: "amount",
      currency: "currency",
      reference: "reference",
    },
    fieldwork.accounts[0].id,
    "negative",
  );
  expect(preview[0]).toMatchObject({
    businessId: fieldwork.profile.id,
    accountId: fieldwork.accounts[0].id,
  });
  const before = structuredClone(northstar);
  expect(() =>
    commitImport(northstar, preview, northstar.accounts[0].id),
  ).toThrow(
    "Import rows were previewed for a different business or account. Preview the file again.",
  );
  expect(northstar).toEqual(before);
});
