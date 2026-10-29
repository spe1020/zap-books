import { describe, expect, test, vi } from "vitest";
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
  now,
  parseMoney,
  saveDraft,
  today,
  totals,
  validDate,
  validation,
} from "./ledger";
import { commitImport, guessMapping, parseCSV, previewImport } from "./import";
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
    expect(totals(b, currentPeriod()).USD).toBe(458559);
    expect(
      categoryTotals(b, currentPeriod(), "USD").reduce(
        (sum, c) => sum + c.amount,
        0,
      ),
    ).toBe(458559);
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
    expect(totals(posted, currentPeriod()).USD).toBe(468159);
    expect(() => approve(posted, d.id)).toThrow("already been posted");
    expect(posted.approvals.filter((a) => a.expenseId === d.id)).toHaveLength(
      1,
    );
    expect(b.drafts.find((x) => x.id === d.id)?.status).toBe("draft");
  });
  test("a saved draft cannot claim a correction or a journal (W13)", () => {
    const b = fresh(),
      original = b.drafts.find((d) => d.vendor === "Adobe")!;
    expect(original.status).toBe("posted");
    // A twin of the posted expense that names it as corrected and points at
    // its journal, without any correction having happened.
    const twin = {
      ...blankDraft(b),
      vendor: original.vendor,
      date: original.date,
      description: original.description,
      amountMinor: original.amountMinor,
      currency: original.currency,
      categoryId: original.categoryId,
      accountId: original.accountId,
      purpose: original.purpose,
      correctionOf: original.id,
      journalId: original.journalId,
    };
    const saved = saveDraft(b, twin);
    const stored = saved.drafts.find((d) => d.id === twin.id)!;
    expect(stored.correctionOf).toBeUndefined();
    expect(stored.journalId).toBeUndefined();
    expect("correctionOf" in stored).toBe(false);
    expect("journalId" in stored).toBe(false);
    // So the original still counts as its duplicate, and the original is
    // untouched.
    expect(duplicates(saved, stored).map((d) => d.id)).toContain(original.id);
    expect(validation(saved, stored)).toContain(
      "Explain why this possible duplicate is a separate expense.",
    );
    expect(saved.drafts.find((d) => d.id === original.id)).toEqual(original);
    // Editing the stored draft cannot attach them either.
    const edited = saveDraft(saved, {
      ...stored,
      correctionOf: original.id,
      journalId: "forged-journal",
    });
    expect(edited.drafts.find((d) => d.id === twin.id)).toEqual(stored);
    // Only a correction records one, and it comes from the original, not
    // from the replacement.
    const next = correct(
      b,
      original.id,
      { ...twin, correctionOf: "someone-else", journalId: "forged-journal" },
      "Reason",
    );
    const replacement = next.drafts.find((d) => d.id === twin.id)!;
    expect(replacement.correctionOf).toBe(original.id);
    expect(replacement.status).toBe("posted");
    expect(next.journals.some((j) => j.id === replacement.journalId)).toBe(
      true,
    );
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
    expect(totals(next, currentPeriod()).USD).toBe(456560);
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
    expect(totals(next, currentPeriod())).toEqual({ USD: 458559, EUR: 10001 });
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

test("today and the current period use the local date; timestamps stay UTC", () => {
  const tz = process.env.TZ;
  process.env.TZ = "America/New_York";
  vi.useFakeTimers({ toFake: ["Date"] });
  try {
    vi.setSystemTime(new Date("2026-10-15T02:00:00.000Z"));
    expect(today()).toBe("2026-10-14");
    expect(blankDraft(fresh()).date).toBe("2026-10-14");
    expect(now()).toBe("2026-10-15T02:00:00.000Z");
    vi.setSystemTime(new Date("2026-11-01T01:30:00.000Z"));
    expect(today()).toBe("2026-10-31");
    expect(currentPeriod()).toBe("2026-10");
    process.env.TZ = "Pacific/Kiritimati";
    expect(today()).toBe("2026-11-01");
  } finally {
    vi.useRealTimers();
    if (tz === undefined) delete process.env.TZ;
    else process.env.TZ = tz;
  }
});

test("import evidence can only be added by an import", () => {
  const b = fresh();
  const rows = parseCSV(
    "date,vendor,description,amount,currency,reference\n2026-10-20,Paper & Pine,Workshop materials,42.50,USD,HIST-1",
  );
  const preview = previewImport(
    b,
    rows,
    guessMapping(rows[0]),
    b.accounts[0].id,
    "positive",
    { kind: "expense-history" },
  );
  const source = preview[0].importSource!;
  const message = "Imported source evidence can only be added by an import.";
  // A new draft cannot arrive with evidence through saveDraft.
  expect(() =>
    saveDraft(b, { ...blankDraft(b), vendor: "X", importSource: source }),
  ).toThrow(message);
  // An existing draft cannot gain evidence.
  const figma = b.drafts.find(
    (d) => d.vendor === "Figma" && d.status === "draft",
  )!;
  expect(() => saveDraft(b, { ...figma, importSource: source })).toThrow(
    message,
  );
  // The history import still attaches it, and the draft stays editable.
  const imported = commitImport(b, preview, b.accounts[0].id);
  const draft = {
    ...imported.drafts.at(-1)!,
    categoryId: b.categories[0].id,
    purpose: "Client workshop",
  };
  expect(draft.importSource).toEqual(source);
  const posted = approve(saveDraft(imported, draft), draft.id);
  // Correcting an imported expense keeps its evidence on the replacement.
  const original = posted.drafts.find((d) => d.id === draft.id)!;
  const replacement = {
    ...original,
    id: id(),
    status: "draft" as const,
    amountMinor: 4000,
  };
  const corrected = correct(posted, original.id, replacement, "Fix total");
  expect(corrected.drafts.find((d) => d.id === replacement.id)).toMatchObject({
    status: "posted",
    importSource: source,
  });
});

test("dates before 1900 are rejected with the caller's existing message", () => {
  expect(validDate("1900-01-01")).toBe(true);
  for (const date of ["1899-12-31", "0000-01-01", "0000-02-29"])
    expect(validDate(date)).toBe(false);
  const b = fresh();
  const d = b.drafts.find((x) => x.vendor === "Figma" && x.status === "draft")!;
  expect(validation(b, { ...d, date: "1899-12-31" })).toContain(
    "A valid expense date is required.",
  );
  // Leading zeros and -0 stay accepted (kept on purpose).
  expect(parseMoney("00012.30")).toBe(1230);
  expect(parseMoney("-0")).toBe(-0);
});

test("W12: payment links can only be created by an import and belong to one expense", () => {
  const b = fresh();
  const rows = parseCSV(
    "date,description,amount,currency,reference\n2026-10-20,Tools,-42.50,USD,P-1",
  );
  const map = {
    date: "date",
    description: "description",
    amount: "amount",
    currency: "currency",
    reference: "reference",
  };
  const account = b.accounts[0].id;
  const imported = commitImport(
    b,
    previewImport(b, rows, map, account, "negative"),
    account,
  );
  const linked = imported.drafts.at(-1)!;
  const paymentId = linked.paymentId!;
  // A manual draft cannot claim the imported payment.
  const claim = {
    ...blankDraft(imported),
    vendor: "Other",
    description: "Other",
    purpose: "Client work",
    categoryId: b.categories[0].id,
    amountMinor: 4250,
    date: "2026-10-20",
    accountId: account,
    paymentId,
  };
  expect(() => saveDraft(imported, claim)).toThrow(
    "Payment links can only be created by an import.",
  );
  const figma = imported.drafts.find(
    (d) => d.vendor === "Figma" && d.status === "draft",
  )!;
  expect(() => saveDraft(imported, { ...figma, paymentId })).toThrow(
    "Payment links can only be created by an import.",
  );
  // The imported draft cannot drop or swap its payment.
  expect(() =>
    saveDraft(imported, { ...linked, paymentId: undefined }),
  ).toThrow("Payment links can only be created by an import.");
  expect(() => saveDraft(imported, { ...linked, paymentId: "other" })).toThrow(
    "Payment links can only be created by an import.",
  );
  // Editing it otherwise, posting it, and correcting it still work.
  const ready = {
    ...linked,
    categoryId: b.categories[0].id,
    purpose: "Client work",
  };
  const posted = approve(saveDraft(imported, ready), linked.id);
  const original = posted.drafts.find((d) => d.id === linked.id)!;
  const replacement = {
    ...original,
    id: id(),
    status: "draft" as const,
    description: "Tools, corrected",
  };
  const corrected = correct(posted, original.id, replacement, "Clarify");
  expect(corrected.drafts.find((d) => d.id === replacement.id)).toMatchObject({
    status: "posted",
    paymentId,
  });

  // The correction hands the payment to the replacement.
  expect(corrected.payments.find((p) => p.id === paymentId)!.draftId).toBe(
    replacement.id,
  );
  // A second correction of the replacement still works.
  const again = {
    ...corrected.drafts.find((d) => d.id === replacement.id)!,
    id: id(),
    status: "draft" as const,
    description: "Tools, corrected again",
  };
  const twice = correct(corrected, replacement.id, again, "Clarify again");
  expect(twice.payments.find((p) => p.id === paymentId)!.draftId).toBe(
    again.id,
  );
  // approve refuses a draft whose payment records a different expense.
  const handedOff = {
    ...imported,
    payments: imported.payments.map((p) =>
      p.id === paymentId ? { ...p, draftId: figma.id } : p,
    ),
  };
  expect(() => approve(saveDraft(handedOff, ready), linked.id)).toThrow(
    "This payment belongs to a different expense.",
  );
});
