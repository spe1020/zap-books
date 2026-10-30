// The privacy guard against invented labels in a temporary folder. No real
// receipt values appear here.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import {
  addedLines,
  allowFile,
  binaryFiles,
  cents,
  corpusDirs,
  corpusVerdict,
  findLeaks,
  lineHash,
  loadSecrets,
  messageLines,
  readAllowed,
  SKIP_VARIABLE,
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
    tip: "6.00",
    date: "2032-11-23",
  }),
);
writeFileSync(
  join(corpus, "nested", "b.expected.json"),
  JSON.stringify({ vendor: "Rook & Raven Market", total: "37.83" }),
);
// The fictional sample shares a total with the corpus, and prints a line item
// equal to a corpus tip; both stay allowed.
writeFileSync(
  join(samples, "demo.html"),
  '<td>Index cards</td><td class="number">$6.00</td>',
);
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
const leaks = (text, file = "src/x.ts", ordinary) =>
  findLeaks([{ file, line: 3, text, ordinary }], secrets, "docs/samples").map(
    (l) => l.kind,
  );

afterAll(() => rmSync(root, { recursive: true, force: true }));

describe("privacy guard", () => {
  it("reads every label in every corpus folder, nested ones too", () => {
    expect(labelCount).toBe(3);
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

  it("doesn't check dates", () => {
    for (const text of ["2031-03-07", "3/7/31", "Mar 7, 2031", "2032-11-23"])
      expect(leaks(text), text).toEqual([]);
  });

  it("allows values the fictional samples use, and doesn't check docs/samples", () => {
    expect(leaks("TOTAL 61.40")).toEqual([]);
    expect(leaks('has_label("Cards $6.00", &["cards"])')).toEqual([]);
    expect(leaks("Quillfeather", "docs/samples/demo.html")).toEqual([]);
  });

  it("reports file, line and kind, never the value", () => {
    const found = findLeaks(
      [{ file: "a.md", line: 9, text: "Quillfeather 1284.19 on 2031-03-07" }],
      secrets,
    );
    expect(found).toEqual([
      { file: "a.md", line: 9, kind: "vendor name" },
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

  it("exempts a trailer only in the exact co-author format (PRIV-5)", () => {
    const kept = (line) => messageLines(`Fix\n\n${line}`).map((l) => l.text);
    expect(kept("Co-Authored-By: Invented Name <a@example.com>")).toEqual([
      "Fix",
      "",
    ]);
    for (const line of [
      "Co-Authored-By: Quillfeather",
      "Co-Authored-By: A <a@example.com> Quillfeather",
      "Co-Authored-By: A <not an address>",
      "co-authored-by:Quillfeather <a@example.com>",
    ])
      expect(kept(line), line).toEqual(["Fix", "", line]);
  });

  it("parses amounts", () => {
    expect(cents("$1,284.19")).toBe(128419);
    expect(cents("0.5")).toBe(50);
    expect(cents("lots")).toBeNull();
  });

  it("takes corpus folders from ZAPBOOKS_CORPUS_DIRS", () => {
    expect(corpusDirs({ ZAPBOOKS_CORPUS_DIRS: "/a:/b" })).toEqual(["/a", "/b"]);
    expect(corpusDirs({})).toHaveLength(2);
  });
});

describe("matching gaps (PRIV-5)", () => {
  it("finds amounts in minor units and with any thousands separator", () => {
    for (const [text, kind] of [
      ["amountMinor: 128419", "amount in minor units"],
      ["minor(7419)", "amount in minor units"],
      ["-7419", "amount in minor units"],
      ["1.284,19", "amount"],
      ["1 284.19", "amount"],
      ["1\u00a0284.19", "amount"],
      ["1'284.19", "amount"],
      ["74,19 EUR", "amount"],
      // Fullwidth digits.
      ["\uff17\uff14.\uff11\uff19", "amount"],
    ])
      expect(leaks(text), text).toEqual([kind]);
    for (const text of ["id 1284190", "7419px", "v7419.2", "12841", "74.1"])
      expect(leaks(text), text).toEqual([]);
  });

  it("finds a distinctive word of a vendor's name on its own", () => {
    expect(leaks("the brindle crossing")).toEqual(["part of a vendor name"]);
    expect(leaks("FERRY times")).toEqual(["part of a vendor name"]);
    // Short and generic words don't count; nor do the samples' words.
    expect(leaks("the bay")).toEqual([]);
    expect(leaks("market day")).toEqual([]);
    expect(leaks("pine trees")).toEqual([]);
  });

  it("leaves alone what the tree already held before the change", () => {
    const ordinary = new Set(["brindle", "7419"]);
    expect(leaks("the brindle crossing", "src/x.ts", ordinary)).toEqual([]);
    expect(leaks("minor(7419)", "src/x.ts", ordinary)).toEqual([]);
    // A whole name and a printed amount are matched regardless.
    expect(leaks("Brindle Bay Ferry", "src/x.ts", ordinary)).toEqual([
      "vendor name",
    ]);
    expect(leaks("74.19", "src/x.ts", ordinary)).toEqual(["amount"]);
  });

  it("sees through common encodings, accents and & for and", () => {
    for (const text of [
      "Brindle&nbsp;Bay&nbsp;Ferry",
      "Brindle%20Bay%20Ferry",
      "Brindle\\u0020Bay\\u0020Ferry",
      "Brin\u200bdle Bay Ferry",
      "Bríndle Bay Fërry",
      "Brindle  Bay\tFerry",
      "rook and raven market",
      "Rook &amp; Raven Market",
      Buffer.from("Brindle Bay Ferry, total 61.40").toString("base64"),
    ])
      expect(leaks(text), text).toContain("vendor name");
    expect(leaks(Buffer.from("TOTAL 1284.19").toString("base64"))).toContain(
      "amount",
    );
  });

  it("lists binary files, which it can't check", () => {
    const diff = [
      "diff --git a/r.png b/r.png",
      "new file mode 100644",
      "Binary files /dev/null and b/r.png differ",
      "diff --git a/old.pdf b/old.pdf",
      "Binary files a/old.pdf and /dev/null differ",
    ].join("\n");
    expect(binaryFiles(diff)).toEqual(["r.png"]);
  });
});

describe("a missing corpus blocks (PRIV-3)", () => {
  const dirs = [corpus, join(root, "missing")];
  it("blocks when a configured folder is missing, and says why", () => {
    const loaded = loadSecrets(dirs, samples);
    expect(loaded.problems).toEqual([`${join(root, "missing")} doesn't exist`]);
    const v = corpusVerdict(loaded, dirs, {});
    expect(v.block).toBe(true);
    expect(v.message).toContain("doesn't exist");
    expect(v.message).toContain("CI has no corpus");
    expect(v.message).toContain(SKIP_VARIABLE);
  });

  it("blocks when no label loads, or one can't be read, without naming it", () => {
    const empty = join(root, "empty");
    mkdirSync(empty, { recursive: true });
    expect(
      corpusVerdict(loadSecrets([empty], samples), [empty], {}).block,
    ).toBe(true);
    const broken = join(root, "broken");
    mkdirSync(broken, { recursive: true });
    writeFileSync(join(broken, "IMG_0001.expected.json"), "{ not json");
    const loaded = loadSecrets([corpus, broken], samples);
    expect(loaded.problems).toEqual([
      `1 label file(s) in ${broken} can't be read`,
    ]);
    expect(corpusVerdict(loaded, [corpus, broken], {}).block).toBe(true);
  });

  it("lets a commit through only with a reason", () => {
    const loaded = loadSecrets(dirs, samples);
    for (const reason of ["", "1", "true", "  yes   "])
      expect(
        corpusVerdict(loaded, dirs, { [SKIP_VARIABLE]: reason }).block,
        reason,
      ).toBe(true);
    const v = corpusVerdict(loaded, dirs, {
      [SKIP_VARIABLE]: "docs-only change on another laptop",
    });
    expect(v).toEqual({
      block: false,
      message: `privacy guard: NOT CHECKED (${SKIP_VARIABLE}: docs-only change on another laptop).`,
    });
    expect(corpusVerdict(loadSecrets([corpus], samples), [corpus], {})).toEqual(
      {
        block: false,
      },
    );
  });
});

describe("an allow-list outside the repository (PRIV-6)", () => {
  it("skips an allowed line, and only that line", () => {
    const allowed = new Set([lineHash("Ferry times, as printed  ")]);
    const found = (text) =>
      findLeaks(
        [{ file: "a.md", line: 1, text }],
        secrets,
        "docs/samples",
        allowed,
      );
    expect(found("Ferry times, as printed")).toEqual([]);
    expect(found("Ferry times, as printed again")).toHaveLength(1);
  });

  it("must be outside the repository, and holds hashes only", () => {
    const repo = join(root, "repo");
    expect(() =>
      allowFile(repo, { ZAPBOOKS_PRIVACY_ALLOW: join(repo, "allow") }),
    ).toThrow("inside the repository");
    const outside = join(root, "allow");
    expect(allowFile(repo, { ZAPBOOKS_PRIVACY_ALLOW: outside })).toBe(outside);
    writeFileSync(
      outside,
      `# a comment\n${lineHash("x")}  # docs/a.md:3\nnot a hash\n`,
    );
    expect(readAllowed(outside)).toEqual(new Set([lineHash("x")]));
    expect(readAllowed(join(root, "nothing"))).toEqual(new Set());
  });
});
