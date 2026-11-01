#!/usr/bin/env node
/**
 * Checks that the parity fixtures changed only by id renumbering since a
 * base commit (default: origin/business-finance-mvp).
 *
 *   node scripts/check-fixture-renumbering.mjs [base]
 *
 * Every fixture (and state file) in the base must still exist, and be equal
 * to its new copy once sequential ids (`id-0001`) are renamed in order of
 * first appearance and `env.nextId` is set aside: the only difference a
 * shifted id counter can make. Fixtures new since the base are listed, not
 * compared. Exits 1 if anything else changed.
 */
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";

const base = process.argv[2] ?? "origin/business-finance-mvp";
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

const withoutNextId = (f) =>
  f && typeof f === "object" && f.env
    ? { ...f, env: { ...f.env, nextId: null } }
    : f;

let identical = 0;
const renumbered = [];
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
console.log(`  ${added.length} new`);
for (const a of added) console.log(`    ${a}`);
console.log(`  ${changed.length} changed otherwise`);
for (const c of changed) console.log(`    ${c}`);
process.exit(changed.length ? 1 : 0);
