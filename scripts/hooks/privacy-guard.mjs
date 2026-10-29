// Blocks a commit that contains values from real receipt labels: vendor
// names, amounts and receipt numbers read from every *.expected.json in the
// corpus folders. Dates aren't checked: fictional fixtures use dates close to
// real receipts' dates, so a date check blocks ordinary edits. It reports the
// file and line, never the value, and it never writes the values anywhere.
//
// Corpus folders: ZAPBOOKS_CORPUS_DIRS (separated by ":"), or by default
// ~/Desktop/reciepts and ~/Desktop/receipts-holdout. The committed fictional
// samples in docs/samples/ are exempt: their values are allowed (label fields
// and every amount printed on a sample page), and changes under docs/samples/
// aren't checked.
//
// Run as a pre-commit hook it checks the staged diff; run as a commit-msg
// hook (with the message file as its argument) it checks the message.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";

const AMOUNT_FIELDS = ["total", "subtotal", "tax", "fees", "discounts", "tip"];
const SAMPLES_DIR = "docs/samples";
/** Shorter receipt numbers (`3`, `61`) would match almost anything. */
const MIN_REFERENCE_LENGTH = 4;
const MIN_VENDOR_LENGTH = 3;
const AMOUNT = /(?<![\d.,])\d[\d,]*\.\d{2}(?!\d|\.\d)/g;

/** Every *.expected.json (or expected.json) under `dir`, recursively. */
export function labelFiles(dir) {
  if (!existsSync(dir) || !statSync(dir).isDirectory()) return [];
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...labelFiles(path));
    else if (
      entry.name.endsWith(".expected.json") ||
      entry.name === "expected.json"
    )
      out.push(path);
  }
  return out;
}

/** The text of every sample page (*.html) in `dir`. */
function samplePages(dir) {
  if (!existsSync(dir) || !statSync(dir).isDirectory()) return [];
  return readdirSync(dir)
    .filter((name) => name.endsWith(".html"))
    .map((name) => readFileSync(join(dir, name), "utf8"));
}

/** An amount as whole cents, or null: `"$1,042.50"` → 104250. */
export function cents(text) {
  if (typeof text !== "string" && typeof text !== "number") return null;
  const s = String(text)
    .replace(/[$€£\s]/g, "")
    .replace(/^-|-$/g, "");
  if (!/^\d{1,3}(,\d{3})*(\.\d{1,2})?$|^\d+(\.\d{1,2})?$/.test(s)) return null;
  const [whole, frac = ""] = s.replace(/,/g, "").split(".");
  return Number(whole) * 100 + Number(frac.padEnd(2, "0"));
}

const list = (v) => (Array.isArray(v) ? v : v == null ? [] : [v]);
const fold = (s) => s.toLowerCase().replace(/[’‘]/g, "'");
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * The values to block, from the labels in `dirs`, less any value the
 * fictional samples in `samplesDir` also use. Each is `{ kind, test }`;
 * `test(line)` says whether a line contains it.
 */
export function loadSecrets(dirs, samplesDir) {
  const read = (files) =>
    files.flatMap((f) => {
      try {
        return [JSON.parse(readFileSync(f, "utf8"))];
      } catch {
        return [];
      }
    });
  const labels = read(dirs.flatMap(labelFiles));
  const samples = read(labelFiles(samplesDir));

  const collect = (labelsList) => {
    const vendors = new Set();
    const amounts = new Set();
    const references = new Set();
    for (const label of labelsList) {
      for (const v of list(label.vendor))
        if (typeof v === "string" && v.trim().length >= MIN_VENDOR_LENGTH)
          vendors.add(fold(v.trim()));
      for (const field of AMOUNT_FIELDS)
        for (const v of list(label[field])) {
          const c = cents(v);
          if (c) amounts.add(c);
        }
      for (const v of list(label.reference))
        if (
          typeof v === "string" &&
          v.replace(/[^a-z0-9]/gi, "").length >= MIN_REFERENCE_LENGTH
        )
          references.add(fold(v.trim()));
    }
    return { vendors, amounts, references };
  };
  const real = collect(labels);
  const fictional = collect(samples);
  // A line item on a sample page is as fictional as its total, so quoting the
  // sample never trips the guard.
  for (const page of samplePages(samplesDir))
    for (const amount of page.match(AMOUNT) ?? []) {
      const c = cents(amount);
      if (c) fictional.amounts.add(c);
    }

  const secrets = [];
  const word = (kind, text) => {
    const re = new RegExp(`(?<![a-z0-9])${escape(text)}(?![a-z0-9])`, "i");
    secrets.push({ kind, test: (line) => re.test(fold(line)) });
  };
  for (const v of real.vendors)
    if (!fictional.vendors.has(v)) word("vendor name", v);
  for (const r of real.references)
    if (!fictional.references.has(r)) word("receipt number", r);
  const blocked = [...real.amounts].filter((c) => !fictional.amounts.has(c));
  if (blocked.length > 0) {
    const set = new Set(blocked);
    secrets.push({
      kind: "amount",
      test: (line) =>
        [...line.matchAll(AMOUNT)].some((m) => set.has(cents(m[0]))),
    });
  }
  return { secrets, labelCount: labels.length };
}

/** The added lines of a `git diff -U0`, as `{ file, line, text }`. */
export function addedLines(diff) {
  const out = [];
  let file = null;
  let line = 0;
  for (const raw of diff.split("\n")) {
    if (raw.startsWith("+++ ")) {
      file = raw === "+++ /dev/null" ? null : raw.replace(/^\+\+\+ b\//, "");
    } else if (raw.startsWith("@@")) {
      const m = /\+(\d+)/.exec(raw);
      line = m ? Number(m[1]) : 0;
    } else if (raw.startsWith("+") && file) {
      out.push({ file, line, text: raw.slice(1) });
      line += 1;
    }
  }
  return out;
}

/** The lines of a commit message git keeps: no comments, nothing below the scissors. */
export function messageLines(message) {
  const out = [];
  const lines = message.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const text = lines[i];
    if (/^# -+ >8 -+/.test(text)) break;
    if (text.startsWith("#")) continue;
    // Attribution trailers name people and tools, never receipt content.
    if (/^co-authored-by:/i.test(text)) continue;
    out.push({ file: "commit message", line: i + 1, text });
  }
  return out;
}

/** Where `lines` contain a secret: `{ file, line, kind }`, never the value. */
export function findLeaks(lines, secrets, samplesDir = SAMPLES_DIR) {
  const leaks = [];
  for (const { file, line, text } of lines) {
    if (file.startsWith(`${samplesDir}/`)) continue;
    for (const { kind } of secrets.filter((s) => s.test(text))) {
      if (
        !leaks.some(
          (l) => l.file === file && l.line === line && l.kind === kind,
        )
      )
        leaks.push({ file, line, kind });
    }
  }
  return leaks;
}

export function corpusDirs(env = process.env) {
  const configured = env.ZAPBOOKS_CORPUS_DIRS;
  if (configured)
    return configured
      .split(delimiter)
      .filter(Boolean)
      .map((d) => d.replace(/^~(?=$|\/)/, homedir()));
  return [
    join(homedir(), "Desktop", "reciepts"),
    join(homedir(), "Desktop", "receipts-holdout"),
  ];
}

function main() {
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], {
    encoding: "utf8",
  }).trim();
  const dirs = corpusDirs();
  const { secrets, labelCount } = loadSecrets(dirs, join(root, SAMPLES_DIR));
  if (labelCount === 0) {
    console.error(
      `privacy guard: no receipt labels found in ${dirs.join(", ")}; nothing to check.`,
    );
    return 0;
  }
  const messageFile = process.argv[2];
  const lines = messageFile
    ? messageLines(readFileSync(messageFile, "utf8"))
    : addedLines(
        execFileSync(
          "git",
          ["diff", "--cached", "--no-color", "--no-ext-diff", "-U0"],
          { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
        ),
      );
  const leaks = findLeaks(lines, secrets);
  if (leaks.length === 0) return 0;
  console.error(
    "privacy guard: this commit contains values from real receipt labels.",
  );
  for (const { file, line, kind } of leaks)
    console.error(`  ${file}:${line}: a real receipt's ${kind}`);
  console.error(
    "Use invented values instead. (The values aren't printed, so they stay out of terminal history and logs.)",
  );
  return 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exit(main());
}
