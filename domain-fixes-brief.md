# Brief: Domain fixes W1–W9 (follow-up to PR #1)

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Start from:** `business-finance-mvp` *after* PR #1 (`rust/ledger-core`) is merged. Pull first.
**Branch name:** `fix/domain-scoping-and-import`

## Goal

Fix the TypeScript behaviors Seth approved from `PARITY_NOTES.md`, and make the Rust crate follow. After this PR, TypeScript and Rust still agree on every fixture; they just agree on the corrected behavior.

## The one rule

For every fix, in this order, inside **one commit**:

1. Change the TypeScript in `src/domain/`.
2. Add or update the TS unit test that proves the new behavior.
3. Run `npm run parity:fixtures` to regenerate fixtures.
4. Change the Rust in `crates/ledger-core/` until `cargo test` passes against the new fixtures.
5. Update `PARITY_NOTES.md` (see "Notes file" below).

Never change the Rust alone. Never hand-edit a fixture.

## Fixture-diff discipline

After step 3 of each commit, inspect `git diff --stat fixtures/parity`. Only fixtures related to that fix should change. If an unrelated fixture changes (for example, sequential ids shift in a scenario that has nothing to do with the fix), **stop and explain it** before continuing. The PR description lists, per commit, which fixtures changed and why.

## Commit 0: housekeeping from the PR #1 review

- Add `npm run format:check` to `.github/workflows/ci.yml`, after `npm run build`.
- In the PR description, state in one or two sentences what the failure recorded in `crates/ledger-core/tests/properties.proptest-regressions` was (the GBP `Save` case), and confirm whether it was a test-harness problem or a domain bug. If it was a domain bug, stop and report before continuing.

## Commit 1: W3, payment rows must belong to the selected business and account

**Problem:** `commitImport` (`src/domain/import.ts`) checks import-source rows against the business but not payment rows. Rows previewed for another business or account can be committed, and their fingerprints then carry the wrong ids, which breaks duplicate detection on re-import.

**Fix:**

- Add `businessId: string` and `accountId: string` to `ImportRow`. `previewImport` fills them from `b.profile.id` and the `accountId` argument.
- In `commitImport`, before processing any row: if any row's `businessId !== b.profile.id` or `accountId !== accountId`, throw, and import nothing:
  `"Import rows were previewed for a different business or account. Preview the file again."`
- This check runs before the existing per-row loop so the import stays all-or-nothing.

**Tests:** TS test in `ledger.test.ts`: rows previewed for Fieldwork, committed to Northstar, throw with that message and leave Northstar unchanged. The existing fixture `import/commitImport: payment rows previewed for another business` should flip from success to this error.

## Commit 2: W1, W4, W5, scope every lookup to the business

All three are one-line filters. Do them in one commit.

- **W1** `approve` (`src/domain/ledger.ts`, the category lookup after validation): find the category by `id` **and** `businessId === b.profile.id`.
- **W4** `totals` and `categoryTotals`: skip journals where `j.businessId !== b.profile.id`. Also, in `categoryTotals`, look up the category name with the same business filter.
- **W5** `duplicates`: add `x.businessId === d.businessId` to the filter.

No new error messages. The fixtures `approve/category found by id in another business first`, `totals/*: mixed businesses`, and `validation/duplicates: drafts from another business` should flip to the corrected results.

## Commit 3: W7, allow whitespace before an opening quote

**Problem:** `parseCSV` throws `"Malformed CSV quote."` for `a, "b"`, which many exporters write.

**Fix:** When an opening quote is reached and the cell holds only whitespace, discard the whitespace and start the quoted value. If the cell holds any non-whitespace text, keep throwing `"Malformed CSV quote."`. Use the same whitespace definition as `.trim()` (the Rust side already has this in `src/js.rs`).

**W6 is not changing.** Trimming inside quoted cells stays, deliberately: the importer should never store a description or vendor with stray spaces, and changing it would alter fingerprints for files people may already have imported. Move W6 to the "Kept on purpose" section of the notes with that reason.

**Tests:** `a, "b"` parses to `["a", "b"]`; `a,x "b"` still throws. The fixture `csv/parseCSV: space before quote` flips to success.

## Commit 4: W8, "today" is the user's local date

**Problem:** `today()` uses the UTC date, so an expense created in the evening in the Americas is dated tomorrow, and `currentPeriod()` can show next month on the last evening of a month.

**Fix, TypeScript:**

- `today()` returns the **local** calendar date as `YYYY-MM-DD`, built from `getFullYear()`, `getMonth()`, and `getDate()`. Do not use `toLocaleDateString`; its format varies by locale.
- `now()` is unchanged. Timestamps (`createdAt`, `at`, `occurredAt`) stay UTC ISO-8601.
- `currentPeriod()` keeps deriving from `today()`.

**Fix, fixtures:** make the generator's timezone explicit so results don't depend on the machine:

- Change the npm script to `TZ=America/New_York vitest run --config scripts/vitest.parity.config.mjs`.
- Keep the existing frozen clock (`2026-10-15T12:00:00.000Z`, which is the same date in New York).
- Add scenarios at `2026-10-15T02:00:00.000Z` (local date `2026-10-14`) and `2026-11-01T01:30:00.000Z` (local date `2026-10-31`, period `2026-10`) for `blankDraft` and `currentPeriod`.

**Fix, Rust:**

- Add a required method to `Env`: `fn today(&self) -> String;` returning the local `YYYY-MM-DD`. The crate does not compute timezones; the caller supplies the date. Remove the crate-private `today()` helper that slices `now()`.
- The deterministic test `Env` takes the date explicitly. Parity fixtures record it alongside `now`.

**Also check:** `npm run test:e2e`. If any browser test assumed the UTC date, fix the test's expectation, not the domain code, and list it in the PR description.

## Commit 5: W2, import evidence can only come from an import

**Problem:** `saveDraft` blocks *changing* or *removing* an `importSource`, but lets a draft *gain* one, either as a new draft or an existing draft that had none. So "this came from an import" isn't guaranteed.

**Fix:**

- Split `saveDraft` into:
  - a module-private `writeDraft(b, draft, { allowImportSource })` that holds the current body;
  - public `saveDraft(b, draft)`, which calls `writeDraft` with `allowImportSource: false`.
- With `allowImportSource: false`, throw if the draft's `importSource` differs from the stored draft's (including when the stored draft has none, or the draft is new):
  `"Imported source evidence can only be added by an import."`
  Keep the existing `"Imported source evidence cannot be changed."` for the change and removal cases, so the messages say what actually happened.
- `commitImport` calls `writeDraft` with `allowImportSource: true`.
- `correct` passes the replacement through the restricted path. Its existing check already requires the replacement's `importSource` to equal the original's, so correcting an imported expense must still work. Make sure the restricted check compares against the **original** in that case, not "no prior draft." Add a test for correcting an imported expense.
- `seed.ts`: if it creates drafts with an `importSource` through `saveDraft`, switch it to the import path or `writeDraft`. Seed output must not change.

**Do not change `paymentId` handling in this commit.** Check whether a manual `saveDraft` can attach a `paymentId` the same way, and **report** what you find in the PR description. Seth will decide separately.

**Rust:** mirror the split. `write_draft` stays `pub(crate)`.

**Tests:** new draft with an `importSource` via `saveDraft` throws; existing draft gaining one throws; history import still works; correcting an imported expense still works. Fixture `drafts/saveDraft: import source added to existing draft` flips to the new error.

## Commit 6: W9, reject implausible years

- `validDate` rejects any year before `1900`. The message stays whatever the caller already uses (`"A valid expense date is required."`, `"Date does not match the selected format or is invalid."`, and so on). Do not add a new message.
- Leading zeros in `parseMoney` (`00012.30`) and `-0` stay accepted. Move those to "Kept on purpose."
- Fixtures `date/validDate "0000-01-01"` and `"0000-02-29"` flip to `false`. Add `"1899-12-31"` (false) and `"1900-01-01"` (true).

## Not in this PR

- **W10** (`""` treated as absent): deferred to step 2, where the server validates input at its boundary.
- **W11** (overflow cascades to later rows): kept; the file can't be committed either way.
- **W6**: kept, see commit 3.
- Any `paymentId` change: report only, see commit 5.
- Anything in step 2 (the accounting service) or step 3 (WASM).

## Notes file

Restructure `PARITY_NOTES.md` W section into three parts:

- **Open:** W10, W11 (with the reason each is deferred or kept).
- **Kept on purpose:** W6, the leading-zero and `-0` parts of W9.
- **Resolved:** W1, W2, W3, W4, W5, W7, W8, and the year part of W9. Each entry gets one line: what changed and the commit hash.

The D section is unchanged except where a fix removes a divergence. Say so if that happens.

## Checks (all must pass, locally and in CI)

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm test
npm run build
npm run format:check
npm run test:e2e
```

Plus the CI fixture-drift check.

## Stop and ask if

- a fix changes a fixture unrelated to it;
- a fix needs a new dependency in either language;
- an e2e test fails in a way that suggests a real UI regression rather than a date expectation;
- the proptest regression in commit 0 turns out to be a domain bug.

## PR description must include

1. The results of every check above.
2. Per commit: which fixtures changed, and why.
3. The proptest regression explanation (commit 0).
4. What you found about `paymentId` and manual `saveDraft` (commit 5).
5. Any e2e expectations you updated (commit 4), with before and after.
6. The full updated `PARITY_NOTES.md`.
