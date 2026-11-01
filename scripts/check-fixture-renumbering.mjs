#!/usr/bin/env node
/**
 * Checks that the parity fixtures changed only by id renumbering since a
 * base commit (default: origin/business-finance-mvp).
 *
 *   node scripts/check-fixture-renumbering.mjs [--appended-columns] [base]
 *
 * Every fixture (and state file) in the base must still exist, and be equal
 * to its new copy once sequential ids (`id-0001`) are renamed in order of
 * first appearance and `env.nextId` is set aside: the only difference a
 * shifted id counter can make. Fixtures new since the base are listed, not
 * compared. Exits 1 if anything else changed.
 *
 * With `--appended-columns`, a fixture that changed only by cells added at
 * the end of its rows is listed as such and allowed: each list of strings
 * (a CSV row or its headers) may only grow at its end, and each CSV text
 * only by cells added at the end of each line. For a change agreed to add
 * export columns; the default refuses it.
 */
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";

const args = process.argv.slice(2);
const allowAppended = args.includes("--appended-columns");
const base =
  args.find((a) => !a.startsWith("--")) ?? "origin/business-finance-mvp";
const root = new URL("..", import.meta.url).pathname;
const dir = "fixtures/parity";

const git = (...args) =>
  execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 1 << 30,
  });

/** The fixture files at `base`, relative to the repository. */
const baseFiles = git("ls-tree", "-r", "--name-only", base, "--", dir)
  .split("\n")
  .filter((f) => f.endsWith(".json"));

/** `value` with ids renamed in order of first appearance. */
function renumber(value) {
  const names = new Map();
  const rename = (s) =>
    s.replace(/id-\d{4,}/g, (id) => {
      if (!names.has(id)) names.set(id, `#${names.size}`);
      return names.get(id);
    });
  const walk = (v) => {
    if (typeof v === "string") return rename(v);
    if (Array.isArray(v)) return v.map(walk);
    if (v && typeof v === "object")
      return Object.fromEntries(
        Object.entries(v).map(([k, x]) => [rename(k), walk(x)]),
      );
    return v;
  };
  return JSON.stringify(walk(value));
}

/**
 * Whether `after` is `before` with cells added only at the end: of a list of
 * strings, or of each line of a CSV text. Everything else must be equal.
 */
function appendedOnly(before, after) {
  if (typeof before === "string" && typeof after === "string") {
    if (before === after) return true;
    const csv = /\r\n/.test(before) || before.startsWith("\uFEFF");
    if (!csv) return false;
    const [a, b] = [before.split("\r\n"), after.split("\r\n")];
    return (
      a.length === b.length &&
      a.every((line, i) => line === b[i] || b[i].startsWith(`${line},`))
    );
  }
  if (Array.isArray(before) && Array.isArray(after)) {
    const row =
      before.every((x) => typeof x === "string") &&
      after.every((x) => typeof x === "string");
    if (row ? after.length < before.length : after.length !== before.length)
      return false;
    return before.every((x, i) => appendedOnly(x, after[i]));
  }
  if (
    before &&
    after &&
    typeof before === "object" &&
    typeof after === "object" &&
    !Array.isArray(before) &&
    !Array.isArray(after)
  ) {
    const keys = Object.keys(before);
    return (
      keys.length === Object.keys(after).length &&
      keys.every((k) => k in after && appendedOnly(before[k], after[k]))
    );
  }
  return before === after;
}

const withoutNextId = (f) =>
  f && typeof f === "object" && f.env
    ? { ...f, env: { ...f.env, nextId: null } }
    : f;

let identical = 0;
const renumbered = [];
const appended = [];
const changed = [];
const added = [];
for (const file of baseFiles) {
  const before = JSON.parse(git("show", `${base}:${file}`));
  const path = join(root, file);
  if (!existsSync(path)) {
    changed.push(`${file}: missing`);
    continue;
  }
  const after = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(before)) {
    // A state file: one record, compared whole.
    const same = JSON.stringify(before) === JSON.stringify(after);
    if (same) identical++;
    else if (renumber(before) === renumber(after)) renumbered.push(file);
    else changed.push(file);
    continue;
  }
  const byName = new Map(after.map((f) => [f.name, f]));
  for (const f of before) {
    const g = byName.get(f.name);
    const label = `${file}: ${f.name}`;
    if (!g) changed.push(`${label} (missing)`);
    else if (JSON.stringify(f) === JSON.stringify(g)) identical++;
    else if (renumber(withoutNextId(f)) === renumber(withoutNextId(g)))
      renumbered.push(label);
    else if (allowAppended && appendedOnly(f, g)) appended.push(label);
    else changed.push(label);
  }
  const names = new Set(before.map((f) => f.name));
  for (const g of after)
    if (!names.has(g.name)) added.push(`${file}: ${g.name}`);
}
const baseSet = new Set(baseFiles);
const walk = (d) =>
  readdirSync(join(root, d), { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)],
  );
for (const file of walk(dir))
  if (file.endsWith(".json") && !baseSet.has(file))
    added.push(`${file} (new file)`);

console.log(`Compared with ${base}:`);
console.log(`  ${identical} unchanged`);
console.log(`  ${renumbered.length} changed only by id renumbering`);
for (const r of renumbered) console.log(`    ${r}`);
if (allowAppended) {
  console.log(`  ${appended.length} changed only by cells added at row ends`);
  for (const a of appended) console.log(`    ${a}`);
}
console.log(`  ${added.length} new`);
for (const a of added) console.log(`    ${a}`);
console.log(`  ${changed.length} changed otherwise`);
for (const c of changed) console.log(`    ${c}`);
process.exit(changed.length ? 1 : 0);
