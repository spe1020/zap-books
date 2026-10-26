import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import { approve, blankDraft, money, saveDraft } from "./ledger";
import {
  acceptAllFound,
  applySuggestion,
  compareWithPayment,
  fieldReason,
  finalizeExtractions,
  recordExtraction,
  type FormValues,
} from "./extraction";
import type { ExtractedField, ExtractionResult } from "./types";

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
      tax: absent(),
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
    const r = result({ tax: unclear("TAX ?") });
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
      "The receipt's subtotal and tax don't add up to the total. Check the amount.",
    ],
  };
  expect(fieldReason(r, "total")).toBe(r.warnings[1]);
  expect(fieldReason(r, "subtotal")).toBe(r.warnings[0]);
  expect(fieldReason(r, "tax")).toBeUndefined();
});
