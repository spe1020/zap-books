// `npm run hooks:install`: points core.hooksPath at this folder in the main
// checkout, so the pre-commit, commit-msg and pre-push hooks here run in
// every worktree and checkout of this clone, at whatever commit each is on.
// Hooks in .git/hooks stop running once core.hooksPath is set, so it refuses
// if one there isn't the privacy guard, or if core.hooksPath already points
// somewhere else.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

const MARKER = "zapbooks privacy guard";

const git = (cwd, ...args) => {
  try {
    return execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
  } catch {
    return "";
  }
};

/** Sets core.hooksPath for the clone at `cwd`; returns [exit code, message]. */
export function install(cwd = process.cwd()) {
  const top = git(cwd, "rev-parse", "--show-toplevel");
  const common = resolve(
    top,
    git(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir"),
  );
  // The main checkout holds .git; a linked worktree's own folder may go away.
  const main = basename(common) === ".git" ? dirname(common) : top;
  const hooks = join(main, "scripts", "hooks");
  if (!existsSync(join(hooks, "privacy-guard.mjs")))
    return [
      1,
      `${hooks} has no privacy guard; check out a current branch there first.`,
    ];

  const current = git(cwd, "config", "--get", "core.hooksPath");
  if (current && resolve(main, current) !== hooks)
    return [
      1,
      `core.hooksPath is already ${current}; left it alone. Unset it to install.`,
    ];

  const old = join(common, "hooks");
  const foreign = existsSync(old)
    ? readdirSync(old).filter(
        (name) =>
          !name.endsWith(".sample") &&
          !readFileSync(join(old, name), "utf8").includes(MARKER),
      )
    : [];
  if (foreign.length)
    return [
      1,
      `${old} holds hooks that would stop running (${foreign.join(", ")}); left everything alone.`,
    ];

  execFileSync("git", ["config", "core.hooksPath", hooks], { cwd });
  return [0, `core.hooksPath is now ${hooks}.`];
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === resolve(import.meta.filename)
) {
  const [code, message] = install();
  (code ? console.error : console.log)(message);
  process.exit(code);
}
