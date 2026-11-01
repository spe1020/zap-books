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
`Currency` is always `USD`, `EUR`, `GBP` or `BTC`. JSON with a non-integer
amount, an amount outside the safe range, or an unsupported currency fails to
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
- `src/domain/transfers.ts` (`post`, behind `recordTransfer`,
  `correctTransfer` and `recordOpeningBalance`): "Choose a supported
  currency." and "Choose a transfer type." are unreachable, and "Amount must
  be greater than zero in whole minor units." fires only for `<= 0`: a
  `TransferInput` with an unknown currency or transfer type, or a
  non-integer amount, doesn't deserialize.
- `src/domain/documents.ts` (`checkDocument`): "A stored receipt must be
  between 1 byte and 10 MB plus its encryption overhead." fires only for a
  size of 0 or over the limit: a negative, fractional or non-numeric
  `stored.size` doesn't deserialize, since it is a `u64`. Test:
  `d2_a_stored_size_that_isnt_a_whole_number_does_not_deserialize`.
- `src/domain/bitcoin.ts` (`fiatValueShape`, behind `writeDraft`,
  `validation`, the transfer commands and `assertBalanced`): "Enter the rate
  as the price of one bitcoin, such as 65000.00." never fires for a rate
  that isn't a string, "A fiat value must be zero or more, in whole minor
  units." only for a negative amount, and "Choose where the fiat value came
  from." is unreachable: a `FiatValue` with a numeric rate, a fractional
  amount or an unknown source doesn't deserialize. Neither do fractional
  `millisats`, so "Millisats must be a whole number greater than zero."
  fires only for zero or less. Test:
  `d2_a_fiat_value_or_millisats_that_cant_be_held_does_not_deserialize`.
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
before returning any state. The JSON shape is unchanged.

The TypeScript browser adapter checks stored state on load too
(`src/adapters/workspace-check.ts`, audit WEB-3): field types, enum values,
safe-integer amounts, each record's business, and every journal through
`assertBalanced`. It differs in what happens next: Rust refuses the whole
business, while the browser sets each failing record aside in the
workspace's `quarantine` list, reports it, and loads the rest, so one
damaged record can't lock the user out. Rust drops `quarantine` as an
unknown key (D7).

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

`SourceDocument` has no `blob` (`src/domain/types.ts`): that is the file
itself, in the browser that attached it, and file bytes belong to a storage
adapter. Unknown JSON keys on any record are dropped on read. Neither
affects domain behavior.

The document's `stored` copy (receipt storage,
[brief](docs/briefs/receipt-storage.md)) is record data, and both languages
carry it: the blob's address and size, its algorithm, and the file's key
and nonce. It is named `stored`, not `blob` as the brief has it, because
`blob` is already the local file. `checkDocument` (`src/domain/documents.ts`)
and `check_document` (`crates/ledger-core/src/documents.rs`) check its shape
with the same messages, and the protocol runs the check on every document
it attaches. `alg` is a string in Rust, not an enum, so that an unknown
algorithm reaches the check's message as in the TypeScript. Documents
without `stored` load and save unchanged.

Fixtures: `documents/checkDocument: *`, `documents/saveDraft: a draft
referring to a stored document`.

### D8. Unicode case-mapping tables

`toLowerCase`/`toUpperCase` (vendor and category matching, header mapping,
currency codes, export filenames) use V8's ICU tables. Rust's
`to_lowercase`/`to_uppercase` use the Unicode tables built into the Rust
standard library. Both apply full Unicode case mapping, including the Greek
final sigma, but the Unicode versions can differ, so a code point added in a
newer Unicode release could map differently. JavaScript whitespace (`trim`,
`\s`) and `\d` are reproduced exactly in `src/js.rs`.

### D9. An empty suggestion list is left out

A business has no `suggestions` list in TypeScript until its first
suggestion (`src/domain/suggestions.ts`), so Rust reads a missing list as
empty and writes an empty one as missing. TypeScript never writes `[]`
there, so this only matters for hand-made JSON, which reads the same either
way.

Fixtures: every state round-trips unchanged; `suggestions/*`.

## O: Owner funding design choices

Choices the [owner funding brief](docs/briefs/owner-funding.md) left open,
made the same way in both languages and pinned by the `transfers` and
`balances` fixtures. Tests: `src/domain/transfers.test.ts` and
`crates/ledger-core/tests/transfers.rs`.

- **O1. An opening balance is a transfer.** It is a journal of kind
  `transfer` with the type `openingBalance`, not a new journal kind, so
  every reader of journals already handles it and it never counts as
  spending. `recordTransfer` refuses that type ("Record an opening balance
  from its own step."). It is corrected with `correctTransfer`, whose
  replacement must then also be an opening balance, and an ordinary
  transfer only by an ordinary one ("Correct an opening balance with an
  opening balance, and a transfer with a transfer."). The replacement must
  be between the same two accounts, the account and its opening balances
  account ("An opening balance correction keeps its account."), and keeps
  the date rule (O4); its amount, direction and date may change. That
  keeps the brief's three commands. Fixtures: `transfers/correctTransfer:
an opening balance *`.
- **O2. A card's balance is what's owed, and an opening balance is
  signed.** `balances` reports a bank account's debits minus credits, and
  a card's, like a loan's or equity's, credits minus debits: a purchase on
  the card raises it and a payment to the card lowers it (decided for PR
  32, and the brief's balance rule updated). An opening balance's
  `amountMinor` is the balance in that convention: what's in a bank
  account, or what's owed on a card or loan account. An overdrawn bank
  account opens negative. Zero is refused. Fixtures:
  `transfers/recordOpeningBalance: a card, as the amount owed`, `… an
overdrawn bank account`, `balances/balances: a card purchase and a
payment`.
- **O3. One opening balances account per currency.** Every account has one
  currency and names are unique, so a single "Opening balances" account
  can't take a balance in another currency. The business's own currency
  uses "Opening balances"; any other uses "Opening balances (EUR)", say.
  An existing equity account of that name and currency is reused; any other
  account with the name, ignoring case, refuses the opening balance ("Another
  account is named Opening balances. Rename it to record opening
  balances."). Creating the account logs `settings.updated`, as adding an
  account does. Fixtures: `transfers/recordOpeningBalance: another currency
gets its own account`, `… another account has the name`.
- **O4. "The first other entry"** is the earliest date among the
  business's journals other than opening balances and their reversals. The
  check runs when an opening balance is recorded or corrected; a later
  entry dated before an existing opening balance isn't refused. The message
  names the date. Fixtures: `transfers/recordOpeningBalance: dated after
the first entry`, `… dated on the first entry`.
- **O5. Linked both ways without changing the original.** Posted journals
  never change, so the original transfer isn't edited. The reversal names
  it (`reversesId`), the replacement names it (`correctionOf`), and the
  `transfer.reversed` event lists all three ids. A reversal doesn't copy its
  original's `correctionOf`. "Already corrected" means some journal reverses
  it. Fixtures: `transfers/correctTransfer: *`.
- **O6. Type and purpose are optional on stored journals.** Transfers
  stored before this change have neither; they load unchanged and can be
  corrected like any transfer. Every new transfer has both, and so does the
  seed's (owner funding PR 2). Fixture: `transfers/correctTransfer: a transfer stored before
transfer types`.
- **O7. The memo** of a new transfer is "from → to" with the account names,
  as the seed's transfers already were. Events are `transfer.recorded`
  ("Owner loan recorded", "Opening balance recorded", …) and
  `transfer.reversed` (the reason).
- **O8. `addAccount`'s refusal keeps its message.** It now accepts
  `liability` and `equity`, and only `expense` reaches "Choose a bank or
  card account.", which `settings/addAccount: expense kind` pins.
- **O9. Imports still accept any non-expense account**, as before:
  `previewImport` and `commitImport` check `kind !== "expense"`, so a CSV of
  expenses paid personally can be imported to a loan account. Imports
  remain expense outflows only.
- **O10. Balances list every non-expense account in the currency**, in
  account order, including those with a zero balance.
- **O11. Exports (owner funding PR 2).** The expense export adds a last
  column, "Paid personally": "Loan" for an expense paid from a liability,
  "Contribution" from equity, empty otherwise; the existing "Payment
  account" column already names the account. The journal export adds
  "Transfer type" (the label, such as "Owner loan"), "Purpose",
  "Reference" and "Corrects journal ID" after its existing columns, so a
  correction's link is in the file too. `exportBalances(business, asOf,
currency)` is one row per `balances` row, with the account type and what
  the balance is ("Held", "Owed", "Owed to owner", "Owner's stake").
  Columns are only appended, so a reader that indexes the old ones still
  works. Fixtures: `export/*`, including the existing ones, whose headers
  and rows gained the columns.
- **O12. The demo seed** gains an "Owner's loan" account and, last month,
  a loan from the owner, an expense paid personally from it, and a
  repayment, so this month's spending is unchanged. Its existing transfer
  now carries the type `betweenAccounts` and a purpose.

## B: Bitcoin design choices

Choices [decision 0005](docs/decisions/0005-bitcoin-accounting.md) (phase 1)
left open, made the same way in both languages and pinned by the `bitcoin`
fixtures (`fixtures/parity/bitcoin.json`, recorded last so no earlier id
shifts; every existing fixture is unchanged, checked by
`scripts/check-fixture-renumbering.mjs`). Tests: `src/domain/bitcoin.test.ts`
and `crates/ledger-core/tests/bitcoin.rs`, which hold the decision's merge
gates in both languages.

- **B1. A precision table.** `precision` (`Currency::precision`) gives 2
  places for USD, EUR and GBP and 8 for BTC, whose minor unit is the sat.
  `parseMoney(input, currency)` and `amountInput(n, currency)` take it, and
  BTC amounts are typed in bitcoin ("0.00012345"). The two-place message is
  unchanged; BTC's is "Use an amount with up to eight decimal places, such
  as 0.00012345." `parseSats` reads whole sats ("Enter whole sats, such as
  2100."). The existing range limit, `MAX_MINOR`, holds 90,071,992.54740991
  BTC, more than the 21 million there will ever be. Fixtures recorded before
  this name no currency, and the Rust replay reads them as USD. Fixtures:
  `bitcoin/parseMoney *`, `bitcoin/parseSats *`, `bitcoin/amountInput *`.
- **B2. BTC is a ledger currency, not a reporting one.** `currencies` gains
  `BTC`; `fiatCurrencies` (`Currency::FIAT`) is the other three. A
  business's own currency, and a fiat value's, is always fiat: genesis
  refuses a BTC profile (`crates/protocol/src/apply.rs`), and the browser's
  workspace check quarantines one. Rust would load one; nothing writes it.
  The receipt extractor reads fiat only, as before, and the screens offer
  fiat only until the screens PR.
- **B3. A wallet is a bank account for bitcoin.** `wallet` must be in BTC
  ("A wallet holds bitcoin. Choose BTC."), and a bank or card account can't
  be ("Hold bitcoin in a wallet account."); a loan, equity or expense
  account may be in any currency, so the owner can lend bitcoin and BTC
  opening balances get their "Opening balances (BTC)" account. A wallet's
  balance is debits minus credits, its opening balance is a debit like a
  bank's but never below zero (B10), it can pay an expense, and it can be either side of the transfer
  types a bank can. Three messages predate wallets and stay as they were,
  so no existing fixture changes: "Choose a bank or card account.",
  "Opening balances are for bank, card and loan accounts." and the transfer
  types' "… bank or card account …". Fixtures: `bitcoin/addAccount: *`,
  `bitcoin/recordOpeningBalance: *`.
- **B4. The fiat value.** `fiatValue` is `{ currency, amountMinor, rate,
source, at }`. `rate` is the price of one bitcoin in `currency`, as a
  decimal string (at most 12 digits each side of the point, more than zero)
  so both languages read exactly what was written; a JSON number could
  round differently. "Within one minor unit" means
  |amount − sats × rate| ≤ 1 in the currency's minor unit, worked out in
  integers (`BigInt`, `u128`). `at` is a day or a UTC time
  (`YYYY-MM-DDTHH:MM[:SS[.sss]]Z`). It is required on every BTC journal
  (expenses, transfers, opening balances, and reversals, which copy it) and
  refused on any other. A draft may be saved without one, with only its
  shape checked, and `validation` asks for it before approval, as for any
  missing detail. `assertBalanced` is the backstop, with the generic
  "Invalid journal fiat value." Fixtures: `bitcoin/recordTransfer: a fiat
value *`, `bitcoin/validation: *`, `bitcoin/assertBalanced: *`.
- **B5. An exchange is one journal in two currencies.** Its `currency` is
  BTC; it has exactly two lines, a BTC wallet's in sats and a fiat bank or
  card account's for the fiat value's `amountMinor`, in that account's
  currency, in opposite directions. No clearing account: each side balances
  in its own currency, through the recorded rate. Its fiat value's source
  must be `exchange`, and its amount more than zero. The caller sends the
  rate; `impliedRate` (a screen helper, to 12 places) always passes the
  check. It is corrected with `correctTransfer`, only by another exchange
  between the same two accounts (B9). `balances` counts it
  in both currencies; the journal export lists it under either currency's
  filter, each line in its own currency. Fixtures: `bitcoin/recordTransfer:
an exchange *`, `bitcoin/correctTransfer: *`, `bitcoin/balances: *`,
  `bitcoin/exportRecords: *`.
- **B6. Millisats are kept beside the sats.** A draft, a transfer and its
  journal may carry `millisats`, BTC only and more than zero. The caller
  sends both amounts, and `amountMinor` must be the millisats rounded to the
  nearest sat, half a sat rounding up (`satsFromMillisats`). Fixtures:
  `bitcoin/satsFromMillisats *`, `bitcoin/recordTransfer: millisats *`.
- **B7. Sats stay sats.** `totals` and `categoryTotals` report BTC in sats
  under their own key. `recordedFiatSpending` is the "dollar value at time
  of transaction": per fiat currency, the recorded fiat values of the
  period's journals with spend lines (a reversal subtracting), so transfers
  and exchanges never count. Fixtures: `bitcoin/totals: *`,
  `bitcoin/recordedFiatSpending: *`, `bitcoin/categoryTotals: *`.
- **B8. Imports read at the account's precision.** A CSV imported to a
  wallet reads BTC amounts to 8 places, with the same comma rule. Its drafts
  have no fiat value, so each needs one before it's approved. Fixture:
  `bitcoin/previewImport: bitcoin payments into a wallet`.
- **B9. An exchange is corrected only by another exchange, between the same
  wallet and the same fiat account,** and nothing else is corrected into
  one, as an opening balance is only by another on its account (O1). The
  amounts, the direction, the date and the rate may change. The refusals,
  checked in this order after O1's: "Only an exchange can be corrected by
  an exchange.", "Correct an exchange with another exchange.", "An exchange
  correction keeps its wallet." and "An exchange correction keeps its
  dollar, euro or pound account." Fixtures: `bitcoin/correctTransfer: an
exchange, by *` and `bitcoin/correctTransfer: a transfer between wallets,
by an exchange`.
- **B10. A wallet never opens below zero.** A wallet can't hold less than
  no bitcoin, so `recordOpeningBalance` refuses a negative amount on one,
  and `correctTransfer` refuses an opening balance replacement that credits
  a wallet: "A wallet can't open below zero. Enter its opening balance as
  sats greater than zero." The check is in the shared opening-balance
  check, so both commands give the same message. Ordinary expenses,
  transfers and exchanges may still take a wallet below zero in this
  phase. **A later balance check** should refuse any journal that would
  leave a wallet holding less than zero sats on its date; this phase
  refuses only the opening balance, as decided. Fixtures:
  `bitcoin/recordOpeningBalance: a negative wallet balance`,
  `bitcoin/correctTransfer: a wallet's opening balance, to below zero`.

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

- Most of `src/domain/extraction.ts` (applying receipt suggestions to the
  form). It is Review-screen logic, not a ledger rule. Rust has the
  `ExtractionRecord` types (serde, `extractions` defaults to `[]`), and
  `outcomesFor` and `finalizeExtractions` (`crates/ledger-core/src/extraction.rs`),
  which `approve` runs, and so `correct` for its replacement: the service
  judges a draft's readings when it posts it, as the browser does.
  Fixtures: `approve/readings: *`, `correct/readings: *`.
- `money()` display formatting (uses `Intl`; a UI concern), including
  bitcoin in sats or BTC, and `impliedRate`, which fills in an exchange's
  rate on the screen.
- The screen helpers at the end of `src/domain/transfers.ts`
  (`paymentAccounts`, `transferAccounts`, `openingBalance`,
  `openingBalanceReplacement`, `firstEntryDate`, and the label maps' use on
  screens). They choose what a screen offers; the commands they feed are
  ported and decide what's allowed.
- `seed.ts`. Rust tests load the seed recorded by the fixture generator
  (`fixtures/parity/states/seed.json`).
- `src/adapters/`, and the adapter-facing interfaces in `types.ts`
  (`AIProposal`, `AIAdapter`, `BusinessRepository`, `ReceiptStorage`,
  `LedgerCommands`, `LedgerQueries`, `EventTransport`).
- The ported test
  `business_separation_rejects_foreign_drafts_accounts_evidence_and_journal_context`
  keeps the domain assertions only; its `demoAI` and `MockTransport`
  assertions test adapters.
