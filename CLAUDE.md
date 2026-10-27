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
- `npm run hooks:install` installs a pre-commit and commit-msg guard that
  blocks commits containing values from the corpus labels. It names the file
  and line, never the value. Don't bypass it with `--no-verify`; replace the
  value instead. It can't check PR descriptions, so check those by hand.
