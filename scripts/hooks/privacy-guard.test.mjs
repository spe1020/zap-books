// The privacy guard against invented labels in a temporary folder. No real
// receipt values appear here.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
  addedLines,
  cents,
  corpusDirs,
  dateForms,
  findLeaks,
  loadSecrets,
  messageLines,
} from "./privacy-guard.mjs";

const root = mkdtempSync(join(tmpdir(), "privacy-guard-"));
const corpus = join(root, "corpus");
const samples = join(root, "samples");
mkdirSync(join(corpus, "nested"), { recursive: true });
mkdirSync(samples);
writeFileSync(
  join(corpus, "a.expected.json"),
  JSON.stringify({
    vendor: ["Quillfeather Stationers", "Quillfeather"],
    date: "2031-03-07",
    total: "$1,284.19",
    subtotal: "1210.00",
    tax: "74.19",
    fees: null,
    reference: ["QF-55821", "7"],
  }),
);
writeFileSync(
  join(corpus, "nested", "expected.json"),
  JSON.stringify({
    vendor: "Brindle Bay Ferry",
    total: "61.40",
    date: "2032-11-23",
  }),
);
// The fictional sample shares a total and a date with the corpus; both stay
// allowed.
writeFileSync(
  join(samples, "demo.expected.json"),
  JSON.stringify({
    vendor: "Paper & Pine",
    total: "61.40",
    date: "2032-11-23",
  }),
);

const { secrets, labelCount } = loadSecrets(
  [corpus, join(root, "missing")],
  samples,
);
const leaks = (text, file = "src/x.ts") =>
  findLeaks([{ file, line: 3, text }], secrets, "docs/samples").map(
    (l) => l.kind,
  );

afterAll(() => rmSync(root, { recursive: true, force: true }));

describe("privacy guard", () => {
  it("reads every label in every corpus folder, nested ones too", () => {
    expect(labelCount).toBe(2);
  });

  it("finds vendors as whole words, ignoring case", () => {
    expect(leaks('vendor: "QUILLFEATHER"')).toEqual(["vendor name"]);
    expect(leaks("Brindle Bay Ferry, pier 2")).toEqual(["vendor name"]);
    expect(leaks("quillfeathers")).toEqual([]);
  });

  it("finds amounts however they are printed", () => {
    for (const text of ["TOTAL $1,284.19", "1284.19", "TAX 74.19-", "(74.19)"])
      expect(leaks(text), text).toEqual(["amount"]);
    for (const text of ["1284.195", "11284.19", "version 74.19.2", "74.1"])
      expect(leaks(text), text).toEqual([]);
  });

  it("finds receipt numbers, but skips very short ones", () => {
    expect(leaks("Order qf-55821")).toEqual(["receipt number"]);
    expect(leaks("Check #7")).toEqual([]);
  });

  it("finds dates in the usual printed forms", () => {
    for (const text of [
      "2031-03-07",
      "03/07/31",
      "3/7/2031",
      "07/03/2031",
      "Mar 7, 2031",
      "7 March 2031",
      "07-Mar-2031",
      "2031-03-07T10:00",
    ])
      expect(leaks(text), text).toEqual(["date"]);
    expect(leaks("2031-03-071")).toEqual([]);
  });

  it("allows values the fictional samples use, and doesn't check docs/samples", () => {
    expect(leaks("TOTAL 61.40")).toEqual([]);
    expect(leaks("2032-11-23")).toEqual([]);
    expect(leaks("Quillfeather", "docs/samples/demo.html")).toEqual([]);
  });

  it("reports file, line and kind, never the value", () => {
    const found = findLeaks(
      [{ file: "a.md", line: 9, text: "Quillfeather 1284.19 on 2031-03-07" }],
      secrets,
    );
    expect(found).toEqual([
      { file: "a.md", line: 9, kind: "vendor name" },
      { file: "a.md", line: 9, kind: "date" },
      { file: "a.md", line: 9, kind: "amount" },
    ]);
    expect(JSON.stringify(found)).not.toMatch(/quillfeather|1284|2031/i);
  });

  it("reads added lines with their line numbers from a diff", () => {
    const diff = [
      "diff --git a/x.md b/x.md",
      "--- a/x.md",
      "+++ b/x.md",
      "@@ -1,0 +4,2 @@",
      "+first",
      "+second",
      "+++ /dev/null",
      "@@ -1 +0,0 @@",
      "-gone",
    ].join("\n");
    expect(addedLines(diff)).toEqual([
      { file: "x.md", line: 4, text: "first" },
      { file: "x.md", line: 5, text: "second" },
    ]);
  });

  it("checks a commit message without git's comments", () => {
    const lines = messageLines(
      "Fix totals\n\n# Quillfeather in a comment\nBody\n# ------------------------ >8 ------------------------\nQuillfeather",
    );
    expect(lines.map((l) => l.text)).toEqual(["Fix totals", "", "Body"]);
    const trailer = messageLines(
      "Fix\n\nCo-Authored-By: Quillfeather <a@example.com>\nQuillfeather",
    );
    expect(trailer.map((l) => l.text)).toEqual(["Fix", "", "Quillfeather"]);
  });

  it("parses amounts and date forms", () => {
    expect(cents("$1,284.19")).toBe(128419);
    expect(cents("0.5")).toBe(50);
    expect(cents("lots")).toBeNull();
    expect(dateForms("2031-03-07")).toContain("3/7/31");
    expect(dateForms("not a date")).toEqual([]);
  });

  it("takes corpus folders from ZAPBOOKS_CORPUS_DIRS", () => {
    expect(corpusDirs({ ZAPBOOKS_CORPUS_DIRS: "/a:/b" })).toEqual(["/a", "/b"]);
    expect(corpusDirs({})).toHaveLength(2);
  });
});
