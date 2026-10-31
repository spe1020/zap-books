# Brief: Owner funding, transfers and balances

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Save this brief as:** `docs/briefs/owner-funding.md` (Prettier only)
**Read first:** `docs/architecture.md`, `docs/protocol.md`, `PARITY_NOTES.md`, decisions `0001`–`0004`.

**Privacy rule:** invented values only. The guard must pass on every commit and PR description.

## Why

Zap Books records expenses paid from a business bank account or card. A small business's books also have to show money flowing between the owner and the business:

- **Owner loans:** the owner lends the business money, and the business owes it back.
- **Owner contributions:** the owner invests money in the business as equity, which isn't repaid.
- **Expenses the owner pays personally** on the business's behalf. These increase what the business owes the owner, or count as a contribution.
- **Repayments** of an owner loan, and **transfers between the business's own accounts.**
- **Opening balances,** for a business that already existed before it started using Zap Books.

Without these, expenses look right, but the books can't say what the business owes its owner. Whether owner money is a loan or a contribution is for the business's accountant to decide. **Zap Books supports both and never chooses for the user.**

## Accounting model

### Account kinds

Add two kinds to the existing `bank`, `card` and `expense`:

- **`liability`:** money the business owes, for example "Loan from owner".
- **`equity`:** the owner's stake, for example "Owner contributions" and "Opening balances".

Rules:

- Expense categories still map only to `expense` accounts. A line with a category must sit on an expense account, and a line without one must not (ACC-2).
- **Payment accounts for an expense** can be `bank`, `card`, `liability` or `equity`. Paying from a liability means "the business now owes this"; paying from equity means "the owner contributed this".
- **Every account has one currency,** as today. Nothing ever converts between currencies.
- **Account names are unique** across all accounts and categories (ACC-3 continues to apply).

### Transfers

A new command, `recordTransfer`, records money moving between two non-expense accounts:

- fields: date, amount (positive minor units), currency, from-account, to-account, purpose, an optional reference, optional document ids, and a **transfer type**: `ownerLoan`, `ownerContribution`, `loanRepayment`, `ownerDraw`, `betweenAccounts`;
- the journal debits the to-account and credits the from-account;
- the type must fit the accounts:
  - `ownerLoan`: liability to bank or card;
  - `ownerContribution`: equity to bank or card;
  - `loanRepayment`: bank or card to liability;
  - `ownerDraw`: bank or card to equity;
  - `betweenAccounts`: bank or card to bank or card.

  Anything else is refused with a plain message, such as "An owner loan moves money from a loan account into a bank or card account.";

- both accounts are in the business and in the transfer's currency, and they differ;
- transfers post on confirmation (there are no transfer drafts), and they **never count as spending,** as today;
- the existing journal kind `transfer` carries them. Store the transfer type and purpose on the journal so exports and reports can show them.

### Correcting a transfer

A new command, `correctTransfer`, takes the original transfer's id, a full replacement and a reason. It posts an exact reversal of the original and the replacement, linked both ways, all-or-nothing. This is the same rule as expense corrections. A reversed transfer can't be corrected again.

### Opening balances

A new command, `recordOpeningBalance`, sets an account's balance as of the business's start date:

- for bank, card and liability accounts. A card's opening balance is entered as the amount owed on it;
- it posts against an `equity` account named "Opening balances", created automatically the first time it's needed;
- at most one opening balance per account, and it can be corrected the same way as a transfer. The correction keeps the type `openingBalance`, the same account, and the date rule below;
- it's dated on or before the business's first other entry. A later date is refused.

### Balances

A new query, `balances(business, asOf, currency)`, returns every non-expense account's balance as of a date, from posted journals only:

- bank balances: debits minus credits;
- card, liability and equity balances: credits minus debits. A card's balance is what's owed on it: purchases raise it, and payments lower it;
- reversals offset their originals, as everywhere else.

It returns one row per account with its kind. Currencies are never combined.

## Protocol

- Extend `UpdateSettings` so `addAccount` accepts `liability` and `equity`.
- Add commands `RecordTransfer`, `CorrectTransfer` and `RecordOpeningBalance`, applied through `apply_command` with `ledger-core`, with record changes that rebuild and audit like every other command.
- **Permissions:** owner and bookkeeper only. The assistant, viewer, submitter and recovery key are refused. Add a test for each refusal.
- Update `docs/protocol.md`, the WASM bindings, and the permission table.

## Parity

This is a `ledger-core` behavior change, so the usual rule applies: TypeScript first, regenerate fixtures, then Rust. New fixtures cover:

- every transfer type with valid accounts, and each wrong-account refusal;
- transfer corrections, including a second correction being refused;
- opening balances, including a late date being refused and a second opening balance for the same account being refused;
- expenses paid from a liability and from an equity account;
- balances as of several dates, with reversals.

Existing fixtures may change only by id renumbering, verified by a script. Add a `PARITY_NOTES.md` entry for each design choice.

## Screens

- **Settings → Accounts:** add loan (liability) and owner equity accounts. Include a one-line explanation of each kind and a note: "Ask your accountant whether owner money is a loan or a contribution."
- **New expense → Paid from:** list business accounts first, then "Paid personally: [loan account]" and "Paid personally: [equity account]", when they exist.
- **Transactions → Record transfer:** one form with the transfer type first. The type preselects the matching accounts and only offers valid ones. A clear summary before posting says, for example, "The business will owe [owner loan] $X more."
- **Opening balances:** a guided step in Settings, for accounts that existed before the business's first entry.
- **Overview:** a balances panel listing each bank, card, loan and equity account's balance for the selected currency, as of today. It includes "Owed to owner" for loan accounts.
- **Exports:**
  - the journal export includes transfer type and purpose;
  - the expense export shows which payment account each expense used, including personally paid ones;
  - a new **balances export** (CSV) as of a chosen date.
- **Demo seed:** add a fictional owner loan, one personally paid expense, and one repayment, so the demo shows the feature. The parity fixtures will change for the seed; that's expected.

## Required tests

Merge gates, in TypeScript and Rust:

- A personally paid expense leaves bank balances unchanged and raises the loan balance by its amount.
- A loan deposit raises both the bank balance and the loan balance; a repayment lowers both.
- Transfers never appear in spending totals or category totals.
- Every wrong-account combination for each transfer type is refused with its message.
- Correcting a transfer reverses it exactly and links the replacement; the reversal and original net to zero on both accounts.
- A transfer between currencies, or between an account and itself, is refused.
- An opening balance dated after the business's first entry is refused.
- Balances are correct at dates before, between and after entries, including reversals.
- Property test: across any sequence of valid commands, total debits equal total credits, and the balances of all non-expense accounts plus spending reconcile with the journal.
- **Through the service:** a bookkeeper's transfer is accepted, an assistant's is refused, and the transfer rebuilds and audits from the chain.
- E2e: record an owner loan, a personally paid expense and a repayment, and check the balances panel and the balances export.

## PR sequence

Do these as separate PRs. **Stop after each one is open.**

1. **Core:** TypeScript domain, `ledger-core`, protocol, WASM bindings, fixtures and their tests. No UI changes beyond what existing screens need to keep working.
2. **Screens:** Settings, the expense payment choice, Record transfer, opening balances, the balances panel, exports, the demo seed, and the e2e tests.

## Allowed dependencies

None new. Stop and ask if one seems needed.

## Stop and ask if

- a design choice here conflicts with how the existing `transfer` journals in the seed are built;
- an existing fixture changes other than by id renumbering, outside the seed;
- the account-kind change would alter how any existing stored record loads;
- an accounting rule here seems wrong. Say why, rather than working around it.

## Out of scope

- Interest on owner loans, loan schedules and promissory notes.
- Income, invoices and sales.
- Bitcoin and Lightning (a separate decision, 0005).
- Bank CSV imports of incoming money. Imports remain expense outflows only.
- Reconciliation and closed periods.

## Each PR description must include

1. Results of every check, including parity and the merge-gate tests.
2. Screenshots of every new screen (PR 2), using invented data.
3. Any deviation from this brief, and why.
4. Confirmation that the privacy guard passed.
