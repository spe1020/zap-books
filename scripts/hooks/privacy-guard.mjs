// Blocks a commit or push that contains values from real receipt labels:
// vendor names, amounts and receipt numbers read from every *.expected.json
// in the corpus folders. It reports the file and line, never the value, and
// never writes the values anywhere.
//
// What it matches, on each added line and on the line decoded from HTML
// entities, %-escapes, \u escapes and base64, with accents, zero-width
// characters and "and"/"&" folded away:
// - a vendor's whole name, and each word of four or more letters from a
//   vendor of several words, unless the word is generic (`GENERIC_WORDS`);
// - an amount as printed (`1,042.50`, `1.042,50`, `1 042.50`, `54,50`) or in
//   minor units (`104250`, three digits or more);
// - a receipt number of four characters or more.
// A vendor word or a minor-unit number the repository already contained
// before the change (`git grep` of the parent tree) counts as ordinary
// vocabulary there and isn't matched on its own: words like these are
// common in code, and a whole number like 1000 is everywhere. A vendor's
// whole name and every printed amount are always matched.
//
// What it doesn't check, so a person must: dates and times (fictional
// fixtures use dates close to real ones, so a check would block ordinary
// edits), addresses, phone numbers and other printed labels the labels don't
// record, a value split across lines, binary files (listed in the output),
// and PR descriptions unless run through `check`.
//
// Corpus folders: ZAPBOOKS_CORPUS_DIRS (separated by ":"), or by default
// ~/Desktop/reciepts and ~/Desktop/receipts-holdout. If a folder is missing
// or a label can't be read, it blocks: the only way past is
// ZAPBOOKS_PRIVACY_GUARD_SKIP set to the reason. CI has no corpus, so it can
// never run this check; this machine is the only one that can.
//
// The committed fictional samples in docs/samples/ are exempt: their values
// are allowed (label fields and every amount printed on a sample page), and
// changes under docs/samples/ aren't checked. A known false positive can be
// allowed with `allow <file>:<line>`, which records a hash of that line in a
// file outside the repository (ZAPBOOKS_PRIVACY_ALLOW, by default
// ~/.config/zapbooks/privacy-allow).
//
// Commands (the hooks in this folder run the first three):
//   privacy-guard.mjs pre-commit          the staged diff
//   privacy-guard.mjs commit-msg <file>   a commit message
//   privacy-guard.mjs pre-push <remote>   every outgoing commit's diff and
//                                         message, refs on stdin as git
//                                         gives them; covers rebase,
//                                         cherry-pick and merge
//   privacy-guard.mjs check <file>...     any text file, such as a PR body
//   privacy-guard.mjs allow <file>:<line> allow that line as it is now

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
} from "node:fs";
import { homedir } from "node:os";
import {
  delimiter,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
} from "node:path";
import { fileURLToPath } from "node:url";

const AMOUNT_FIELDS = ["total", "subtotal", "tax", "fees", "discounts", "tip"];
const SAMPLES_DIR = "docs/samples";
/** Shorter receipt numbers (`3`, `61`) would match almost anything. */
const MIN_REFERENCE_LENGTH = 4;
const MIN_VENDOR_LENGTH = 3;
/** The shortest word of a vendor's name matched on its own. */
const MIN_VENDOR_WORD = 4;
/** An amount in minor units is matched from three digits (`1.00`) up. */
const MIN_MINOR_DIGITS = 3;
export const SKIP_VARIABLE = "ZAPBOOKS_PRIVACY_GUARD_SKIP";
const EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const ZERO_SHA = /^0+$/;

/**
 * Words too common in business names to mean one vendor. Written without
 * reference to the corpus, so the list reveals nothing about it.
 */
const GENERIC_WORDS = new Set(
  (
    "and the with from for inc. inc llc ltd limited corp corporation company " +
    "group holdings services service supply supplies store stores shop shops " +
    "market markets mart center centre online express wholesale outlet depot " +
    "foods food kitchen grill cafe coffee restaurant bakery pizza pharmacy " +
    "station hotel travel airlines airline airways parking park garage auto " +
    "office studio design digital systems solutions software cloud media " +
    "home house street city north south east west main first national " +
    "general international global american united trading co. bank card"
  ).split(" "),
);

const DOT_AMOUNT = /(?<![\d.,])\d[\d,]*\.\d{2}(?!\d|\.\d)/g;
const COMMA_AMOUNT =
  /(?<![\d.,])(?:\d{1,3}(?:\.\d{3})+|\d+),\d{2}(?![\d,]|\.\d)/g;
const SPACED_AMOUNT = /(?<![\d.,'])\d{1,3}(?:['’ ]\d{3})+[.,]\d{2}(?!\d)/g;
const MINOR_AMOUNT = new RegExp(
  `(?<![\\w.,])[1-9]\\d{${MIN_MINOR_DIGITS - 1},}(?![\\w]|[.,]\\d)`,
  "g",
);

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

/** Every amount printed in `text`, in any of the forms above, as cents. */
export function amountsIn(text) {
  const out = [];
  for (const m of text.matchAll(DOT_AMOUNT)) out.push(cents(m[0]));
  for (const m of text.matchAll(COMMA_AMOUNT))
    out.push(cents(m[0].replace(/\./g, "").replace(",", ".")));
  for (const m of text.matchAll(SPACED_AMOUNT))
    out.push(cents(m[0].replace(/['’ ]/g, "").replace(",", ".")));
  return out.filter((c) => c !== null);
}

/** Every whole number of three digits or more in `text`. */
function minorAmountsIn(text) {
  return [...text.matchAll(MINOR_AMOUNT)].map((m) => Number(m[0]));
}

const list = (v) => (Array.isArray(v) ? v : v == null ? [] : [v]);
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Text as it is compared: lowercase, accents and zero-width characters
 * removed, compatibility forms (fullwidth digits, ligatures) unfolded,
 * curly apostrophes straightened, whitespace collapsed, and "and" written
 * as "&" with a space either side.
 */
export function fold(s) {
  return s
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .replace(/[​-‍⁠﻿­]/g, "")
    .toLowerCase()
    .replace(/[’‘]/g, "'")
    .replace(/(^|[^a-z0-9])and(?=[^a-z0-9]|$)/g, "$1&")
    .replace(/\s*&\s*/g, " & ")
    .replace(/\s+/g, " ")
    .trim();
}

const ENTITIES = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
};

/** `text` with HTML entities, %-escapes and \u / \x escapes decoded. */
export function decode(text) {
  let s = text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (all, e) => {
    if (e[0] !== "#") return ENTITIES[e.toLowerCase()] ?? all;
    const n =
      e[1] === "x" || e[1] === "X"
        ? parseInt(e.slice(2), 16)
        : Number(e.slice(1));
    return n > 0 && n <= 0x10ffff ? String.fromCodePoint(n) : all;
  });
  s = s.replace(/(?:%[0-9a-f]{2})+/gi, (run) => {
    try {
      return decodeURIComponent(run);
    } catch {
      return run;
    }
  });
  return s
    .replace(/\\u\{([0-9a-f]{1,6})\}|\\u([0-9a-f]{4})/gi, (all, a, b) => {
      const n = parseInt(a ?? b, 16);
      return n <= 0x10ffff ? String.fromCodePoint(n) : all;
    })
    .replace(/\\x([0-9a-f]{2})/gi, (_, h) =>
      String.fromCharCode(parseInt(h, 16)),
    );
}

/** The text of each base64 run in `text` that decodes to readable text. */
function base64Texts(text) {
  const out = [];
  for (const [run] of text.matchAll(/[A-Za-z0-9+/_-]{16,}={0,2}/g)) {
    const bytes = Buffer.from(
      run.replace(/-/g, "+").replace(/_/g, "/"),
      "base64",
    );
    let decoded;
    try {
      decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      continue;
    }
    const readable = decoded.replace(/[^\p{L}\p{N}\p{P}\p{Zs}$€£]/gu, "");
    if (decoded.length >= 4 && readable.length >= decoded.length * 0.9)
      out.push(decoded);
  }
  return out;
}

/** Every form of `text` the matchers look at, folded. */
export function views(text) {
  const forms = new Set([fold(text), fold(decode(text))]);
  for (const b of base64Texts(text)) {
    forms.add(fold(b));
    forms.add(fold(decode(b)));
  }
  return [...forms];
}

/**
 * Reads the labels in `dirs`. `problems` says, without naming any file
 * inside a folder, what couldn't be read: a missing folder, or a count of
 * label files that aren't readable JSON.
 */
export function readCorpus(dirs) {
  const labels = [];
  const problems = [];
  for (const dir of dirs) {
    let files;
    try {
      if (!existsSync(dir)) {
        problems.push(`${dir} doesn't exist`);
        continue;
      }
      if (!statSync(dir).isDirectory()) {
        problems.push(`${dir} isn't a folder`);
        continue;
      }
      files = labelFiles(dir);
    } catch {
      problems.push(`${dir} can't be read`);
      continue;
    }
    let unreadable = 0;
    for (const f of files)
      try {
        labels.push(JSON.parse(readFileSync(f, "utf8")));
      } catch {
        unreadable += 1;
      }
    if (unreadable)
      problems.push(`${unreadable} label file(s) in ${dir} can't be read`);
    if (files.length === 0) problems.push(`${dir} holds no labels`);
  }
  return { labels, problems };
}

/**
 * The values to block, from the labels in `dirs`, less any value the
 * fictional samples in `samplesDir` also use. Each is `{ kind, match }`;
 * `match(views, file)` says whether a line, in the forms `views` gives,
 * contains it.
 */
export function loadSecrets(dirs, samplesDir) {
  const { labels, problems } = readCorpus(dirs);
  const samples = readCorpus([samplesDir]).labels;

  const collect = (labelsList) => {
    const vendors = new Set();
    const amounts = new Set();
    const references = new Set();
    for (const label of labelsList) {
      for (const v of list(label.vendor))
        if (typeof v === "string" && v.trim().length >= MIN_VENDOR_LENGTH)
          vendors.add(fold(v));
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
          references.add(fold(v));
    }
    return { vendors, amounts, references };
  };
  const real = collect(labels);
  const fictional = collect(samples);
  // A line item on a sample page is as fictional as its total, so quoting the
  // sample never trips the guard.
  for (const page of samplePages(samplesDir))
    for (const c of amountsIn(page)) fictional.amounts.add(c);
  const fictionalWords = new Set(
    [...fictional.vendors].flatMap((v) => v.split(/[^a-z0-9']+/)),
  );

  const secrets = [];
  const word = (kind, text, ordinary = false) => {
    const re = new RegExp(`(?<![a-z0-9])${escape(text)}(?![a-z0-9])`);
    secrets.push({
      kind,
      match: (forms, line) =>
        !(ordinary && line?.ordinary?.has(text)) &&
        forms.some((f) => re.test(f)),
    });
  };
  const words = new Set();
  for (const v of real.vendors) {
    if (fictional.vendors.has(v)) continue;
    word("vendor name", v);
    const parts = v.split(/[^a-z0-9']+/).filter(Boolean);
    if (parts.length > 1)
      for (const w of parts)
        if (
          /[a-z]{4}/.test(w) &&
          w.replace(/[^a-z]/g, "").length >= MIN_VENDOR_WORD &&
          !GENERIC_WORDS.has(w) &&
          !fictionalWords.has(w) &&
          !real.vendors.has(w)
        )
          words.add(w);
  }
  for (const w of words) word("part of a vendor name", w, true);
  for (const r of real.references)
    if (!fictional.references.has(r)) word("receipt number", r);
  const blocked = new Set(
    [...real.amounts].filter((c) => !fictional.amounts.has(c)),
  );
  if (blocked.size > 0) {
    secrets.push({
      kind: "amount",
      match: (forms) =>
        forms.some((f) => amountsIn(f).some((c) => blocked.has(c))),
    });
    secrets.push({
      kind: "amount in minor units",
      match: (forms, line) =>
        forms.some((f) =>
          minorAmountsIn(f).some(
            (c) => blocked.has(c) && !line?.ordinary?.has(String(c)),
          ),
        ),
    });
  }
  const minors = [...blocked]
    .map(String)
    .filter((c) => c.length >= MIN_MINOR_DIGITS);
  return {
    secrets,
    labelCount: labels.length,
    problems,
    // What `ordinaryTerms` looks for in the tree before a change.
    terms: [...words, ...minors],
  };
}

/**
 * Which of `terms` the tree at `rev` already contains as whole words,
 * folded. Reads each text file once and looks its words up, so the terms
 * never reach a command line.
 */
export function ordinaryTerms(root, rev, terms) {
  const wanted = new Set(terms);
  const found = new Set();
  if (!wanted.size) return found;
  let listing;
  try {
    listing = execFileSync("git", ["ls-tree", "-r", "-z", rev], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "ignore"],
    });
  } catch {
    return found; // No such tree (a first commit): nothing is ordinary.
  }
  const blobs = listing
    .split("\0")
    .map((entry) => /^\d+ blob ([0-9a-f]+)\t/.exec(entry)?.[1])
    .filter(Boolean);
  if (!blobs.length) return found;
  const batch = execFileSync("git", ["cat-file", "--batch"], {
    cwd: root,
    input: blobs.join("\n") + "\n",
    maxBuffer: 1024 * 1024 * 1024,
  });
  // Each object is "<sha> blob <size>\n<bytes>\n".
  let at = 0;
  while (at < batch.length && found.size < wanted.size) {
    const eol = batch.indexOf(10, at);
    const size = Number(batch.toString("latin1", at, eol).split(" ")[2]);
    const body = batch.subarray(eol + 1, eol + 1 + size);
    at = eol + 1 + size + 1;
    if (body.includes(0)) continue; // Binary.
    for (const [token] of body
      .toString("utf8")
      .toLowerCase()
      .matchAll(/[a-z0-9']+/g))
      if (wanted.has(token)) found.add(token);
  }
  return found;
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

/** The files a `git diff` shows only as "Binary files ... differ". */
export function binaryFiles(diff) {
  const out = [];
  for (const m of diff.matchAll(
    /^Binary files (?:a\/)?(.+?) and (?:b\/)?(.+?) differ$/gm,
  ))
    if (m[2] !== "/dev/null") out.push(m[2]);
  return out;
}

/**
 * A trailer exempt from the check: exactly `Co-Authored-By: Name <email>`.
 * Anything else on a line that starts the same way is checked.
 */
const CO_AUTHOR = /^co-authored-by: [^<>\n]+ <[^<>\s@]+@[^<>\s@]+>\s*$/i;

/** The lines of a commit message git keeps: no comments, nothing below the scissors. */
export function messageLines(message, file = "commit message") {
  const out = [];
  const lines = message.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const text = lines[i];
    if (/^# -+ >8 -+/.test(text)) break;
    if (text.startsWith("#")) continue;
    // Attribution trailers name people and tools, never receipt content.
    if (CO_AUTHOR.test(text)) continue;
    out.push({ file, line: i + 1, text });
  }
  return out;
}

/** How a line is recorded in the allow-list: its SHA-256, never its text. */
export const lineHash = (text) =>
  createHash("sha256").update(text.trimEnd()).digest("hex");

/**
 * Where `lines` contain a secret: `{ file, line, kind }`, never the value.
 * Lines under `samplesDir` and lines whose hash is in `allowed` are skipped.
 */
export function findLeaks(
  lines,
  secrets,
  samplesDir = SAMPLES_DIR,
  allowed = new Set(),
) {
  const leaks = [];
  for (const { file, line, text, ordinary } of lines) {
    if (file.startsWith(`${samplesDir}/`)) continue;
    if (allowed.size && allowed.has(lineHash(text))) continue;
    const forms = views(text);
    const kinds = secrets
      .filter((s) => s.match(forms, { file, line, text, ordinary }))
      .map((s) => s.kind);
    // A whole name already says what its words would.
    const named = kinds.includes("vendor name");
    for (const kind of kinds) {
      if (named && kind === "part of a vendor name") continue;
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

/** The allow-list file, which must be outside the repository at `root`. */
export function allowFile(root, env = process.env) {
  const path = resolve(
    (env.ZAPBOOKS_PRIVACY_ALLOW ?? "").replace(/^~(?=$|\/)/, homedir()) ||
      join(homedir(), ".config", "zapbooks", "privacy-allow"),
  );
  const inside = relative(resolve(root), path);
  if (!inside.startsWith("..") && !isAbsolute(inside))
    throw new Error(
      `The allow-list ${path} is inside the repository; keep it outside, where it can't be committed.`,
    );
  return path;
}

/** The line hashes in the allow-list; `# comments` and blank lines ignored. */
export function readAllowed(path) {
  if (!existsSync(path)) return new Set();
  return new Set(
    readFileSync(path, "utf8")
      .split("\n")
      .map((l) => l.replace(/#.*/, "").trim())
      .filter((l) => /^[0-9a-f]{64}$/.test(l)),
  );
}

/**
 * Whether to block for want of a corpus, and what to say. An explicit skip
 * (the variable set to a reason of eight characters or more) is the only
 * way past a missing or unreadable corpus.
 */
export function corpusVerdict(
  { labelCount, problems },
  dirs,
  env = process.env,
) {
  if (labelCount > 0 && problems.length === 0) return { block: false };
  const reason = (env[SKIP_VARIABLE] ?? "").trim();
  if (reason.length >= 8)
    return {
      block: false,
      message: `privacy guard: NOT CHECKED (${SKIP_VARIABLE}: ${reason}).`,
    };
  return {
    block: true,
    message: [
      "privacy guard: blocked, because the receipt labels it checks against can't all be read:",
      ...(problems.length ? problems : ["no labels found"]).map(
        (p) => `  ${p}`,
      ),
      `Folders: ${dirs.join(", ")} (set ZAPBOOKS_CORPUS_DIRS to change them).`,
      "Without them nothing is checked. CI has no corpus either, so it can't catch what this misses.",
      `If you're sure this change holds no real receipt values, say why in ${SKIP_VARIABLE}, e.g.`,
      `  ${SKIP_VARIABLE}="corpus is on the other laptop; docs-only change" git commit ...`,
    ].join("\n"),
  };
}

const git = (root, args) =>
  execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });

const DIFF = ["--no-color", "--no-ext-diff", "-U0"];

/** The commits a push sends, for `git push`'s stdin lines. */
export function outgoingCommits(root, remote, input) {
  const commits = [];
  for (const line of input.split("\n")) {
    const [, local, , remoteSha] = line.trim().split(/\s+/);
    if (!local || ZERO_SHA.test(local)) continue;
    const not = ["--not", `--remotes=${remote}`];
    if (remoteSha && !ZERO_SHA.test(remoteSha))
      try {
        git(root, ["cat-file", "-e", `${remoteSha}^{commit}`]);
        not.push(remoteSha);
      } catch {
        // Not here: everything not on the remote's branches is new.
      }
    for (const c of git(root, ["rev-list", local, ...not]).split("\n"))
      if (c && !commits.includes(c)) commits.push(c);
  }
  return commits;
}

/**
 * A commit's own changes (against its first parent) and its message, each
 * line with what its parent tree already held.
 */
function commitLines(root, commit, terms, cache) {
  const short = commit.slice(0, 10);
  let parent = EMPTY_TREE;
  try {
    parent = git(root, [
      "rev-parse",
      "--verify",
      "--quiet",
      `${commit}^1`,
    ]).trim();
  } catch {
    // A root commit: compare with the empty tree.
  }
  const diff = git(root, ["diff", ...DIFF, parent, commit]);
  if (!cache.has(parent)) cache.set(parent, ordinaryTerms(root, parent, terms));
  const ordinary = cache.get(parent);
  return {
    lines: [
      ...addedLines(diff).map((l) => ({ ...l, file: `${short} ${l.file}` })),
      ...messageLines(
        git(root, ["log", "-1", "--format=%B", commit]),
        `${short} commit message`,
      ),
    ].map((l) => ({ ...l, ordinary })),
    binaries: binaryFiles(diff).map((f) => `${short} ${f}`),
  };
}

function report(leaks, binaries, what) {
  if (binaries.length)
    console.error(
      `privacy guard: ${binaries.length} binary file(s) not checked; look at them yourself:\n` +
        binaries.map((b) => `  ${b}`).join("\n"),
    );
  if (leaks.length === 0) return 0;
  console.error(
    `privacy guard: ${what} contains values from real receipt labels.`,
  );
  for (const { file, line, kind } of leaks)
    console.error(`  ${file}:${line}: a real receipt's ${kind}`);
  console.error(
    [
      "Use invented values instead. (The values aren't printed, so they stay out of terminal history and logs.)",
      "If a line is a false positive, allow it as it stands with",
      "  node scripts/hooks/privacy-guard.mjs allow <file>:<line>",
      "which records only a hash of the line, outside the repository. Don't use --no-verify.",
    ].join("\n"),
  );
  return 1;
}

function allow(root, spec, note) {
  const m = /^(.+):(\d+)$/.exec(spec ?? "");
  if (!m) {
    console.error("Usage: privacy-guard.mjs allow <file>:<line> [note]");
    return 2;
  }
  const text = readFileSync(resolve(root, m[1]), "utf8").split("\n")[
    Number(m[2]) - 1
  ];
  if (text === undefined) {
    console.error(`${m[1]} has no line ${m[2]}.`);
    return 2;
  }
  const path = allowFile(root);
  mkdirSync(dirname(path), { recursive: true });
  appendFileSync(
    path,
    `${lineHash(text)}  # ${spec}${note ? ` ${note}` : ""}\n`,
  );
  console.log(`Allowed ${spec} as it stands now, in ${path}.`);
  return 0;
}

function main(argv, stdin) {
  const root = git(process.cwd(), ["rev-parse", "--show-toplevel"]).trim();
  // Before core.hooksPath, a hook ran this with no command (pre-commit) or
  // with just the message file (commit-msg).
  const commands = ["pre-commit", "commit-msg", "pre-push", "check", "allow"];
  const [command, ...args] = commands.includes(argv[0])
    ? argv
    : argv.length
      ? ["commit-msg", ...argv]
      : ["pre-commit"];
  if (command === "allow") return allow(root, args[0], args.slice(1).join(" "));

  const dirs = corpusDirs();
  const corpus = loadSecrets(dirs, join(root, SAMPLES_DIR));
  const verdict = corpusVerdict(corpus, dirs);
  if (verdict.message) console.error(verdict.message);
  if (verdict.block) return 1;
  if (corpus.labelCount === 0) return 0;
  const allowed = readAllowed(allowFile(root));
  const leaks = (lines) =>
    findLeaks(lines, corpus.secrets, SAMPLES_DIR, allowed);
  // What the committed tree already holds, for everything but a push.
  const atHead = (lines) => {
    const ordinary = ordinaryTerms(root, "HEAD", corpus.terms);
    return lines.map((l) => ({ ...l, ordinary }));
  };

  if (command === "commit-msg")
    return report(
      leaks(atHead(messageLines(readFileSync(args[0], "utf8")))),
      [],
      "this commit message",
    );
  if (command === "check")
    return report(
      leaks(
        atHead(
          args.flatMap((f) =>
            readFileSync(f, "utf8")
              .split("\n")
              .map((text, i) => ({ file: f, line: i + 1, text })),
          ),
        ),
      ),
      [],
      "this text",
    );
  if (command === "pre-push") {
    const all = { lines: [], binaries: [] };
    const cache = new Map();
    for (const c of outgoingCommits(root, args[0] ?? "origin", stdin())) {
      const { lines, binaries } = commitLines(root, c, corpus.terms, cache);
      all.lines.push(...lines);
      all.binaries.push(...binaries);
    }
    return report(leaks(all.lines), all.binaries, "this push");
  }
  const diff = git(root, ["diff", "--cached", ...DIFF]);
  return report(
    leaks(atHead(addedLines(diff))),
    binaryFiles(diff),
    "this commit",
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exit(
    main(process.argv.slice(2), () => {
      try {
        return readFileSync(0, "utf8");
      } catch {
        return "";
      }
    }),
  );
}
