import { describe, expect, test } from "vitest";
import { seedWorkspace } from "./seed";
import { approve, correct, id, saveDraft } from "./ledger";
import { commitImport, guessMapping, parseCSV, previewImport } from "./import";
import type { BusinessData } from "./types";
import type { ImportOptions } from "./import";

const kinds = ["payment", "expense-history"] as const;
function preview(
  b: BusinessData,
  account: string,
  kind: ImportOptions["kind"],
  reference = "AUDIT-ONE",
) {
  const rows = parseCSV(
    `date,description,amount,currency,reference\n2026-10-24,Audit Vendor,-42.50,USD,${reference}`,
  );
  return previewImport(b, rows, guessMapping(rows[0]), account, "negative", {
    kind,
  });
}

describe("ledger trust boundaries", () => {
  test.each(kinds)(
    "a forged correction cannot post the same %s evidence twice",
    (kind) => {
      const b = seedWorkspace().businesses[0];
      const imported = commitImport(
        b,
        preview(b, b.accounts[0].id, kind),
        b.accounts[0].id,
      );
      const draft = {
        ...imported.drafts.at(-1)!,
        categoryId: b.categories[0].id,
        purpose: "Client work",
      };
      const posted = approve(saveDraft(imported, draft), draft.id);
      const original = posted.drafts.find((d) => d.id === draft.id)!;
      const replacement = {
        ...original,
        id: id(),
        status: "draft" as const,
        correctionOf: original.id,
      };
      const before = structuredClone(posted);
      expect(() =>
        approve(saveDraft(posted, replacement), replacement.id),
      ).toThrow(
        kind === "payment"
          ? "This payment is already linked"
          : "This imported source already has a posted expense",
      );
      expect(posted).toEqual(before);
      // The same replacement works through the command that reverses the original.
      const corrected = correct(
        posted,
        original.id,
        replacement,
        "Clarify purchase",
      );
      expect(corrected.drafts.find((d) => d.id === original.id)?.status).toBe(
        "corrected",
      );
      expect(
        corrected.drafts.find((d) => d.id === replacement.id)?.status,
      ).toBe("posted");
      expect(
        corrected.journals.filter((j) => j.reversesId === original.journalId),
      ).toHaveLength(1);
    },
  );

  for (const kind of kinds) {
    test.each(["AUDIT-ONE", ""])(
      `${kind} previews bind account and business (reference=%s)`,
      (reference) => {
        const [b, other] = seedWorkspace().businesses;
        const [from, to] = b.accounts;
        const rows = preview(b, from.id, kind, reference);
        const before = structuredClone(b);
        expect(() => commitImport(b, rows, to.id)).toThrow(
          "Import rows were previewed for a different business or account",
        );
        expect(() => commitImport(other, rows, other.accounts[0].id)).toThrow(
          "Import rows were previewed for a different business or account",
        );
        expect(b).toEqual(before);

        const once = commitImport(b, rows, from.id);
        // Scope must be checked even if this fingerprint is already stored.
        expect(() => commitImport(once, rows, to.id)).toThrow(
          "Import rows were previewed for a different business or account",
        );
        expect(commitImport(once, rows, from.id)).toEqual(once);

        const destination = preview(b, to.id, kind, reference);
        // A later invalid row must not partially import earlier valid rows.
        expect(() => commitImport(b, [...destination, ...rows], to.id)).toThrow(
          "Import rows were previewed for a different business or account",
        );
        expect(b).toEqual(before);
        const imported = commitImport(b, destination, to.id);
        expect(imported.drafts.length).toBe(b.drafts.length + 1);
        const repeated = preview(imported, to.id, kind, reference);
        expect(repeated[0].duplicate).toBe(true);
        expect(commitImport(imported, repeated, to.id)).toEqual(imported);
      },
    );
  }

  test.each(kinds)(
    "a %s row with a forged fingerprint is rejected even in scope",
    (kind) => {
      const b = seedWorkspace().businesses[0];
      const rows = preview(b, b.accounts[0].id, kind, "");
      const forged = structuredClone(rows);
      forged[0].fingerprint = '["northstar","elsewhere"]';
      if (forged[0].importSource)
        forged[0].importSource.fingerprint = forged[0].fingerprint;
      expect(() => commitImport(b, forged, b.accounts[0].id)).toThrow(
        "Import preview does not match",
      );
    },
  );

  test("repeated history rows still validate source ownership", () => {
    const b = seedWorkspace().businesses[0];
    const rows = preview(b, b.accounts[0].id, "expense-history");
    const once = commitImport(b, rows, b.accounts[0].id);
    const foreign = structuredClone(rows);
    foreign[0].importSource!.businessId = "fieldwork";
    expect(() => commitImport(once, foreign, b.accounts[0].id)).toThrow(
      "Invalid import source",
    );
  });
});
