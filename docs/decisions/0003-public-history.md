# Decision 0003: Public history keeps a few coincidental values

**Status:** Accepted (2026-10-29)
**Save as:** `docs/decisions/0003-public-history.md`

## Context

The repository is public, so everything in its git history is public. The October 2026 audit ran the privacy guard's matchers over every commit and found values in history that coincide with values on real receipts in the private benchmark corpus:

- **A few fictional demo values.** Round amounts in the demo seed, in the parity fixtures generated from it, and in one test string. They were invented for the demo. Some later turned out to equal amounts on real receipts. They have since been replaced on the main branch, but older commits still hold them.
- **One short number in a test.** A numeric literal in an extractor test table that equals a receipt number on a real receipt. It was replaced when the guard was added, and that commit's diff shows which line changed.

None of these appears with a vendor, a date or a person, in the same line or the same commit. This record describes them only by kind, never by value.

## Decision

Seth reviewed the findings and chose **not to rewrite history**.

## Why

- **Little is exposed.** A bare round amount or a short number identifies no one. Without a vendor, a date or a name beside it, it can't be tied to a purchase.
- **A rewrite has real costs.** Every commit after the rewritten ones would get a new id. That breaks the commit references in pull requests, briefs, `PARITY_NOTES.md` and this repository's own docs, and it breaks every existing clone and branch. Removing the old objects from GitHub would also need GitHub support.
- **It wouldn't be complete.** Forks, clones and caches made before a rewrite keep the old commits anyway.

## Going forward

- The privacy guard blocks real values before a commit and before a push, and fails closed when it can't read the corpus (audit PRIV-3, PRIV-4).
- When the guard flags a value that is already committed, replace it in a commit whose message doesn't single out the line, or batch it with other edits, so the fix doesn't point at the value.
- If anything more identifying is ever found in history (a vendor with an amount, a date or a name), this decision doesn't cover it. Decide that case on its own.
