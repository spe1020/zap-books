// `npm run hooks:install`: installs scripts/hooks/pre-commit as this
// clone's pre-commit and commit-msg hooks. A hook that isn't ours is left
// alone.

import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
} from "node:fs";
import { join, resolve } from "node:path";

const MARKER = "zapbooks privacy guard";
const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

const root = git("rev-parse", "--show-toplevel");
const hooks = resolve(root, git("rev-parse", "--git-path", "hooks"));
const source = join(root, "scripts", "hooks", "pre-commit");
mkdirSync(hooks, { recursive: true });

let failed = false;
for (const name of ["pre-commit", "commit-msg"]) {
  const target = join(hooks, name);
  if (existsSync(target) && !readFileSync(target, "utf8").includes(MARKER)) {
    console.error(
      `${target} already exists and isn't the privacy guard; left it alone.`,
    );
    failed = true;
    continue;
  }
  copyFileSync(source, target);
  chmodSync(target, 0o755);
  console.log(`Installed ${target}`);
}
process.exit(failed ? 1 : 0);
