import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { seedWorkspace } from "../domain/seed";
import {
  approve,
  blankDraft,
  correct,
  event,
  id,
  saveDraft,
} from "../domain/ledger";
import { commitImport, parseCSV, previewImport } from "../domain/import";
import { saveRule, updateSettings } from "../domain/settings";
import { finalizeExtractions, recordExtraction } from "../domain/extraction";
import type {
  BusinessData,
  ExpenseDraft,
  ExtractionResult,
  SourceDocument,
} from "../domain/types";
import type { LedgerCommand, ReceiptReading } from "./ledger-client";
import { applyLocal } from "./local-ledger";

// Fixed ids and time, so the old path and the new one can be compared
// record for record.
let counter = 0;
beforeEach(() => {
  counter = 0;
  vi.spyOn(crypto, "randomUUID").mockImplementation(
    () =>
      `00000000-0000-4000-8000-${String(++counter).padStart(12, "0")}` as `${string}-${string}-${string}-${string}-${string}`,
  );
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-10-15T12:00:00.000Z"));
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/**
 * The Review screen's save, exactly as it was before `LedgerClient`
 * (`src/components/Review.tsx` at the parent of this change).
 */
function reviewBefore(
  b: BusinessData,
  current: BusinessData,
  o: {
    docs: SourceDocument[];
    next: ExpenseDraft;
    saveRule: boolean;
    extraction?: { documentId: string; result: ExtractionResult };
    accepted: string[];
    correcting: boolean;
    initialId: string;
    reason: string;
    post: boolean;
  },
): BusinessData {
  const { docs, next, saveRule, extraction, accepted, correcting, post } = o;
  let updated = {
    ...current,
    documents: [...current.documents, ...docs],
  };
  if (saveRule) {
    const existing = updated.rules.find(
      (r) =>
        r.vendorIncludes.toLowerCase() === next.vendor.trim().toLowerCase(),
    );
    const rule = {
      id: existing?.id ?? id(),
      businessId: b.profile.id,
      vendorIncludes: next.vendor.trim(),
      categoryId: next.categoryId,
      purpose: next.purpose,
      enabled: true,
    };
    updated = {
      ...updated,
      rules: [...updated.rules.filter((r) => r.id !== rule.id), rule],
    };
    event(
      updated,
      "rule.saved",
      rule.id,
      `Explicitly saved classification rule for ${rule.vendorIncludes}`,
    );
  }
  const withReading = (state: BusinessData) =>
    extraction
      ? recordExtraction(state, {
          draftId: next.id,
          documentId: extraction.documentId,
          result: extraction.result,
          accepted: [...accepted] as ReceiptReading["accepted"],
        })
      : state;
  if (correcting)
    return finalizeExtractions(
      withReading(correct(updated, o.initialId, next, o.reason)),
      next.id,
    );
  updated = withReading(saveDraft(updated, next));
  return post
    ? finalizeExtractions(approve(updated, next.id), next.id)
    : updated;
}

/** The Review screen's commands now, as it builds them. */
function reviewNow(o: Parameters<typeof reviewBefore>[2]): LedgerCommand[] {
  const reading = o.extraction
    ? {
        documentId: o.extraction.documentId,
        result: o.extraction.result,
        accepted: [...o.accepted] as ReceiptReading["accepted"],
      }
    : undefined;
  return o.correcting
    ? [
        {
          type: "Correct",
          originalId: o.initialId,
          replacement: o.next,
          reason: o.reason,
          documents: o.docs,
          extraction: reading,
          rememberRule: o.saveRule,
        },
      ]
    : [
        {
          type: "SaveDraft",
          draft: o.next,
          documents: o.docs,
          extraction: reading,
          rememberRule: o.saveRule,
        },
        ...(o.post ? [{ type: "Approve" as const, expenseId: o.next.id }] : []),
      ];
}

const found = <T>(value: T, evidence: string) => ({
  status: "found" as const,
  value,
  evidence,
});
const none = () => ({ status: "absent" as const, value: null, lines: [] });

function reading(documentId: string): {
  documentId: string;
  result: ExtractionResult;
} {
  return {
    documentId,
    result: {
      fields: {
        vendor: found("Harbor Ink Supply", "HARBOR INK SUPPLY"),
        date: found("2031-04-08", "04/08/2031"),
        total: found(3317, "TOTAL $33.17"),
        currency: found("USD", "$"),
        reference: { status: "absent", value: null, evidence: "" },
        subtotal: { status: "absent", value: null, evidence: "" },
        tax: none(),
        fees: none(),
        discounts: none(),
        tip: { status: "absent", value: null, evidence: "" },
        category: found("northstar-category-1", "Ink"),
      },
      warnings: [],
      model: "test-model",
      promptVersion: "test-1",
      pagesRead: 1,
      usedTextLayer: false,
      durationMs: 10,
      modelSupportsThinking: false,
    },
  };
}

function receipt(b: BusinessData): SourceDocument {
  return {
    id: "doc-harbor",
    businessId: b.profile.id,
    name: "harbor-ink.pdf",
    mime: "application/pdf",
    size: 9,
    blob: new Blob(["%PDF-1.7\n"], { type: "application/pdf" }),
    kind: "receipt",
    createdAt: "2026-10-15T12:00:00.000Z",
    demo: false,
  };
}

function draft(b: BusinessData): ExpenseDraft {
  return {
    ...blankDraft(b),
    id: "draft-harbor",
    vendor: " Harbor Ink Supply ",
    date: "2031-04-08",
    description: "Printer ink",
    amountMinor: 3317,
    categoryId: "northstar-category-1",
    purpose: "Ink for client proofs ",
    documentIds: ["doc-harbor"],
  };
}

/** Runs `run` twice from the same ids and time: the results must match. */
function same(run: (b: BusinessData) => BusinessData) {
  const b = seedWorkspace().businesses[0];
  const reset = () => {
    counter = 0;
    vi.setSystemTime(new Date("2026-10-15T12:00:00.000Z"));
  };
  reset();
  const a = run(structuredClone(b));
  reset();
  return { a, b, reset };
}

describe("LocalLedgerClient applies each action as the screens did", () => {
  const cases = [
    { name: "save a draft", post: false, correcting: false, rule: false },
    {
      name: "save with a reading and a rule",
      post: false,
      correcting: false,
      rule: true,
    },
    { name: "save and approve", post: true, correcting: false, rule: false },
    {
      name: "save and approve with a rule",
      post: true,
      correcting: false,
      rule: true,
    },
  ];
  for (const c of cases)
    for (const withReading of [false, true])
      test(`${c.name}${withReading ? ", read" : ""}`, () => {
        const {
          a: before,
          b,
          reset,
        } = same((x) => {
          const doc = receipt(x);
          return reviewBefore(x, x, {
            docs: [doc],
            next: draft(x),
            saveRule: c.rule,
            extraction: withReading ? reading(doc.id) : undefined,
            accepted: ["vendor", "total"],
            correcting: false,
            initialId: "draft-harbor",
            reason: "",
            post: c.post,
          });
        });
        reset();
        const x = structuredClone(b);
        const doc = receipt(x);
        const after = reviewNow({
          docs: [doc],
          next: draft(x),
          saveRule: c.rule,
          extraction: withReading ? reading(doc.id) : undefined,
          accepted: ["vendor", "total"],
          correcting: false,
          initialId: "draft-harbor",
          reason: "",
          post: c.post,
        }).reduce(applyLocal, x);
        expect(after).toEqual(before);
        if (c.post)
          expect(
            after.drafts.find((d) => d.id === "draft-harbor")?.status,
          ).toBe("posted");
      });

  test("correct a posted expense, with a reading and a rule", () => {
    const options = (x: BusinessData) => {
      const doc = receipt(x);
      const original = x.drafts.find((d) => d.status === "posted")!;
      return {
        docs: [doc],
        next: {
          ...original,
          id: "draft-correction",
          amountMinor: original.amountMinor + 100,
          status: "draft" as const,
          documentIds: [...original.documentIds, doc.id],
        },
        saveRule: true,
        extraction: reading(doc.id),
        accepted: ["vendor"],
        correcting: true,
        initialId: original.id,
        reason: "The receipt shows a higher total.",
        post: false,
      };
    };
    const { a: before, b, reset } = same((x) => reviewBefore(x, x, options(x)));
    reset();
    const x = structuredClone(b);
    const after = reviewNow(options(x)).reduce(applyLocal, x);
    expect(after).toEqual(before);
    const original = options(b).initialId;
    expect(after.drafts.find((d) => d.id === original)?.status).toBe(
      "corrected",
    );
  });

  test("a failed step throws, so the action saves nothing", () => {
    const b = seedWorkspace().businesses[0];
    const invalid = { ...draft(b), categoryId: "" };
    expect(() =>
      [
        { type: "SaveDraft", draft: invalid, documents: [] },
        { type: "Approve", expenseId: invalid.id },
      ].reduce(applyLocal as never, b),
    ).toThrow();
  });

  test("import, settings and rules call their domain functions", () => {
    const b = seedWorkspace().businesses[0];
    const account = b.accounts.find((a) => a.kind === "bank")!;
    const rows = previewImport(
      b,
      parseCSV("Date,Description,Amount\n2031-04-07,Harbor Ink Supply,-33.17"),
      {
        date: "Date",
        description: "Description",
        amount: "Amount",
        currency: "",
        reference: "",
      },
      account.id,
      "negative",
    );
    const pairs: [LedgerCommand, (x: BusinessData) => BusinessData][] = [
      [
        { type: "CommitImport", accountId: account.id, rows },
        (x) => commitImport(x, rows, account.id),
      ],
      [
        {
          type: "UpdateSettings",
          change: { op: "profile", name: "Northstar", industry: "Design" },
        },
        (x) =>
          updateSettings(x, {
            op: "profile",
            name: "Northstar",
            industry: "Design",
          }),
      ],
      [
        {
          type: "SaveRule",
          rule: {
            vendorIncludes: "Harbor Ink",
            categoryId: "northstar-category-1",
            purpose: "Ink",
            enabled: true,
          },
        },
        (x) =>
          saveRule(x, {
            vendorIncludes: "Harbor Ink",
            categoryId: "northstar-category-1",
            purpose: "Ink",
            enabled: true,
          }),
      ],
    ];
    expect(rows.filter((r) => !r.error)).toHaveLength(1);
    for (const [command, before] of pairs) {
      counter = 0;
      const want = before(structuredClone(b));
      counter = 0;
      expect(want).not.toEqual(b);
      expect(applyLocal(structuredClone(b), command)).toEqual(want);
    }
  });

  test("membership needs the service", () => {
    const b = seedWorkspace().businesses[0];
    for (const command of [
      { type: "ChangeMembership", change: {} },
      { type: "Veto", pending: "0".repeat(64) },
    ] as LedgerCommand[])
      expect(() => applyLocal(b, command)).toThrow(
        /needs a business on the service/,
      );
  });
});
