import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import { approve, blankDraft, money, saveDraft } from "./ledger";
import {
  acceptAllFound,
  applySuggestion,
  compareWithPayment,
  fieldReason,
  finalizeExtractions,
  linesEvidence,
  recordExtraction,
  upgradeExtractionFields,
  type FormValues,
} from "./extraction";
import type {
  ExtractedField,
  ExtractedLines,
  ExtractionFields,
  ExtractionResult,
} from "./types";

const found = <T>(value: T, evidence: string): ExtractedField<T> => ({
  status: "found",
  value,
  evidence,
});
const unclear = <T>(evidence = ""): ExtractedField<T> => ({
  status: "unclear",
  value: null,
  evidence,
});
const absent = <T>(): ExtractedField<T> => ({
  status: "absent",
  value: null,
  evidence: "",
});
const noLines = (): ExtractedLines => ({
  status: "absent",
  value: null,
  lines: [],
});

function result(
  overrides: Partial<ExtractionResult["fields"]> = {},
): ExtractionResult {
  return {
    fields: {
      vendor: found("Paper & Pine", "PAPER & PINE"),
      date: found("2026-10-23", "10/23/2026"),
      total: found(4250, "TOTAL $42.50"),
      currency: found("USD", "$"),
      reference: found("R-1", "Receipt R-1"),
      subtotal: absent(),
      tax: noLines(),
      fees: noLines(),
      discounts: noLines(),
      tip: absent(),
      category: found("northstar-category-1", "Office supplies"),
      ...overrides,
    },
    warnings: [],
    model: "test-model",
    promptVersion: "test-1",
    pagesRead: 1,
    usedTextLayer: false,
    durationMs: 10,
  };
}
const blank: FormValues = {
  vendor: "",
  date: "2026-10-15",
  amount: "",
  currency: "USD",
  categoryId: "",
};

describe("applying receipt suggestions", () => {
  test("Accept applies one found field; unclear fields apply nothing", () => {
    const b = seedWorkspace().businesses[0];
    const d = blankDraft(b);
    const r = result({ total: unclear("TOTAL 42,50") });
    expect(applySuggestion(blank, d, r, "vendor").vendor).toBe("Paper & Pine");
    expect(applySuggestion(blank, d, r, "total")).toEqual(blank);
  });
  test("Accept all never overwrites a field the user edited", () => {
    const b = seedWorkspace().businesses[0];
    const d = blankDraft(b);
    const typed = { ...blank, vendor: "My vendor", amount: "40.00" };
    const { values, applied } = acceptAllFound(
      typed,
      d,
      result(),
      new Set(["vendor", "total"]),
    );
    expect(values).toEqual({
      ...typed,
      date: "2026-10-23",
      categoryId: "northstar-category-1",
    });
    expect(applied).toEqual(["date", "currency", "category"]);
  });
  test("fields locked to an imported payment are compared, never applied", () => {
    const b = seedWorkspace().businesses[0];
    const d = b.drafts.find((x) => x.paymentId)!;
    const payment = b.payments.find((p) => p.id === d.paymentId)!;
    const values = { ...blank, amount: "1.00" };
    const { values: after, applied } = acceptAllFound(
      values,
      d,
      result(),
      new Set(),
    );
    expect(applied).toEqual(["vendor", "category"]);
    expect(after.amount).toBe("1.00");
    expect(applySuggestion(values, d, result(), "total")).toEqual(values);
    const fmt = (n: number, c: string) => money(n, c as "USD");
    const same = compareWithPayment(
      result({ total: found(payment.amountMinor, "TOTAL") }),
      payment,
      fmt,
    );
    expect(same[0]).toEqual({
      field: "total",
      matches: true,
      message: `Receipt total ${fmt(payment.amountMinor, "USD")} matches the payment`,
    });
    const other = compareWithPayment(
      result({ total: found(payment.amountMinor + 250, "TOTAL") }),
      payment,
      fmt,
    );
    expect(other[0].matches).toBe(false);
    expect(other[0].message).toBe(
      `Receipt total ${fmt(payment.amountMinor + 250, "USD")} doesn't match the payment (${fmt(payment.amountMinor, "USD")}). Check that this is the right receipt.`,
    );
  });
});

describe("recording what the reader proposed", () => {
  test("outcomes are finalized when the draft is approved", () => {
    let b = seedWorkspace().businesses[0];
    const draft = {
      ...blankDraft(b),
      vendor: "Paper & Pine Co",
      date: "2026-10-23",
      amountMinor: 4250,
      categoryId: b.categories[0].id,
      description: "Workshop materials",
      purpose: "Client workshop",
    };
    b = saveDraft(b, draft);
    const r = result({
      tax: {
        status: "unclear",
        value: null,
        lines: [{ amount: null, evidence: "TAX ?" }],
      },
    });
    b = recordExtraction(b, {
      draftId: draft.id,
      documentId: "doc-1",
      result: r,
      accepted: ["vendor", "total", "date"],
    });
    // Reading again replaces the pending record.
    b = recordExtraction(b, {
      draftId: draft.id,
      documentId: "doc-1",
      result: { ...r, fields: { ...r.fields, total: unclear("TOTAL") } },
      accepted: ["vendor", "date"],
    });
    expect(b.extractions).toHaveLength(1);
    expect(b.extractions[0].outcomes).toBeNull();
    // Unposted drafts keep pending outcomes.
    expect(finalizeExtractions(b, draft.id)).toEqual(b);
    const posted = finalizeExtractions(approve(b, draft.id), draft.id);
    expect(posted.extractions[0]).toMatchObject({
      businessId: b.profile.id,
      draftId: draft.id,
      documentId: "doc-1",
      model: "test-model",
      promptVersion: "test-1",
      accepted: ["vendor", "date"],
      outcomes: {
        vendor: "edited",
        date: "accepted",
        total: "unclear",
        currency: "ignored",
        category: "ignored",
      },
    });
  });
});

test("unclear fields find the warning that explains them", () => {
  const r = {
    ...result({ total: unclear("TOTAL $45.00"), subtotal: unclear("SUB") }),
    warnings: [
      "The subtotal couldn't be read as an exact amount. Check it on the receipt.",
      "Subtotal $50.00, tax $3.00 and fees $1.50 add up to $54.50, but the total reads $55.50. Check the amount.",
      "The fees lines include a negative amount. Check them on the receipt.",
    ],
  };
  expect(fieldReason(r, "total")).toBe(r.warnings[1]);
  expect(fieldReason(r, "subtotal")).toBe(r.warnings[0]);
  expect(fieldReason(r, "fees")).toBe(r.warnings[2]);
  expect(fieldReason(r, "tax")).toBeUndefined();
});

test("an unclear total explains a missing tip or a refund line", () => {
  for (const warning of [
    "Subtotal $40.00 and tax $2.50 add up to $42.50, but the total reads $51.00. If you added a tip, check the amount matches what you paid.",
    "The subtotal reads $40.00, but the total reads $45.00. If you added a tip, check the amount matches what you paid.",
    "The only amount is a payment or refund line (−$24.00). Enter the total yourself.",
  ]) {
    const r = {
      ...result({ total: unclear("TOTAL") }),
      warnings: [warning],
    };
    expect(fieldReason(r, "total")).toBe(warning);
    expect(fieldReason(r, "tip")).toBeUndefined();
  }
});

test("records saved before tax was a list load as lists", () => {
  const legacy = {
    ...result().fields,
    tax: found(250, "TAX $2.50"),
  } as Record<string, unknown>;
  delete legacy.fees;
  delete legacy.discounts;
  delete legacy.tip;
  const fields = upgradeExtractionFields(legacy as unknown as ExtractionFields);
  expect(fields.tax).toEqual({
    status: "found",
    value: 250,
    lines: [{ amount: 250, evidence: "TAX $2.50" }],
  });
  expect(fields.fees).toEqual(noLines());
  expect(fields.discounts).toEqual(noLines());
  expect(fields.tip).toEqual(absent());
  expect(
    upgradeExtractionFields({
      ...result().fields,
      tax: unclear("TAX ?") as unknown as ExtractedLines,
    }).tax,
  ).toEqual({
    status: "unclear",
    value: null,
    lines: [{ amount: null, evidence: "TAX ?" }],
  });
  // Records already in the list shape are unchanged.
  const current = result().fields;
  expect(upgradeExtractionFields(current)).toEqual(current);
});

test("a list's evidence joins its lines", () => {
  expect(
    linesEvidence({
      status: "found",
      value: 45,
      lines: [
        { amount: 0, evidence: "TAX 0.00" },
        { amount: 45, evidence: "TAX 0.45" },
      ],
    }),
  ).toBe("TAX 0.00 · TAX 0.45");
});
