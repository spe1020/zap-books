# Parity notes: `ledger-core` vs `src/domain`

`crates/ledger-core` is a behavior-preserving port of `src/domain/`. The
TypeScript is the specification: where the two disagree, the TypeScript is
right until Seth decides otherwise.

Parity is enforced by `fixtures/parity/`, recorded from the real TypeScript
functions by `scripts/gen-parity-fixtures.test.ts` (sequential ids, clock
frozen at `2026-10-15T12:00:00.000Z`, time zone `America/New_York`) and
replayed by `crates/ledger-core/tests/parity.rs`. Regenerate with
`npm run parity:fixtures`; CI fails if the committed fixtures drift.

This file has two lists:

- **D: deliberate differences.** Places where Rust cannot or should not copy
  the TypeScript exactly. None of them is reachable from data the TypeScript
  itself writes. Each is pinned by a test in
  `crates/ledger-core/tests/divergences.rs` or by the type system.
- **W: TypeScript behavior that looked wrong.** Ported exactly as written and
  pinned by a parity fixture, then decided by Seth: resolved, kept on purpose,
  or still open.

Fixtures are cited as `area/name`: the file `fixtures/parity/<area>.json` and
the fixture's `name` (`*` matches any text).

## D: Deliberate differences

### D1. Crashes become `LedgerError::Internal`

Where the TypeScript uses a non-null assertion (`!`) or reads a property of
`undefined`, it would throw a `TypeError`. Library code here must not panic,
so these return `LedgerError::Internal(..)` instead. The fixture generator
refuses to record `TypeError`s, so no fixture covers these paths.

| Site                                         | TypeScript                                                                          | Rust                                                                                                                   |
| -------------------------------------------- | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `src/domain/ledger.ts:353` (`approve`)       | `b.categories.find((x) => x.id === d.categoryId && x.businessId === b.profile.id)!` | `Internal("approved expense has no category")`. Unreachable: `validation` has already required the category.           |
| `src/domain/ledger.ts:432` (`correct`)       | `b.journals.find((x) => x.id === original.journalId)!`                              | `Internal("posted expense has no journal")`. Reached only by corrupt data (a posted expense whose journal is missing). |
| `src/domain/import.ts:211` (`previewImport`) | `rows[0].indexOf(...)` with `rows = []`                                             | `Internal("import has no header row")`. `parseCSV` never returns fewer than two rows.                                  |
| `src/domain/import.ts:456` (`commitImport`)  | `draft.paymentId!`                                                                  | No error path: the payment id is created in the same branch, so the type system guarantees it.                         |

Tests: `d1_correct_without_a_journal_is_an_internal_error`,
`d1_preview_without_a_header_is_an_internal_error`.

### D2. Range and enum checks happen when JSON is read

`Minor` is always within `±MAX_MINOR` (`Number.MAX_SAFE_INTEGER`) and
`Currency` is always `USD`, `EUR`, or `GBP`. JSON with a non-integer amount, an
amount outside the safe range, or an unsupported currency fails to
deserialize. The TypeScript would instead accept it and later report a
message. The affected checks are always true in Rust:

- `src/domain/ledger.ts:80`: "Amount must be greater than zero in whole minor
  units." still fires for `<= 0`, but never for non-integers.
- `src/domain/ledger.ts:82`: "Choose a supported currency." is unreachable.
- `src/domain/ledger.ts:133` and `:151`: the currency and safe-integer parts
  of "Invalid journal context." / "Invalid journal amounts." are unreachable.
  The `>= 0` part is still checked.
- `src/domain/import.ts:414`: the `Number.isSafeInteger` part of "Invalid
  import row." is unreachable.
- `src/domain/settings.ts` (`updateSettings`): "Choose a supported currency."
  is unreachable for a new account or category.
- `src/domain/ledger.ts` (`writeDraft`, so `saveDraft`, `correct` and
  `commitImport`): "Choose a supported currency." and "Amount must be in
  whole minor units." are unreachable. They were added to the TypeScript
  (audit WEB-1 / ACC-4) so the browser can never store a draft Rust can't
  load: a receipt reading could fill in either value. Rust needs no runtime
  check, since `ExpenseDraft` can't hold such a value, and no fixture records
  it, since the input itself wouldn't deserialize; the TypeScript is pinned
  by "a saved draft keeps to the values Rust can load (ACC-4, WEB-1)" in
  `src/domain/ledger.test.ts`, the Rust by the test below.
- `src/domain/ledger.ts:36` and `:52` (`parseMoney`, `sum`) and
  `src/domain/import.ts:311` (combined import amount) are ported as explicit
  range checks against `MAX_MINOR`, with the same messages.

Test: `d2_unrepresentable_values_are_rejected_at_deserialization`.

### D3. Journal construction and loading require business context

`JournalEntry::new(&business, entry)` is the only constructor and runs every
`assertBalanced` check, including account ownership, account currency, and
category membership. There is no context-free `TryFrom<UncheckedJournal>` or
`Deserialize` implementation for `JournalEntry`. A single wire entry can be
read as `UncheckedJournal`, then passed to `JournalEntry::new`.

Deserialize `BusinessData` or `Workspace` to load saved state. The loader reads
raw journals privately and validates each one against its containing business
before returning any state. The JSON shape is unchanged. The TypeScript
browser adapter still loads stored journals without this validation; malformed
stored state is therefore an intentional Rust loading difference.

Tests: `d3_unbalanced_stored_journals_do_not_deserialize`,
`stored_journals_validate_business_accounts_categories_and_currency`, and the
two `compile_fail` doctests on `JournalEntry`.

Some query fixtures deliberately mix valid journals from different businesses
or remove categories after construction. Their explicit `journalContexts`
metadata reconstructs the original validated journals for those in-memory
negative cases. This does not bypass the production loader; ordinary fixture
states must still deserialize and round-trip unchanged.

### D4. Import sources compare by value, not `JSON.stringify`

`src/domain/ledger.ts:252` and `:258` (`writeDraft`, behind `saveDraft`) and
`:426` (`correct`) compare `JSON.stringify(a) !== JSON.stringify(b)`, which
depends on key order and on unknown extra keys. Rust compares the structs
field by field. The results agree for every source the TypeScript creates.
They differ only for a source whose keys were reordered, for example by a
storage layer or migration that sorts keys. The TypeScript would wrongly
treat it as "changed", so W2's restricted check in `saveDraft` would throw
"Imported source evidence cannot be changed." where Rust saves the draft.

Test: `d4_import_source_comparison_ignores_key_order`.

### D5. `localeCompare` sorts by UTF-16 code unit

`src/domain/export.ts:76` and `:123-124` sort with `localeCompare` (ICU
collation). Rust compares UTF-16 code units. The two agree for the values
sorted there: ISO dates, ISO timestamps, and ids made of lower-case ASCII
letters, digits, and `-` (UUIDs, seeded ids such as `northstar-checking`, and
`id-0001`-style test ids). They can disagree for ids with upper-case or
non-ASCII characters. Implementing ICU collation would need a dependency
outside the allowed list.

Covered by every `export` fixture, which sorts generated and seeded ids.

### D6. No negative zero

`parseMoney("-0")` returns `-0` in JavaScript (`src/domain/ledger.ts:37`).
Rust has no negative zero, so it returns `0`. Both serialize as `0` and format
as `0.00`, and every comparison in the domain treats them the same.

Test: `d6_negative_zero_is_zero`. Fixtures: `money/parseMoney "-0"` and
`import/previewImport: negative zero`.

### D7. Receipt bytes and unknown fields are not carried

`SourceDocument` has no `blob` (`src/domain/types.ts:30`): file bytes belong
to a storage adapter. Unknown JSON keys on any record are dropped on read.
Neither affects domain behavior.

### D8. Unicode case-mapping tables

`toLowerCase`/`toUpperCase` (vendor and category matching, header mapping,
currency codes, export filenames) use V8's ICU tables. Rust's
`to_lowercase`/`to_uppercase` use the Unicode tables built into the Rust
standard library. Both apply full Unicode case mapping, including the Greek
final sigma, but the Unicode versions can differ, so a code point added in a
newer Unicode release could map differently. JavaScript whitespace (`trim`,
`\s`) and `\d` are reproduced exactly in `src/js.rs`.

## W: TypeScript behavior that looked wrong

Each item was ported exactly and pinned by a parity fixture, then decided by
Seth. Fixes change the TypeScript first, regenerate the fixtures, then make
the Rust follow, so both languages still agree on every fixture.

### Resolved

- **W1, W4, W5.** `approve` finds the posting category by id and business;
  `totals` and `categoryTotals` skip other businesses' journals, and
  `categoryTotals` names categories from this business only; `duplicates`
  only matches drafts in the same business. Commit `82918a6`.
- **W2.** `saveDraft` no longer lets a draft gain import evidence ("Imported
  source evidence can only be added by an import."); changing or removing it
  is still "Imported source evidence cannot be changed." Only `commitImport`
  attaches evidence, through `writeDraft` with `allowImportSource: true`;
  `correct` checks the replacement against its original's evidence. Commit
  `b63c3b7`.
- **W3.** `ImportRow` carries the `businessId` and `accountId` it was
  previewed for, and `commitImport` rejects the whole import ("Import rows
  were previewed for a different business or account. Preview the file
  again.") if any row names another. Commit `ee084c0`; the fingerprint
  recheck from `74c382e` stays as a second check.
- **W7.** `parseCSV` drops whitespace (as `.trim()` defines it) between a
  comma and an opening quote, so `a, "b"` parses to `["a", "b"]`; any other
  text before a quote is still "Malformed CSV quote." Commit `e17587b`.
- **W8.** `today()` is the user's local date (from `getFullYear`,
  `getMonth`, `getDate`), so `blankDraft` and `currentPeriod` follow it, while
  `now()` timestamps stay UTC. Rust takes the date from the new required
  `Env::today`; fixtures are recorded with `TZ=America/New_York` and store
  `today` beside `now`. Commit `1f4c5c0`.
- **W9 (dates).** `validDate` rejects years before 1900, so callers report
  their existing messages ("A valid expense date is required.", "Date does
  not match the selected format or is invalid.", "Choose valid export
  dates."). Commit `7cd3764`.
- **W12.** Only `commitImport` sets a draft's `paymentId`: `saveDraft`
  rejects a new draft with one, or any change to the stored value, with
  "Payment links can only be created by an import." (first guarded in
  `ddf319f`; W12 gives both cases this one message). `approve` rejects a
  draft unless its payment's `draftId` is the draft's id ("This payment
  belongs to a different expense."), and `correct` moves the payment's
  `draftId` to the replacement. The commit after `dbc94a9`.

- **W13.** `writeDraft` builds the stored draft from an explicit allow-list
  of caller-settable fields and takes every authority-bearing field from the
  record's own history: `paymentId` and `importSource` from the stored draft
  (or the original, for a correction; `commitImport` alone supplies its
  own), `journalId` from the stored draft, and `correctionOf` from the
  correction's original. A caller-supplied `correctionOf` or `journalId` is
  dropped, not stored; `correct` no longer sets `correctionOf` itself. The
  W12 messages are unchanged. Why: `writeDraft` had guarded `paymentId` and
  `importSource` by enumeration and stored everything else as given, so
  `correctionOf` and `journalId` were trusted by default. `duplicates`
  exempts `x.id === d.correctionOf`, so a bookkeeper could post an expense,
  save a twin naming it in `correctionOf`, and the first dropped out of
  duplicate detection with no reversal, while the export and the
  Corrections filter showed a correction that never happened. With the
  allow-list a new field on `ExpenseDraft` is inert until it is placed on one
  side or the other (2026-10-28 audit, ACC-1). Fixture:
  `drafts/saveDraft: draft claiming a correction and a journal`; tests
  `a saved draft cannot claim a correction or a journal (W13)` and
  `a_saved_draft_cannot_claim_a_correction_or_a_journal`.

- **W14.** `assertBalanced` holds every line to its account's kind: a line
  with a category is spend and must sit on an expense account (the
  category's own), and a line without one must not sit on an expense
  account. Both fail with "Invalid journal category.". Why: the category
  check only required `category.accountId === line.accountId`, so a
  category pointed at a bank or card account let `approve` post
  `[debit bank, credit bank]`, which balanced, moved no balance and still
  counted as spend in `totals`. Only damaged state could do it (`addCategory`
  and genesis always use an expense account), but the Rust loader accepted
  such journals too. The seed and every fixture already keep the rule
  (2026-10-28 audit, ACC-2). Fixtures: `validation/assertBalanced: category
on the paying account`, `validation/assertBalanced: expense account line
without a category`; tests `spend sits on an expense account, and only
spend does (ACC-2)` and `spend_sits_on_an_expense_account_and_only_spend_does`.
- **W15.** A category and its expense account keep one name. `addCategory`
  and `renameCategory` refuse a name any account uses ("That name is already
  in use.", the category's own account excepted), `renameCategory` renames
  the category's expense account with it, and `renameAccount` refuses an
  expense account ("Rename an expense account through its category."). Why:
  `addCategory` checked only the category list although it also adds an
  account, and an expense account could be renamed apart from its category,
  so the journal export's Account column could disagree with its Category
  column (2026-10-28 audit, ACC-3). The Settings screen already listed only
  bank and card accounts for renaming. Fixtures:
  `settings/renameCategory: renamed with its expense account` (was
  `renamed, its account unchanged`), `settings/addCategory: a bank account's
name is taken`, `settings/renameAccount: an expense account`,
  `settings/renameCategory: another account's name is taken`,
  `settings/renameCategory: its own account's name, in another case`; tests
  `a category and its expense account keep one name (ACC-3)` and
  `a_category_and_its_expense_account_keep_one_name`.
- **W16.** `commitImport` gives each import source a fresh id, asked for
  after the draft's own id, instead of keeping the id the preview chose; the
  `expense.matched` event names the new id. Why: on the shared ledger the
  rows come from a member's proposal, and every other new record's id comes
  from the service's environment, so rows could pick ids that collide with
  other records or with each other (2026-10-28 audit, ACC-5). The preview
  still generates an id, so `ImportRow` keeps its shape. Commit still trusts
  a referenced row's previewed date, amount and description: see
  `docs/protocol.md`. Every expense-history `commitImport` fixture now uses
  one more id; the ones that differ beyond renumbering differ only in source
  ids. Fixture: `history/commitImport: source ids from the rows are
replaced`; tests `a committed import source gets its own id, whatever the
rows say (ACC-5)` and `a_committed_import_source_gets_its_own_id`.

- **W17.** `csvCell` also neutralizes a leading `|` (after any whitespace),
  which some spreadsheets treat as a DDE command, and no longer prefixes a
  cell that is a sign followed only by digits, `.` and `,`
  (`/^[+-]\d[\d.,]*$/`): a spreadsheet reads `-12.50` or `+1` as a number,
  and a formula needs a name, a reference or an operator. Any other cell
  starting with `+` or `-` keeps its `'`, since a spreadsheet would evaluate
  `-x` or `+1 x`; so does a signed number after a space, to keep the rule
  simple. Exported amounts are never signed, so only text such as a vendor
  or description changes (2026-10-28 audit, WEB-6). Fixtures:
  `export/encodeCSV: escaping` (cases added) and
  `export/exportRecords: formula-injection escaping 1` and `2`, which now
  keep `+1` and `-2`; tests `CSV neutralizes a leading | and leaves signed
numbers alone (WEB-6)` and
  `csv_neutralizes_a_pipe_and_leaves_signed_numbers_alone`.

`74c382e` also removed the `correctionOf` exemptions from the posted payment
and import-source uniqueness checks. A real correction first reverses its
original and marks it corrected, then posts the replacement; merely naming an
existing expense as a correction cannot authorize a second posting.

### Kept on purpose

#### W6. `parseCSV` trims whitespace inside quoted cells

`src/domain/import.ts` trims every cell after unquoting, so `" y "` becomes
`y`, although RFC 4180 keeps whitespace inside quotes. Kept deliberately: the
importer should never store a description or vendor with stray spaces, and
changing it would alter fingerprints for files people may already have
imported.

Fixture: `csv/parseCSV: cells trimmed, quoted whitespace too`.

#### W9 (amounts). Leading zeros and `-0` in `parseMoney`

`parseMoney` (`src/domain/ledger.ts:27`) accepts leading zeros (`00012.30` is
`1230`) and `-0`. Both read unambiguously, so they stay accepted. The year
part of W9 is resolved above.

Fixtures: `money/parseMoney "00012.30"`, `money/parseMoney "-0"`.

### Open

#### W10. Empty strings count as absent

`commitImport` treats `error: ""` as no error (`src/domain/import.ts:356`),
and `missingDetails` treats `paymentId: ""` as no payment
(`src/domain/ledger.ts:107`). `correct` compares `paymentId` strictly
(`:423`), and so does `saveDraft`'s payment-link check, so `""` and absent
differ there. Deferred to step 2, where the server validates input at its
boundary.

Fixtures: `import/commitImport: empty error string is not an error`,
`validation/*: empty payment id is ignored`.

#### W11. One overflowing import row fails every later row

`src/domain/import.ts:310-312` keeps adding to `combinedAmount` after it
leaves the safe range, so every later valid row also reports "Combined import
amount exceeds the supported range." The Rust accumulator (an `i128`) is
written to give the same result. Kept for now: the file cannot be committed
either way.

Fixture: `import/previewImport: combined amount exceeds the safe range`.

## Not ported

- `src/domain/extraction.ts` (applying receipt suggestions and recording
  their outcomes). It is Review-screen logic, not a ledger rule. Rust has the
  `ExtractionRecord` types only (serde, `extractions` defaults to `[]`).
- `money()` display formatting (uses `Intl`; a UI concern).
- `seed.ts`. Rust tests load the seed recorded by the fixture generator
  (`fixtures/parity/states/seed.json`).
- `src/adapters/`, and the adapter-facing interfaces in `types.ts`
  (`AIProposal`, `AIAdapter`, `BusinessRepository`, `ReceiptStorage`,
  `LedgerCommands`, `LedgerQueries`, `EventTransport`).
- The ported test
  `business_separation_rejects_foreign_drafts_accounts_evidence_and_journal_context`
  keeps the domain assertions only; its `demoAI` and `MockTransport`
  assertions test adapters.
