# Notes for Claude

## Real receipts stay out of the repository

The receipt benchmark runs on real receipts kept outside the repository (by
default `~/Desktop/reciepts` and `~/Desktop/receipts-holdout`).

- Never put real receipt values in tests, docs, code comments, commit messages
  or PR descriptions: no vendor names, amounts, receipt or order numbers,
  dates, times, or distinctive printed labels from those receipts. Use
  invented values. The fictional samples in `docs/samples/` are the only
  receipts that may be quoted.
- A value that only resembles a real one (same format, same date, a short
  number like an order number) still counts. Change it.
- Bench output that names files or values (`*.details.local.md`) stays local
  and is gitignored. Keep receipt values only in temporary files, and delete
  them when done.
- Check by hand what no tool checks: **dates and times**, **PR descriptions**
  (run `node scripts/hooks/privacy-guard.mjs check <file>` on the body, then
  read it for dates), addresses, phone numbers and other printed labels the
  corpus labels don't record, binary files the guard lists, and the
  "resembles a real value" rule above.

## What the privacy guard enforces

`npm run hooks:install` points `core.hooksPath` at `scripts/hooks` in the
main checkout, so every worktree and checkout of the clone runs its hooks,
whatever commit it's on. `scripts/hooks/privacy-guard.mjs` explains the
details.

- **pre-commit** checks the staged diff, **commit-msg** the message, and
  **pre-push** every outgoing commit's diff and message. The pre-push check
  is what covers rebase, cherry-pick and merge, which skip the commit hooks.
- It matches vendor names (whole, and each distinctive word of four or more
  letters), amounts (decimal with any thousands separator, and in minor
  units) and receipt numbers. It also reads text through HTML entities,
  %-escapes, `\u` escapes and base64, and ignores accents, zero-width
  characters and "and" written as "&". It names the file, line and kind,
  never the value. A single vendor word or minor-unit number that the
  repository already contained before the change counts as ordinary
  vocabulary and isn't matched on its own. A whole vendor name and every
  printed amount are always matched.
- It **fails closed**. If a corpus folder is missing or a label can't be
  read, it blocks. The only way past is to set
  `ZAPBOOKS_PRIVACY_GUARD_SKIP` to the reason. If the guard script itself is
  missing, the hooks warn loudly but can't check anything.
- A false positive is allowed with
  `node scripts/hooks/privacy-guard.mjs allow <file>:<line>`. This records a
  hash of that exact line in `~/.config/zapbooks/privacy-allow`, outside the
  repository. Never use `--no-verify`. Replace the value, or allow the line.
- **CI can't run the guard.** It has no corpus, and the corpus must never be
  there. The guard on this machine is the only check, so a commit made
  elsewhere is checked when this machine pushes it.
- Commits made in GitHub's web UI, and squash-merge messages composed there,
  run no hook. Check them by hand.
