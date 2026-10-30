// What the CI workflow must keep: a read-only token, pinned actions and
// images, and a locked dependency set. Read as text; no YAML parser needed.

import { readFileSync } from "node:fs";
import { expect, test } from "vitest";

const workflow = readFileSync(
  new URL("../.github/workflows/ci.yml", import.meta.url),
  "utf8",
);
const lines = workflow.split("\n");

test("the token can only read the repository (SC-1)", () => {
  const at = lines.indexOf("permissions:");
  expect(at, "a top-level permissions block").toBeGreaterThan(-1);
  expect(lines[at + 1]).toBe("  contents: read");
  expect(lines[at + 2] ?? "").not.toMatch(/^ {2}\S/);
  expect(workflow).not.toMatch(/: write\b/);
});

test("every action is pinned to a full commit, with its release noted (SC-1)", () => {
  const uses = lines.filter((l) => /^\s*-?\s*uses:/.test(l));
  expect(uses.length).toBeGreaterThan(0);
  for (const line of uses)
    expect(line, line).toMatch(
      /uses: [\w.-]+\/[\w.-]+@[0-9a-f]{40} # v\d+\.\d+\.\d+$/,
    );
});

test("clippy and the tests run against Cargo.lock as committed (SC-2)", () => {
  const cargo = lines.filter((l) => /run: cargo (clippy|test)\b/.test(l));
  expect(cargo).toHaveLength(2);
  for (const line of cargo) expect(line, line).toContain("--locked");
});

test("the runner and the database image are pinned versions (SC-4)", () => {
  const runner = lines.find((l) => l.includes("runs-on:"));
  expect(runner).toMatch(/runs-on: ubuntu-\d+\.\d+$/);
  const image = lines.find((l) => l.includes("image: postgres"));
  expect(image).toMatch(/image: postgres:\d+\.\d+@sha256:[0-9a-f]{64}$/);
});

test("Playwright adds no sandbox flag of its own (SC-4)", () => {
  const config = readFileSync(
    new URL("../playwright.config.ts", import.meta.url),
    "utf8",
  );
  expect(config).not.toMatch(/args:.*--no-sandbox/);
});
