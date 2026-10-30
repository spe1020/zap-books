// The hooks as git runs them, in a throwaway repository with invented
// labels. No real receipt values appear here.

import { execFileSync, spawnSync } from "node:child_process";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const here = new URL(".", import.meta.url).pathname;
const root = realpathSync(mkdtempSync(join(tmpdir(), "privacy-hooks-")));
const repo = join(root, "repo");
const remote = join(root, "remote.git");
const corpus = join(root, "corpus");
const allowList = join(root, "allow");
const VENDOR = "Quillfeather Stationers";

const env = {
  PATH: process.env.PATH,
  HOME: root,
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: join(root, "gitconfig"),
  GIT_AUTHOR_NAME: "Invented Person",
  GIT_AUTHOR_EMAIL: "invented@example.com",
  GIT_COMMITTER_NAME: "Invented Person",
  GIT_COMMITTER_EMAIL: "invented@example.com",
  ZAPBOOKS_CORPUS_DIRS: corpus,
  ZAPBOOKS_PRIVACY_ALLOW: allowList,
};

/** Runs git (or node) in `cwd`; returns status and output, never throws. */
function run(cmd, args, { cwd = repo, extra = {}, input } = {}) {
  const r = spawnSync(cmd, args, {
    cwd,
    env: { ...env, ...extra },
    encoding: "utf8",
    input,
  });
  return { status: r.status, out: `${r.stdout}${r.stderr}` };
}
const git = (args, opts) => run("git", args, opts);
const ok = (args, opts) => {
  const r = git(args, opts);
  if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.out}`);
  return r.out;
};
function commitFile(name, text, opts = {}) {
  writeFileSync(join(opts.cwd ?? repo, name), text);
  ok(["add", name], opts);
  return git(
    [
      "commit",
      "-q",
      "-m",
      opts.message ?? `Add ${name}`,
      ...(opts.flags ?? []),
    ],
    opts,
  );
}

beforeAll(() => {
  mkdirSync(corpus);
  writeFileSync(
    join(corpus, "a.expected.json"),
    JSON.stringify({ vendor: VENDOR, total: "1284.19", reference: "QF-55821" }),
  );
  writeFileSync(env.GIT_CONFIG_GLOBAL, "");
  execFileSync("git", ["init", "-q", "--bare", remote], { env });
  execFileSync("git", ["init", "-q", "-b", "main", repo], { env });
  ok(["remote", "add", "origin", remote]);
  // A first commit from before the hooks existed, for the worktree test.
  commitFile("README.md", "An invented project.\n");
  mkdirSync(join(repo, "scripts", "hooks"), { recursive: true });
  for (const f of [
    "pre-commit",
    "commit-msg",
    "pre-push",
    "privacy-guard.mjs",
    "install.mjs",
  ])
    cpSync(join(here, f), join(repo, "scripts", "hooks", f));
  ok(["add", "scripts"]);
  ok(["commit", "-q", "-m", "Add the hooks"]);
  const installed = run("node", ["scripts/hooks/install.mjs"]);
  expect(installed.status, installed.out).toBe(0);
  ok(["push", "-q", "origin", "main"]);
});

afterAll(() => rmSync(root, { recursive: true, force: true }));

describe("the hooks", () => {
  it("are installed through core.hooksPath (PRIV-7)", () => {
    expect(ok(["config", "core.hooksPath"]).trim()).toBe(
      join(repo, "scripts", "hooks"),
    );
  });

  it("block a staged leak and a leaking message, naming file and line only", () => {
    const staged = commitFile(
      "notes.md",
      "fine\nBought at quillfeather stationers\n",
    );
    expect(staged.status).not.toBe(0);
    expect(staged.out).toContain("notes.md:2: a real receipt's vendor name");
    expect(staged.out).not.toMatch(/quillfeather/i);
    const message = commitFile("notes.md", "fine\n", {
      message: "Total was 128419 in minor units",
    });
    expect(message.status).not.toBe(0);
    expect(message.out).toContain("commit message:1");
    expect(commitFile("notes.md", "fine\n").status).toBe(0);
  });

  it("block when the corpus is missing, unless a reason is given (PRIV-3)", () => {
    const missing = { ZAPBOOKS_CORPUS_DIRS: join(root, "nowhere") };
    const blocked = commitFile("a.md", "a\n", { extra: missing });
    expect(blocked.status).not.toBe(0);
    expect(blocked.out).toContain("doesn't exist");
    ok(["reset", "-q"]);
    const skipped = commitFile("a.md", "a\n", {
      extra: {
        ...missing,
        ZAPBOOKS_PRIVACY_GUARD_SKIP: "invented reason here",
      },
    });
    expect(skipped.status, skipped.out).toBe(0);
    expect(skipped.out).toContain(
      "NOT CHECKED (ZAPBOOKS_PRIVACY_GUARD_SKIP: invented reason here)",
    );
  });

  it("check every outgoing commit when pushing, so a cherry-pick can't slip past (PRIV-4)", () => {
    ok(["push", "-q", "origin", "main"]);
    // A commit made where nothing checked it, then cherry-picked: neither
    // runs a commit hook.
    ok(["switch", "-q", "-c", "elsewhere"]);
    commitFile("b.md", `Receipt QF-55821\n`, { flags: ["--no-verify"] });
    const picked = ok(["rev-parse", "HEAD"]).trim();
    ok(["switch", "-q", "main"]);
    ok(["cherry-pick", picked]);
    const pushed = git(["push", "origin", "main"]);
    expect(pushed.status).not.toBe(0);
    expect(pushed.out).toMatch(
      /[0-9a-f]{10} b\.md:1: a real receipt's receipt number/,
    );
    expect(pushed.out).not.toMatch(/55821/);
    // Replaced with an invented value, the push goes through.
    ok(["reset", "-q", "--hard", "origin/main"]);
    commitFile("b.md", "Receipt INV-0001\n");
    expect(git(["push", "-q", "origin", "main"]).status).toBe(0);
  });

  it("check what a merge itself adds, and its message (PRIV-4)", () => {
    ok(["switch", "-q", "-c", "side", "origin/main"]);
    commitFile("c.md", "side\n");
    ok(["switch", "-q", "main"]);
    // Both parents are clean; the value arrives in the merge's resolution,
    // which no commit hook checks.
    ok(["merge", "-q", "--no-ff", "--no-commit", "side"]);
    writeFileSync(join(repo, "c.md"), "side\nTOTAL 1,284.19\n");
    ok(["add", "c.md"]);
    ok(["commit", "-q", "--no-verify", "-m", "Merge side"]);
    const pushed = git(["push", "origin", "main"]);
    expect(pushed.status).not.toBe(0);
    expect(pushed.out).toContain("c.md:2: a real receipt's amount");
    ok(["reset", "-q", "--hard", "origin/main"]);
    // A clean merge with a leaking message is caught too.
    ok([
      "merge",
      "-q",
      "--no-ff",
      "--no-verify",
      "-m",
      "Merge QF-55821",
      "side",
    ]);
    const message = git(["push", "origin", "main"]);
    expect(message.status).not.toBe(0);
    expect(message.out).toContain(
      "commit message:1: a real receipt's receipt number",
    );
    ok(["reset", "-q", "--hard", "origin/main"]);
  });

  it("run in a worktree checked out before the hooks existed (PRIV-7)", () => {
    const first = ok(["rev-list", "--max-parents=0", "HEAD"]).trim();
    const tree = join(root, "old-worktree");
    ok(["worktree", "add", "-q", "-b", "old", tree, first]);
    const blocked = commitFile("d.md", `${VENDOR}\n`, { cwd: tree });
    expect(blocked.status).not.toBe(0);
    expect(blocked.out).toContain("d.md:1: a real receipt's vendor name");
  });

  it("warn loudly when the guard script itself is missing", () => {
    const script = join(repo, "scripts", "hooks", "privacy-guard.mjs");
    renameSync(script, `${script}.away`);
    try {
      const r = commitFile("e.md", "e\n");
      expect(r.status).toBe(0);
      for (const hook of ["pre-commit", "commit-msg"])
        expect(r.out).toContain(
          `This ${hook} was NOT checked for real receipt values.`,
        );
    } finally {
      renameSync(`${script}.away`, script);
    }
  });

  it("let a false positive through once allowed, keeping only a hash (PRIV-6)", () => {
    writeFileSync(
      join(repo, "f.md"),
      "Quillfeather Stationers, in an old story\n",
    );
    const allowed = run("node", [
      "scripts/hooks/privacy-guard.mjs",
      "allow",
      "f.md:1",
    ]);
    expect(allowed.status, allowed.out).toBe(0);
    expect(readFileSync(allowList, "utf8")).not.toMatch(/quillfeather/i);
    ok(["add", "f.md"]);
    const r = git(["commit", "-q", "-m", "Add f.md"]);
    expect(r.status, r.out).toBe(0);
    // The same name on another line is still blocked.
    expect(
      commitFile("g.md", "Quillfeather Stationers again\n").status,
    ).not.toBe(0);
  });
});
