import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { seedWorkspace } from "../domain/seed";
import { blankDraft } from "../domain/ledger";
import { commitImport, parseCSV, previewImport } from "../domain/import";
import { saveRule, updateSettings } from "../domain/settings";
import type {
  BusinessData,
  ExpenseDraft,
  ExtractionResult,
  SourceDocument,
} from "../domain/types";
import type { LedgerCommand } from "./ledger-client";
import { applyLocal } from "./local-ledger";

// Fixed ids and time, so two runs can be compared record for record.
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

const run = (b: BusinessData, commands: LedgerCommand[]) =>
  commands.reduce(applyLocal, b);

function save(b: BusinessData, withReading: boolean): LedgerCommand {
  const doc = receipt(b);
  return {
    type: "SaveDraft",
    draft: draft(b),
    documents: [doc],
    extraction: withReading
      ? { ...reading(doc.id), accepted: ["vendor", "total"] }
      : undefined,
  };
}

const ruleFor = (d: ExpenseDraft, id?: string): LedgerCommand => ({
  type: "SaveRule",
  rule: {
    id,
    vendorIncludes: d.vendor.trim(),
    categoryId: d.categoryId,
    purpose: d.purpose,
    enabled: true,
  },
});

describe("LocalLedgerClient applies each command with the domain functions", () => {
  test("saving attaches receipts and keeps a reading pending", () => {
    const b = seedWorkspace().businesses[0];
    const after = run(b, [save(b, true)]);
    expect(after.documents.at(-1)?.id).toBe("doc-harbor");
    const record = after.extractions.find((x) => x.draftId === "draft-harbor");
    expect(record?.outcomes).toBeNull();
  });

  test("approving posts the draft and judges its reading", () => {
    const b = seedWorkspace().businesses[0];
    const after = run(b, [
      save(b, true),
      { type: "Approve", expenseId: "draft-harbor" },
    ]);
    expect(after.drafts.find((d) => d.id === "draft-harbor")?.status).toBe(
      "posted",
    );
    const record = after.extractions.find((x) => x.draftId === "draft-harbor");
    expect(record?.outcomes).toEqual({
      vendor: "edited",
      date: "ignored",
      total: "accepted",
      currency: "ignored",
      category: "ignored",
    });
  });

  test("a correction attaches receipts and judges the replacement's reading", () => {
    const b = seedWorkspace().businesses[0];
    const original = b.drafts.find((d) => d.status === "posted")!;
    const doc = receipt(b);
    const after = run(b, [
      {
        type: "Correct",
        originalId: original.id,
        replacement: {
          ...original,
          id: "draft-correction",
          amountMinor: 3317,
          status: "draft",
          documentIds: [...original.documentIds, doc.id],
        },
        reason: "The receipt shows a different total.",
        documents: [doc],
        extraction: { ...reading(doc.id), accepted: ["total"] },
      },
    ]);
    expect(after.drafts.find((d) => d.id === original.id)?.status).toBe(
      "corrected",
    );
    const record = after.extractions.find(
      (x) => x.draftId === "draft-correction",
    );
    expect(record?.outcomes?.total).toBe("accepted");
  });

  test("a rule from the Review screen goes through saveRule's checks", () => {
    const b = seedWorkspace().businesses[0];
    const d = draft(b);
    const after = run(b, [ruleFor(d), save(b, false)]);
    const rule = after.rules.find(
      (r) => r.vendorIncludes === "Harbor Ink Supply",
    );
    // saveRule trims the purpose and logs its own message.
    expect(rule).toMatchObject({
      purpose: "Ink for client proofs",
      enabled: true,
    });
    expect(
      after.audits.some(
        (a) => a.detail === "Classification rule saved: Harbor Ink Supply",
      ),
    ).toBe(true);
    // Saving it again replaces it.
    const again = run(after, [ruleFor(d, rule!.id)]);
    expect(
      again.rules.filter((r) => r.vendorIncludes === "Harbor Ink Supply"),
    ).toHaveLength(1);
    // A rule saveRule refuses fails the whole action.
    expect(() =>
      run(b, [ruleFor({ ...d, purpose: " " }), save(b, false)]),
    ).toThrow("Vendor and purpose are required.");
    expect(() => run(after, [ruleFor(d), save(b, false)])).toThrow(
      "A rule for this vendor already exists.",
    );
  });

  test("a failed step throws, so the action saves nothing", () => {
    const b = seedWorkspace().businesses[0];
    const invalid = { ...draft(b), categoryId: "" };
    expect(() =>
      run(b, [
        { type: "SaveDraft", draft: invalid, documents: [] },
        { type: "Approve", expenseId: invalid.id },
      ]),
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
